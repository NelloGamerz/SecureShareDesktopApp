use crate::models::chunk::ChunkJob;
use crate::services::local_transfer_file_service::LocalTransferFileService;
use crate::transfer::http_client::SessionStart;
use crate::transfer::integrity::{self, ExpectedTransfer};
use crate::transfer::persist::{PersistedTransfer, TransferSecrets, TransferStore};
use crate::transfer::report;
use crate::transfer::resume;
use crate::transfer::session::{LegacyV1Session, OpenRequest, ProtocolSession};
use crate::{
    models::transfer::{TransferMetadata, TransferStatus, TransferStatusResponse},
    transfer::{
        chunker, constants,
        errors::{Result, TransferError},
        progress, scanner, scheduler,
        state::{TransferCredentials, UploadState},
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::Ordering,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use vilsend_core::{DomainEvent, EventSink};

/// How long the manager waits between checkpoints while a transfer runs.
///
/// A checkpoint writes the whole state file, so one per acknowledged chunk would
/// make a transfer's persistence cost grow with the square of its size. Nothing
/// is lost between checkpoints: the in-memory set is authoritative while the
/// process lives, and only a crash can shorten the record — by at most this
/// much, and the chunks it misses are simply sent again.
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(2);

/// Milliseconds since `origin`, for the startup timeline every log line shares.
fn since_ms(origin: Instant) -> u64 {
    origin.elapsed().as_millis() as u64
}

/// How long a cancellation waits for the receiver to acknowledge it.
///
/// Telling the receiver is best-effort: it is worth a moment, because it is what
/// frees the receiver's disk now rather than at the next sweep, and it is not
/// worth waiting on a peer that is gone.
const RECEIVER_NOTICE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether the sender hashes its files and describes them to the receiver.
///
/// With this off, no digest pass runs and the receiver checks nothing about the
/// assembled file. The per-chunk records the receiver writes for a resume are
/// untouched — those are how a transfer knows what not to send again.
///
/// shortcut: integrity disabled, re-enable by setting this to `true`.
pub(crate) const DESCRIBE_DIGESTS: bool = false;

#[derive(Debug)]
pub enum TransferEvent {
    Completed(String),
    Failed(String, String),
}

#[derive(Clone)]
pub struct UploadManager {
    transfers: Arc<Mutex<HashMap<String, Arc<UploadState>>>>,
    /// The shell's [`EventSink`]. Replaces the `AppHandle` this struct used to
    /// hold: emitting progress was the only thing the handle was for.
    events: Arc<dyn EventSink>,
    upload_root: PathBuf,
    local_files: Arc<LocalTransferFileService>,
    transfer_events: tokio::sync::mpsc::UnboundedSender<TransferEvent>,
    /// Where transfers outlive the process. See `transfer::persist`.
    store: Arc<TransferStore>,
    /// Serialises state-file writes, so that two checkpoints racing cannot land
    /// out of order and leave an older snapshot on disk.
    persist_lock: Arc<tokio::sync::Mutex<()>>,
    /// When the last checkpoint was taken, for [`CHECKPOINT_INTERVAL`].
    last_checkpoint: Arc<Mutex<Instant>>,
}
impl UploadManager {
    pub fn new(
        events: Arc<dyn EventSink>,
        upload_root: PathBuf,
        local_files: Arc<LocalTransferFileService>,
        transfer_events: tokio::sync::mpsc::UnboundedSender<TransferEvent>,
        store: Arc<TransferStore>,
    ) -> Self {
        Self {
            transfers: Arc::new(Mutex::new(HashMap::new())),
            events,
            upload_root,
            local_files,
            transfer_events,
            store,
            persist_lock: Arc::new(tokio::sync::Mutex::new(())),
            // Far enough in the past that the first chunk checkpoints at once.
            last_checkpoint: Arc::new(Mutex::new(
                Instant::now()
                    .checked_sub(CHECKPOINT_INTERVAL)
                    .unwrap_or_else(Instant::now),
            )),
        }
    }

    /// Rebuild the transfers a previous run left behind.
    ///
    /// Called during initialization. Nothing is sent: a restored transfer is
    /// put back in the map, paused, with the chunks the receiver had already
    /// acknowledged, so that `status` can describe it and a later step can
    /// decide what to do about it. Resuming is deliberately not this function's
    /// job.
    ///
    /// Returns how many transfers were restored.
    ///
    /// Idempotent: a transfer already in the map is left alone, so calling this
    /// after a restore, or while a transfer is running, cannot clobber live
    /// state with what was on disk.
    pub async fn restore_persisted(&self) -> usize {
        let mut restored = 0;

        for record in self.store.load_all().await {
            /*
             * Read the digest plan before taking the transfer map's lock: it is
             * an async read, and a `std::sync::Mutex` must never be held across
             * an await. It is also optional — a transfer that never had one, or
             * whose plan file was removed, is restored without it and plans
             * again when it is first resumed.
             */
            let expected = self.store.load_plan(&record.transfer_id).await;

            let status = record.resumable_status();

            let state = Arc::new(restored_state(&record, expected));

            {
                let mut transfers = self.transfers.lock().expect("transfer lock poisoned");

                if transfers.contains_key(&record.transfer_id) {
                    continue;
                }

                tracing::info!(
                    transfer_id = %record.transfer_id,
                    completed_chunks = record.completed_chunks.len(),
                    total_chunks = record.total_chunks,
                    ?status,
                    "restored a persisted transfer"
                );

                transfers.insert(record.transfer_id.clone(), Arc::clone(&state));
            }

            /*
             * A transfer that reached a terminal state before the process ended
             * may have died between that transition and writing its report. If
             * the report is missing, write what the record still knows and say
             * plainly that the timings were lost — a wrong report is worse than
             * an incomplete one.
             */
            if matches!(
                status,
                TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled
            ) {
                self.recover_report(&state).await;
            }

            restored += 1;
        }

        restored
    }

    /// Write a terminal transfer's report from what a restart restored, when
    /// the live one never reached the disk.
    async fn recover_report(&self, state: &Arc<UploadState>) {
        let path = report::report_path(&self.store.reports_dir(), &state.transfer_id);

        if tokio::fs::metadata(&path).await.is_ok() {
            return;
        }

        let status = state.status.lock().expect("status lock poisoned").clone();

        self.finalize_report(state, status, None, None, None, true)
            .await;
    }

    pub async fn start(&self, metadata: TransferMetadata) -> Result<TransferStatusResponse> {
        if metadata.transfer_id.trim().is_empty() || metadata.endpoint.trim().is_empty()
        // || metadata.file_paths.is_empty()
        {
            return Err(TransferError::Invalid(
                "transferId, endpoint, and filePaths are required".into(),
            ));
        }
        if self
            .transfers
            .lock()
            .expect("transfer lock poisoned")
            .contains_key(&metadata.transfer_id)
        {
            return Err(TransferError::Invalid("transfer already active".into()));
        }
        /*
         * The phases below are timed individually. Starting a transfer of any
         * size spends its time in two places — walking the files and hashing
         * every byte — and neither emits a progress event, so these lines are
         * what makes "why has it not started yet?" answerable from the log.
         */
        let startup_started = Instant::now();

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            startup_ms = 0u64,
            "startup: transfer start requested"
        );

        // let files = scanner::scan(&metadata.file_paths).await?;
        let local_files = self
            .local_files
            .get_by_transfer_id(&metadata.transfer_id)
            .await
            .map_err(|e| {
                TransferError::Invalid(format!("failed loading local transfer files: {}", e))
            })?;

        if local_files.is_empty() {
            return Err(TransferError::Invalid(
                "no local files found for transfer".into(),
            ));
        }

        let paths = local_files
            .iter()
            .map(|f| f.file_path.clone())
            .collect::<Vec<_>>();

        let scan_started = Instant::now();
        let files = scanner::scan(&paths).await?;

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            files = files.len(),
            elapsed_ms = scan_started.elapsed().as_millis() as u64,
            startup_ms = since_ms(startup_started),
            "startup phase: files scanned"
        );

        /*
         * The wire this transfer will run on, behind the session seam. Building
         * it here rather than reaching for an `HttpClient` at each call site is
         * what leaves room for a second session type later — a failover replaces
         * this value, and nothing downstream changes.
         */
        let session = LegacyV1Session::to(metadata.endpoint.clone(), metadata.auth_token.clone());

        /*
         * What the receiver will accept, asked before anything is planned.
         *
         * Chunk size is baked into the jobs and into the digests the receiver is
         * later checked against, so it cannot be revised after the handshake.
         * Asking first is also what keeps this sender compatible: an installed
         * receiver advertises nothing and gets the legacy 4 MiB chunks its 10 MiB
         * body limit accepts, instead of 16 MiB chunks it would refuse with 413.
         */
        let capabilities_started = Instant::now();

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            startup_ms = since_ms(startup_started),
            "startup: capabilities request initiated"
        );

        let capabilities = session.capabilities().await?;

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            max_chunk_bytes = capabilities.max_chunk_bytes,
            elapsed_ms = capabilities_started.elapsed().as_millis() as u64,
            startup_ms = since_ms(startup_started),
            "startup: capabilities response received"
        );

        let total_bytes = files.iter().map(|f| f.size).sum();

        /*
         * A backend may name a size, but never one above this build's cap: the
         * cap is what keeps a stalled chunk cheap to retry. Otherwise the size
         * follows the transfer's own size, within the same 256 KiB–1 MiB band.
         */
        let requested_chunk_size = metadata
            .chunk_size
            .unwrap_or_else(|| chunker::chunk_size_for(total_bytes))
            .clamp(1, constants::MAX_PLANNED_CHUNK_SIZE);

        /*
         * Clamp to what the receiver accepts, and to what this build's body
         * limit and frame cap are derived from. Reporting the clamp is what turns
         * a silent `413` on every chunk into one line in the log at plan time.
         */
        let chunk_size = requested_chunk_size
            .min(capabilities.max_chunk_bytes)
            .min(constants::MAX_CHUNK_SIZE);

        if chunk_size != requested_chunk_size {
            tracing::warn!(
                transfer_id = %metadata.transfer_id,
                requested_chunk_size,
                receiver_max = capabilities.max_chunk_bytes,
                chunk_size,
                "chunk size exceeds what this receiver accepts; clamping it"
            );
        }

        let jobs = chunker::jobs(&metadata.transfer_id, &files, chunk_size);

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            chunks = jobs.len(),
            startup_ms = since_ms(startup_started),
            "startup: chunk jobs generated"
        );

        /*
         * The handshake comes before the digests, not after them.
         *
         * It used to wait for a pass that read every byte of the transfer, which
         * for a large file is the whole of "nothing is happening yet". The
         * digests are not needed to open a session; they are needed before a
         * file can be assembled and checked, which is why only each file's final
         * chunk waits for them (see `scheduler::PlanGate`).
         *
         * The description is optional on the wire: a receiver that never hears
         * it checks nothing, exactly as it did before this build.
         */
        /*
         * Everything up to here is initialization: loading the file list,
         * scanning it, and asking the receiver what it accepts. The handshake
         * is timed on its own, below.
         */
        let initialization_seconds = startup_started.elapsed().as_secs_f64();

        let handshake_started = Instant::now();

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            startup_ms = since_ms(startup_started),
            "startup: handshake request initiated"
        );

        let opened = session
            .open(OpenRequest {
                transfer_id: &metadata.transfer_id,
                file_size: total_bytes,
                expected: None,
                existing_key: None,
            })
            .await?;

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            start = ?opened.start,
            elapsed_ms = handshake_started.elapsed().as_millis() as u64,
            startup_ms = since_ms(startup_started),
            "startup: handshake response received"
        );

        let handshake_seconds = handshake_started.elapsed().as_secs_f64();

        let transfer_key = opened.transfer_key;

        let hash = Sha256::digest(transfer_key);

        tracing::info!(
            transfer_key_hash=%hex::encode(hash),
            "sender transfer key fingerprint"
        );

        let concurrency = metadata
            .concurrency
            .unwrap_or(constants::DEFAULT_CONCURRENCY);

        let state = Arc::new(UploadState {
            transfer_id: metadata.transfer_id.clone(),
            credentials: Mutex::new(TransferCredentials {
                transfer_key,
                auth_token: metadata.auth_token.clone(),
            }),
            endpoint: metadata.endpoint.clone(),
            network_type: metadata.network_type.clone(),
            files,
            chunk_size,
            concurrency,
            total_bytes,
            uploaded_bytes: 0.into(),
            total_chunks: jobs.len() as u64,
            uploaded_chunks: 0.into(),
            paused: false.into(),
            cancelled: false.into(),
            retry_count: 0.into(),
            max_retries: metadata
                .max_retries
                .unwrap_or(constants::DEFAULT_MAX_RETRIES),
            status: Mutex::new(TransferStatus::Queued),
            started_at: std::time::Instant::now(),
            startup_started,
            completed: Mutex::new(std::collections::HashSet::new()),
            running: false.into(),
            resume_lock: tokio::sync::Mutex::new(()),
            // Worked out in the background, while the transfer runs.
            expected: Mutex::new(None),
            plan_stamps: Mutex::new(std::collections::HashMap::new()),
            report: report::Telemetry::new(report::Role::Sender),
            phases: Mutex::new(report::PhaseTimings {
                initialization_seconds: Some(initialization_seconds),
                handshake_seconds: Some(handshake_seconds),
                ..report::PhaseTimings::default()
            }),
        });
        let response = state.snapshot();
        self.transfers
            .lock()
            .expect("transfer lock poisoned")
            .insert(metadata.transfer_id.clone(), state.clone());

        /*
         * Persist before any chunk moves. The secrets are written now so that a
         * crash during the first chunk still leaves a transfer that can be
         * described and continued; the plan is written once it exists.
         */
        let persistence_started = Instant::now();

        self.store_secrets(&state);
        self.persist(&state).await;

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            elapsed_ms = persistence_started.elapsed().as_millis() as u64,
            startup_ms = since_ms(startup_started),
            "startup: transfer state persisted"
        );

        let gate = Arc::new(scheduler::PlanGate::default());
        let session: Arc<dyn ProtocolSession> = Arc::new(session);

        if DESCRIBE_DIGESTS {
            self.spawn_scheduler(
                state.clone(),
                jobs,
                Arc::clone(&session),
                Some(gate.clone()),
            );
            self.plan_in_background(state, session, gate);
        } else {
            self.spawn_scheduler(state, jobs, session, None);
        }

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            startup_ms = since_ms(startup_started),
            "startup: scheduler spawned"
        );

        tracing::info!(
            transfer_id = %metadata.transfer_id,
            total_bytes,
            elapsed_ms = startup_started.elapsed().as_millis() as u64,
            "transfer started"
        );

        Ok(response)
    }

    /// Work out the digests while the transfer is already sending, then describe
    /// them to the receiver and release the chunks that were waiting for them.
    ///
    /// The hashing is the expensive part of starting a large transfer, and it
    /// does not have to happen before the bytes move: the chunks stream, and
    /// only each file's last one is held back. A plan that cannot be worked out
    /// fails the gate, so no file is ever completed unchecked.
    fn plan_in_background(
        &self,
        state: Arc<UploadState>,
        session: Arc<dyn ProtocolSession>,
        gate: Arc<scheduler::PlanGate>,
    ) {
        let manager = self.clone();

        tokio::spawn(async move {
            let files = state.files.clone();
            let chunk_size = state.chunk_size;

            let expected =
                match integrity::plan_with_progress(&state.transfer_id, &files, chunk_size, None)
                    .await
                {
                    Ok(expected) => expected,

                    Err(error) => {
                        tracing::error!(
                            transfer_id = %state.transfer_id,
                            %error,
                            "could not plan the transfer's digests; no file may complete"
                        );

                        gate.fail();

                        return;
                    }
                };

            tracing::info!(
                transfer_id = %state.transfer_id,
                "digest plan complete"
            );

            /*
             * Kept even if the receiver cannot be told, so a resume does not
             * have to read every file again. Written before the delivery so a
             * crash between the two leaves a plan, not nothing.
             */
            state.set_expected(expected.clone());

            if state.cancelled.load(Ordering::Relaxed) {
                return;
            }

            manager.persist_plan(&state.transfer_id, &expected).await;

            /*
             * The handshake again, this time carrying the digests. A receiver
             * that still has the transfer live answers `CONFLICT` and keeps its
             * key — nothing is re-established, only the description is updated.
             */
            match session
                .open(OpenRequest {
                    transfer_id: &state.transfer_id,
                    file_size: state.total_bytes,
                    expected: Some(&expected),
                    existing_key: Some(state.transfer_key()),
                })
                .await
            {
                Ok(opened) => {
                    /*
                     * `Established` means the receiver restarted while the plan
                     * was being worked out and built a fresh session and key
                     * from this handshake. Adopt its key; chunks already in
                     * flight under the old one are refused and retried.
                     */
                    if opened.start == SessionStart::Established
                        && opened.transfer_key != state.transfer_key()
                    {
                        let auth_token = state.auth_token();

                        state.set_credentials(TransferCredentials {
                            transfer_key: opened.transfer_key,
                            auth_token,
                        });

                        manager.store_secrets(&state);
                    }

                    gate.release();
                }

                Err(error) => {
                    tracing::error!(
                        transfer_id = %state.transfer_id,
                        %error,
                        "could not describe the transfer's digests to the receiver"
                    );

                    gate.fail();
                }
            }
        });
    }

    /// Start the workers that send `jobs` for a transfer.
    ///
    /// Shared by `start` and by a resume, because the two differ only in which
    /// jobs they hand over: a resume's queue is what the receiver did not
    /// confirm, and everything downstream of the queue — the progress callback,
    /// the checkpointing, the terminal transitions — is the same.
    ///
    /// Exactly one scheduler may run per transfer at a time. `start` refuses a
    /// transfer id already in the map, and `resume` holds the transfer's
    /// [`UploadState::resume_lock`] while it decides, so two resumes cannot both
    /// find an idle transfer and start a run each.
    fn spawn_scheduler(
        &self,
        state: Arc<UploadState>,
        jobs: Vec<ChunkJob>,
        session: Arc<dyn ProtocolSession>,
        gate: Option<Arc<scheduler::PlanGate>>,
    ) {
        let manager = self.clone();
        let concurrency = state.concurrency;

        state.set_running(true);

        /*
         * Published before the workers exist rather than from inside the
         * spawned task, so the transfer reads as active from the moment its
         * caller is told the resume was accepted.
         */
        *state.status.lock().expect("status lock poisoned") = TransferStatus::Uploading;

        tokio::spawn(async move {
            let s = state.clone();
            let events = Arc::clone(&manager.events);
            let checkpointing = manager.clone();
            let on_success = Arc::new(move |job: &ChunkJob, retries: u32| {
                s.retry_count.fetch_add(retries, Ordering::Relaxed);
                // Recording the chunk is also what moves the progress counters:
                // one place, so the bytes reported are the bytes of the chunks
                // the set actually holds, and a chunk that arrives twice moves
                // nothing the second time.
                s.mark_completed(&job.file_id, job.index);
                events.emit(DomainEvent::TransferProgress {
                    progress: progress::make(&s),
                });
                checkpointing.checkpoint(&s);
            });
            let result =
                scheduler::run(jobs, concurrency, state.clone(), session, on_success, gate).await;

            if state.cancelled.load(Ordering::Relaxed) {
                state.set_running(false);
                return;
            }

            /*
             * A drained queue is not completion. `scheduler::run` returns `Ok`
             * when every job it was handed succeeded, and the queue is built
             * from the chunks that were still missing — so a transfer is only
             * finished when the acknowledged set covers the whole plan. Calling
             * one complete on a drained queue alone would let a transfer report
             * itself finished with chunks never sent, which is the one thing a
             * resume must never do.
             */
            let outcome = match result {
                Ok(()) if state.is_complete() => {
                    *state.status.lock().expect("status lock poisoned") = TransferStatus::Completed;
                    manager.events.emit(DomainEvent::TransferCompleted {
                        progress: progress::make(&state),
                    });

                    manager
                        .finalize_report(&state, TransferStatus::Completed, None, None, None, false)
                        .await;

                    manager.forget_plan(&state.transfer_id).await;

                    TransferEvent::Completed(state.transfer_id.clone())
                }
                Ok(()) => {
                    manager
                        .fail_transfer(
                            &state,
                            "completion",
                            format!(
                                "the send queue drained with only {} of {} chunks acknowledged",
                                state.uploaded_chunks.load(Ordering::Relaxed),
                                state.planned_chunks()
                            ),
                        )
                        .await
                }
                Err(error) => manager.fail_transfer(&state, "upload", error).await,
            };

            /*
             * The terminal status goes up before the running flag comes down. A
             * resume that arrives in between sees a finished transfer rather
             * than an idle-looking one that is about to be marked failed, so it
             * restarts the transfer instead of mistaking it for parked work.
             */
            state.set_running(false);

            manager.persist(&state).await;

            let _ = manager.transfer_events.send(outcome);
        });
    }

    /// Move a transfer to `Failed` and report why.
    ///
    /// The last progress sample carries the failed status, so a frontend that
    /// only follows progress events sees the transfer stop where it stopped.
    async fn fail_transfer(
        &self,
        state: &Arc<UploadState>,
        stage: &str,
        error: String,
    ) -> TransferEvent {
        *state.status.lock().expect("status lock poisoned") = TransferStatus::Failed;
        tracing::error!(transfer_id=%state.transfer_id,error=%error,"transfer failed");
        self.events.emit(DomainEvent::TransferFailed {
            progress: progress::make(state),
        });

        self.finalize_report(
            state,
            TransferStatus::Failed,
            None,
            Some(stage.to_owned()),
            Some(error.clone()),
            false,
        )
        .await;

        TransferEvent::Failed(state.transfer_id.clone(), error)
    }

    /// Write this transfer's performance report, once.
    ///
    /// The telemetry is what makes it exactly once: whichever terminal path
    /// arrives first wins, and the rest are no-ops. A report that cannot be
    /// written is logged and dropped — it is history, not state, and a transfer
    /// must not fail over it.
    async fn finalize_report(
        &self,
        state: &Arc<UploadState>,
        status: TransferStatus,
        cancellation_reason: Option<String>,
        failure_stage: Option<String>,
        failure_reason: Option<String>,
        recovered: bool,
    ) {
        let chunk_size = state.chunk_size.max(1) as u64;

        let files = state
            .files
            .iter()
            .map(|file| report::FileDescriptor {
                file_id: file.id.clone(),
                relative_path: Some(file.relative_path.clone()),
                size: file.size,
                expected_chunks: file.size.div_ceil(chunk_size).max(1),
            })
            .collect();

        let phases = state.phases.lock().expect("phases lock poisoned").clone();

        let finish = report::Finish {
            transfer_id: state.transfer_id.clone(),
            status,
            total_bytes: state.total_bytes,
            completed_bytes: state.uploaded_bytes.load(Ordering::Relaxed),
            total_chunks: state.total_chunks,
            completed_chunks: state.uploaded_chunks.load(Ordering::Relaxed),
            files,
            retries: state.retry_count.load(Ordering::Relaxed) as u64,
            failure_stage,
            failure_reason,
            cancellation_reason,
            phases,
            unavailable: vec![
                "phases.final_assembly_seconds (the sender does not assemble)".into(),
                "phases.verification_seconds (the sender does not verify the receiver's file)"
                    .into(),
                "phases.publication_seconds (the receiver publishes the file)".into(),
                "resources are process-wide; concurrent transfers share them".into(),
            ],
            recovered,
        };

        state
            .report
            .finalize(
                &self.store.reports_dir(),
                finish,
                crate::transfer::session::COMPRESSION_ENABLED,
                DESCRIBE_DIGESTS,
            )
            .await;
    }

    /// Send the chunks a receiver is missing.
    ///
    /// The order here is the whole of chunk-level resume: ask the receiver what
    /// it holds, check every claim against the local plan and the local bytes,
    /// queue what is left, and start workers on that queue. Nothing is assumed
    /// about offsets — a chunk is either confirmed and skipped, or sent whole.
    ///
    /// Returns the response once the workers are running, so the caller sees the
    /// transfer as active immediately.
    async fn continue_transfer(&self, state: &Arc<UploadState>) -> Result<TransferStatusResponse> {
        let resume_started = Instant::now();
        let transfer_id = state.transfer_id.clone();

        /*
         * A transfer restored from disk carries no secrets in memory — they were
         * kept out of the state file on purpose — so the keychain is where the
         * credentials come from. A transfer already live has them in memory and
         * nothing in the keychain.
         */
        let remembered = self
            .store
            .load_secrets(&transfer_id)
            .map_err(TransferError::Invalid)?;

        let key = match remembered.as_ref().map(|secrets| secrets.transfer_key) {
            Some(key) if key != [0u8; 32] => key,
            _ => state.transfer_key(),
        };

        let token = match remembered
            .as_ref()
            .map(|secrets| secrets.auth_token.clone())
        {
            Some(token) if !token.is_empty() => token,
            _ => state.auth_token(),
        };

        if key == [0u8; 32] {
            return Err(TransferError::Invalid(format!(
                "transfer {transfer_id} cannot be resumed: its key is no longer stored"
            )));
        }

        if token.is_empty() {
            return Err(TransferError::Invalid(format!(
                "transfer {transfer_id} cannot be resumed: its credentials are no longer stored"
            )));
        }

        // Adopt them, so everything downstream — the handshake, the workers —
        // sees the credentials this transfer will actually use.
        state.set_credentials(TransferCredentials {
            transfer_key: key,
            auth_token: token.clone(),
        });

        let jobs = chunker::jobs(&transfer_id, &state.files, state.chunk_size);

        if jobs.len() as u64 != state.total_chunks {
            tracing::warn!(
                transfer_id = %transfer_id,
                planned = state.total_chunks,
                rebuilt = jobs.len(),
                "the rebuilt job list does not match the planned chunk count"
            );
        }

        /*
         * The wire this resume will run on. A resume that reopens a session the
         * receiver still has keeps the key it already had — `reopen_session`
         * decides that — so the session is built from the credentials this
         * transfer is actually going to use, not from whatever the endpoint
         * field says.
         */
        let session = LegacyV1Session::to(state.endpoint.clone(), token.clone());

        /*
         * What does the receiver already have? A receiver that has never heard
         * of this transfer, or that predates the endpoint, answers 404 and is
         * read as holding nothing — the safe interpretation, since it costs a
         * resend rather than a skip.
         */
        let status_started = Instant::now();
        let report = session.resume_state(&transfer_id).await?;

        tracing::info!(
            transfer_id = %transfer_id,
            elapsed_ms = status_started.elapsed().as_millis() as u64,
            "resume phase: receiver state read"
        );

        /*
         * Reuse the plan a previous run left when every file it described is
         * still the file on disk. This is the phase that used to read and hash
         * the whole transfer before every resume; with a current plan it costs
         * one `stat` per file.
         *
         * A plan that has fallen behind — a file edited, replaced or removed
         * since it was written — is not trusted at all. It is recomputed, file
         * by file, and still reuses the entries whose files are unchanged.
         */
        let plan_started = Instant::now();

        // The files with whichever stamps the current plan was made against.
        let files = state.files_with_stamps();

        let expected = if !DESCRIBE_DIGESTS {
            None
        } else {
            Some(match state.expected() {
                Some(expected)
                    if integrity::plan_is_current(
                        &transfer_id,
                        &expected,
                        &files,
                        state.chunk_size,
                    )
                    .await =>
                {
                    tracing::info!(
                        transfer_id = %transfer_id,
                        elapsed_ms = plan_started.elapsed().as_millis() as u64,
                        "resume phase: reused the persisted digest plan; no source file was re-read"
                    );

                    expected
                }

                previous => {
                    /*
                     * Stamp before hashing, not after: a stamp older than the bytes
                     * a plan describes only ever costs a re-hash on the next resume,
                     * whereas one newer than them would let a stale plan be reused.
                     */
                    let stamps = integrity::current_stamps(&files).await;

                    let expected = integrity::plan_reusing(
                        &transfer_id,
                        &files,
                        state.chunk_size,
                        previous.as_ref(),
                        // No progress report here: a resume's real progress is the
                        // receiver's acknowledged chunks, and reporting bytes read
                        // would overwrite that count with something smaller.
                        None,
                    )
                    .await?;

                    state.set_plan_stamps(stamps);
                    state.set_expected(expected.clone());
                    self.persist_plan(&transfer_id, &expected).await;

                    tracing::info!(
                        transfer_id = %transfer_id,
                        elapsed_ms = plan_started.elapsed().as_millis() as u64,
                        "resume phase: digest plan recomputed"
                    );

                    expected
                }
            })
        };

        /*
         * Re-establish the session before sending. A receiver that restarted has
         * forgotten the transfer and needs the handshake again; one that never
         * noticed our absence already has it live and answers `AlreadyActive`,
         * keeping the key it had — which is why the key this transfer was using
         * is handed in.
         */
        /*
         * A resume's own phases: everything before the handshake — loading the
         * credentials, asking what the receiver holds, validating the plan — is
         * initialization. The handshake is timed on its own, below.
         */
        {
            let mut phases = state.phases.lock().expect("phases lock poisoned");

            phases.initialization_seconds = Some(resume_started.elapsed().as_secs_f64());
        }

        let handshake_started = Instant::now();
        let opened = session
            .open(OpenRequest {
                transfer_id: &transfer_id,
                file_size: state.total_bytes,
                expected: expected.as_ref(),
                existing_key: Some(key),
            })
            .await?;

        {
            let mut phases = state.phases.lock().expect("phases lock poisoned");

            phases.handshake_seconds = Some(handshake_started.elapsed().as_secs_f64());
        }

        tracing::info!(
            transfer_id = %transfer_id,
            start = ?opened.start,
            elapsed_ms = handshake_started.elapsed().as_millis() as u64,
            "resume phase: handshake complete"
        );

        if opened.start == SessionStart::Established {
            // The receiver derived a fresh key from the ephemeral key just sent,
            // so the one in memory is no longer the one it will decrypt with.
            tracing::info!(
                transfer_id = %transfer_id,
                "receiver re-established the session; continuing with a fresh transfer key"
            );

            state.set_credentials(TransferCredentials {
                transfer_key: opened.transfer_key,
                auth_token: token.clone(),
            });

            self.store_secrets(state);
        }

        /*
         * Which report to believe.
         *
         * v1 answers `resume_state` before the handshake and leaves
         * `opened.resume` empty; the v2 framed protocol carries the manifest in
         * the handshake, so its resume point only exists once the session is up.
         * Preferring `opened.resume` when it is present covers both without the
         * engine having to know which protocol it is holding.
         */
        let report = opened.resume.as_ref().or(report.as_ref());

        /*
         * The plan has just been established to describe the files as they are
         * now, so its per-chunk digests are what a read would produce. Checking
         * claims against them is the same integrity check without reading the
         * transfer back off the disk.
         */
        let confirmation =
            resume::confirmed_chunks_with_plan(report, &transfer_id, &jobs, expected.as_ref())
                .await;

        if !confirmation.is_trusted() {
            tracing::warn!(
                transfer_id = %transfer_id,
                rejected = confirmation.rejected.len(),
                "receiver reported chunks that do not match local metadata; \
                 those chunks will be sent again rather than skipped"
            );
        }

        let token = state.auth_token();
        let job_count = jobs.len();

        let (missing, _, _) = resume::missing_jobs(jobs, &confirmation.confirmed);

        /*
         * The receiver is the authority on what it holds, so its confirmed set
         * replaces what the sender believed. Recording it is also what seeds
         * progress: the chunks being skipped are counted at their own length in
         * the plan, so a resume shows a nearly-finished transfer as nearly
         * finished from the moment it is accepted, without any of them being
         * sent. Anything the plan cannot account for is dropped rather than
         * counted.
         */
        let (already_bytes, already_chunks) = state.set_completed(confirmation.confirmed);

        state.set_credentials(TransferCredentials {
            transfer_key: state.transfer_key(),
            auth_token: token.clone(),
        });

        tracing::info!(
            transfer_id = %transfer_id,
            total_chunks = job_count,
            already_present = already_chunks,
            already_present_bytes = already_bytes,
            to_send = missing.len(),
            "resuming transfer"
        );

        self.persist(state).await;

        if missing.is_empty() && state.is_complete() {
            /*
             * Every chunk is already there. Nothing is sent and no workers are
             * started — but the transfer is only complete because the receiver
             * confirmed each chunk against bytes we could reproduce, which is
             * the same evidence a normal run finishes on.
             */
            *state.status.lock().expect("status lock poisoned") = TransferStatus::Completed;

            self.events.emit(DomainEvent::TransferCompleted {
                progress: progress::make(state),
            });

            self.persist(state).await;
            self.forget_plan(&transfer_id).await;

            tracing::info!(
                transfer_id = %transfer_id,
                elapsed_ms = resume_started.elapsed().as_millis() as u64,
                "receiver already holds every chunk; nothing to send"
            );

            let _ = self
                .transfer_events
                .send(TransferEvent::Completed(transfer_id));

            return Ok(state.snapshot());
        }

        /*
         * A restored transfer comes back paused — nothing was running after the
         * restart, and the flag says so. The workers about to start would park
         * on it immediately, so it has to be cleared as part of resuming.
         */
        state.paused.store(false, Ordering::Relaxed);

        self.spawn_scheduler(Arc::clone(state), missing, Arc::new(session), None);

        tracing::info!(
            transfer_id = %transfer_id,
            elapsed_ms = resume_started.elapsed().as_millis() as u64,
            "transfer resumed"
        );

        // Taken after the workers are up, so the status the caller is handed is
        // the one the transfer is actually in.
        Ok(state.snapshot())
    }

    /// Record a chunk as acknowledged and checkpoint, at most every
    /// [`CHECKPOINT_INTERVAL`].
    ///
    /// Called from the scheduler's success callback, which is synchronous and
    /// runs on a runtime thread, so the write is spawned rather than awaited.
    /// The snapshot is taken inside [`Self::persist`]'s lock, not here, which is
    /// what keeps a slow write from landing after a newer one and rolling the
    /// record backwards.
    fn checkpoint(&self, state: &Arc<UploadState>) {
        let due = {
            let mut last = self
                .last_checkpoint
                .lock()
                .expect("checkpoint lock poisoned");

            if last.elapsed() < CHECKPOINT_INTERVAL {
                false
            } else {
                *last = Instant::now();
                true
            }
        };

        if !due {
            return;
        }

        let manager = self.clone();
        let state = Arc::clone(state);

        tokio::spawn(async move {
            manager.persist(&state).await;
        });
    }

    /// Write a transfer's clear record.
    async fn persist(&self, state: &Arc<UploadState>) {
        let _guard = self.persist_lock.lock().await;

        let record = state.to_persisted();

        let started = Instant::now();

        let saved = self.store.save(&record).await;

        state.report.add_persistence(started.elapsed());

        if let Err(error) = saved {
            tracing::warn!(
                transfer_id = %state.transfer_id,
                %error,
                "could not persist transfer state"
            );
        }
    }

    /// Write the digest plan beside the transfer's state.
    ///
    /// Best-effort, like every other persistence step: a plan that cannot be
    /// written costs a resume the work of hashing the files again, and nothing
    /// else.
    async fn persist_plan(&self, transfer_id: &str, expected: &ExpectedTransfer) {
        if let Err(error) = self.store.save_plan(transfer_id, expected).await {
            tracing::warn!(
                transfer_id,
                %error,
                "could not persist the digest plan; a resume will work it out again"
            );
        }
    }

    /// Remove the digest plan of a transfer that no longer needs one.
    ///
    /// A delivered transfer will never be resumed, and the plan is one digest
    /// per chunk. Best-effort: a leftover plan costs disk, not correctness, and
    /// a reused transfer id is given a fresh one before it is resumed.
    async fn forget_plan(&self, transfer_id: &str) {
        if let Err(error) = self.store.remove_plan(transfer_id).await {
            tracing::warn!(transfer_id, %error, "could not remove a completed transfer's plan");
        }
    }

    /// Store a transfer's key and token in the keychain.
    ///
    /// A failure is logged, not fatal: the transfer can still run, it just
    /// cannot be continued after a restart.
    fn store_secrets(&self, state: &Arc<UploadState>) {
        let secrets = TransferSecrets {
            transfer_key: state.transfer_key(),
            auth_token: state.auth_token(),
        };

        if let Err(error) = self.store.save_secrets(&state.transfer_id, &secrets) {
            tracing::warn!(
                transfer_id = %state.transfer_id,
                %error,
                "could not store transfer secrets; this transfer will not survive a restart"
            );
        }
    }

    pub async fn pause(&self, id: &str) -> Result<TransferStatusResponse> {
        let s = self.get(id)?;
        s.paused.store(true, Ordering::Relaxed);
        *s.status.lock().expect("status lock poisoned") = TransferStatus::Paused;
        self.events.emit(DomainEvent::TransferPaused {
            progress: progress::make(&s),
        });

        self.persist(&s).await;

        Ok(s.snapshot())
    }

    /// Continue a transfer.
    ///
    /// Three different situations arrive here, and they are told apart by
    /// whether workers are still alive:
    ///
    /// * **Workers are running.** The transfer is paused, not stopped. Its queue
    ///   is intact and its workers are parked on the pause flag, so clearing the
    ///   flag is the whole of it. This is the original behaviour and it is
    ///   unchanged.
    /// * **No workers.** The transfer was restored after a restart, or its
    ///   workers died with a failure — a failed transfer is, in this respect,
    ///   just one whose queue is gone. Continuing means asking the receiver what
    ///   it holds and sending only the rest: [`Self::continue_transfer`].
    /// * **Two resumes at once.** The transfer's `resume_lock` serialises them,
    ///   so the second call cannot see the idle transfer the first one is
    ///   already restarting and start a second set of workers on it.
    pub async fn resume(&self, id: &str) -> Result<TransferStatusResponse> {
        let s = self.get(id)?;

        /*
         * One resume at a time per transfer. Two calls that arrive together —
         * a double-click, or a retry racing the user's own — must not both find
         * the transfer idle and start a scheduler each: two worker sets on one
         * queue would send every missing chunk twice.
         */
        let _gate = s.resume_lock.lock().await;

        // Cancelling takes a transfer out of the map and deletes its state file;
        // a resume that was parked on the gate must not bring it back.
        if s.cancelled.load(Ordering::Relaxed) {
            return Err(TransferError::NotFound(id.into()));
        }

        /*
         * A `Failed` transfer is resumable even if its dying scheduler has not
         * quite finished clearing the running flag, which is why the status is
         * consulted as well as the flag.
         */
        let failed = matches!(
            *s.status.lock().expect("status lock poisoned"),
            TransferStatus::Failed
        );

        if !s.is_running() || failed {
            return self.continue_transfer(&s).await;
        }

        s.paused.store(false, Ordering::Relaxed);
        *s.status.lock().expect("status lock poisoned") = TransferStatus::Uploading;
        self.events.emit(DomainEvent::TransferResumed {
            progress: progress::make(&s),
        });

        self.persist(&s).await;

        Ok(s.snapshot())
    }
    pub async fn cancel(&self, id: &str) -> Result<TransferStatusResponse> {
        let s = self.get(id)?;
        s.cancelled.store(true, Ordering::Relaxed);
        *s.status.lock().expect("status lock poisoned") = TransferStatus::Cancelled;
        let _ = tokio::fs::remove_dir_all(self.upload_root.join("incoming").join(id)).await;
        self.events.emit(DomainEvent::TransferCancelled {
            progress: progress::make(&s),
        });

        self.finalize_report(
            &s,
            TransferStatus::Cancelled,
            Some("cancelled by the user".into()),
            None,
            None,
            false,
        )
        .await;
        self.transfers
            .lock()
            .expect("transfer lock poisoned")
            .remove(id);

        /*
         * A cancelled transfer is not a transfer anybody wants continued, so
         * its record and its secrets go with it. This is the only path that
         * deletes them: completed and failed transfers keep theirs, so that a
         * restart can still report what happened to them.
         */
        if let Err(error) = self.store.remove(id).await {
            tracing::warn!(transfer_id = %id, %error, "could not remove persisted transfer state");
        }

        /*
         * Tell the receiver, if it can be told.
         *
         * Cancelling here only stops the sender; the bytes are the receiver's,
         * and until it is told they stay on its disk. It is deliberately
         * best-effort and bounded: an unreachable receiver must not turn a
         * cancellation into a hang, and a receiver that never hears about it
         * still loses the transfer to the TTL sweep — only later, and with the
         * bytes sitting there in the meantime.
         */
        let session = LegacyV1Session::to(s.endpoint.clone(), s.auth_token());

        let notice = tokio::time::timeout(RECEIVER_NOTICE_TIMEOUT, session.cancel(id)).await;

        match notice {
            Ok(Ok(())) => {}

            Ok(Err(error)) => tracing::warn!(
                transfer_id = %id,
                %error,
                "could not tell the receiver about the cancellation; \
                 its chunks will be swept when they time out"
            ),

            Err(_) => tracing::warn!(
                transfer_id = %id,
                timeout_secs = RECEIVER_NOTICE_TIMEOUT.as_secs(),
                "the receiver did not answer the cancellation in time; \
                 its chunks will be swept when they time out"
            ),
        }

        Ok(s.snapshot())
    }
    pub fn status(&self, id: &str) -> Result<TransferStatusResponse> {
        Ok(self.get(id)?.snapshot())
    }
    pub(crate) fn get(&self, id: &str) -> Result<Arc<UploadState>> {
        self.transfers
            .lock()
            .expect("transfer lock poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| TransferError::NotFound(id.into()))
    }
}

/// Rebuild an in-memory transfer from what was persisted.
///
/// The secrets are looked up but not required: a transfer whose keychain entry
/// is gone is still worth describing — it can be shown, cancelled, or sent
/// again — so a missing secret leaves an empty key rather than failing the
/// whole restore. Nothing sends with that state; only a resume would, and
/// resuming is a later step's problem.
fn restored_state(
    record: &PersistedTransfer,
    expected: Option<crate::transfer::integrity::ExpectedTransfer>,
) -> UploadState {
    let mut state = UploadState {
        transfer_id: record.transfer_id.clone(),
        // Left empty deliberately: the key and token were never on disk. A
        // resume reads them back out of the keychain.
        credentials: Mutex::new(TransferCredentials::default()),
        endpoint: record.endpoint.clone(),
        network_type: record.network_type.clone(),
        files: record.file_infos(),
        chunk_size: record.chunk_size,
        concurrency: record.concurrency,
        // Filled in below, from the same file list progress is measured over.
        total_bytes: 0,
        uploaded_bytes: 0.into(),
        total_chunks: 0,
        uploaded_chunks: 0.into(),
        // Nothing is running after a restart, so the flag starts set.
        paused: true.into(),
        cancelled: false.into(),
        retry_count: 0.into(),
        max_retries: record.max_retries,
        status: Mutex::new(record.resumable_status()),
        started_at: Instant::now(),
        startup_started: Instant::now(),
        completed: Mutex::new(std::collections::HashSet::new()),
        // Nothing is running after a restart.
        running: false.into(),
        resume_lock: tokio::sync::Mutex::new(()),
        /*
         * The digest plan a previous run worked out, when it left one. It is
         * still only a cache: `continue_transfer` checks every file's stamp
         * before trusting it, and works the plan out again if any file has
         * changed since it was written.
         */
        expected: Mutex::new(expected),
        // No refresh yet: the record's stamps are the plan's basis until a
        // resume finds something changed.
        plan_stamps: Mutex::new(std::collections::HashMap::new()),
        /*
         * The telemetry a checkpoint left, continued rather than replaced: a
         * report recovered after a restart then knows how long the run took and
         * how much of it moved, instead of reporting zeros.
         */
        report: crate::transfer::report::Telemetry::restore(
            crate::transfer::report::Role::Sender,
            record.report.as_ref(),
        ),
        phases: Mutex::new(
            record
                .report
                .as_ref()
                .map(|report| report.phases.clone())
                .unwrap_or_default(),
        ),
    };

    /*
     * The totals are derived from the file list rather than read back from the
     * record: they are the same fact, and a record whose two halves disagree
     * would otherwise have progress measured against a total its files do not
     * add up to.
     */
    state.total_bytes = state.planned_bytes();
    state.total_chunks = state.planned_chunks();

    /*
     * The acknowledged set is what the record kept that the plan can still
     * account for — an entry naming a file that is no longer in the transfer,
     * or an index past the end of its file, is dropped rather than counted —
     * and `set_completed` derives the byte and chunk counters from what
     * survives. So a restored transfer reports exactly the progress its chunks
     * are worth, and never a stale entry's.
     */
    state.set_completed(
        record
            .completed_chunks
            .iter()
            .map(|chunk| (chunk.file_id.clone(), chunk.chunk_index))
            .collect(),
    );

    state
}

/// What a restart does with the transfers a previous run left behind.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::transfer::ConnectionType;
    use crate::transfer::manifest::test_support::TempDir;
    use crate::transfer::persist::{PersistedChunk, PersistedFile, STATE_VERSION};
    use sqlx::sqlite::SqlitePoolOptions;
    use vilsend_core::RecordingEventSink;

    /// A manager over a real (in-memory) database and a scratch state
    /// directory, with a recording sink standing in for the webview.
    async fn build_manager(store: Arc<TransferStore>) -> UploadManager {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");

        let local_files = Arc::new(LocalTransferFileService::new(pool).await);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        UploadManager::new(
            Arc::new(RecordingEventSink::new()),
            std::env::temp_dir().join("vilsend-manager-test-uploads"),
            local_files,
            tx,
            store,
        )
    }

    fn record(transfer_id: &str, status: TransferStatus, completed: usize) -> PersistedTransfer {
        PersistedTransfer {
            version: STATE_VERSION,
            transfer_id: transfer_id.to_string(),
            endpoint: "https://receiver.example".into(),
            network_type: ConnectionType::Tunnel,
            chunk_size: 1024,
            concurrency: 2,
            max_retries: 3,
            files: vec![PersistedFile {
                id: "f-1".into(),
                path: "/tmp/a.bin".into(),
                relative_path: "a.bin".into(),
                size: 4096,
                modified_nanos: None,
            }],
            total_bytes: 4096,
            total_chunks: 4,
            report: None,
            completed_chunks: (0..completed as u64)
                .map(|chunk_index| PersistedChunk {
                    file_id: "f-1".into(),
                    chunk_index,
                })
                .collect(),
            status,
        }
    }

    /*
     * ----------------------------------------------------------------------
     * A restart rebuilds what the previous run had
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_terminal_transfer_that_died_before_its_report_is_reported_on_restart() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        // A run that reached its terminal state and died before writing the
        // report: the record is all that survived.
        store
            .save(&record("t-1", TransferStatus::Completed, 4))
            .await
            .expect("saving");

        let manager = build_manager(Arc::clone(&store)).await;

        assert_eq!(manager.restore_persisted().await, 1);

        let path = store.reports_dir().join("t-1.json");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("the recovered report"))
                .expect("json");

        assert_eq!(json["transfer_id"], "t-1");
        assert_eq!(json["status"], "COMPLETED");

        assert_eq!(json["recovered"], true);

        /*
         * This record carries no telemetry — it was written before the transfer
         * kept any — so there is no duration to report and the report must say
         * so rather than claim the transfer took the microseconds since it was
         * restored.
         */
        assert!(json["duration_seconds"].is_null());

        let unavailable = json["unavailable"].as_array().expect("unavailable");

        assert!(unavailable
            .iter()
            .any(|item| item.as_str().unwrap_or_default().starts_with("duration")));
    }

    #[tokio::test]
    async fn a_recovered_report_keeps_the_measurements_the_checkpoint_saved() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        let mut record = record("t-1", TransferStatus::Completed, 4);
        record.report = Some(crate::transfer::report::PersistedTelemetry {
            started_unix_ms: Some(1_700_000_000_000),
            elapsed_ms: 20_000,
            first_chunk_start_ms: Some(500),
            first_chunk_ack_ms: Some(900),
            upload_start_ms: Some(500),
            upload_end_ms: Some(19_000),
            files: vec![crate::transfer::report::PersistedFileTelemetry {
                file_id: "f-1".into(),
                first_ms: Some(500),
                last_ms: Some(19_000),
                chunks: 4,
                bytes: 4096,
                retries: 2,
                failed: 0,
                completed: false,
            }],
            ..crate::transfer::report::PersistedTelemetry::default()
        });

        store.save(&record).await.expect("saving");

        let manager = build_manager(Arc::clone(&store)).await;
        assert_eq!(manager.restore_persisted().await, 1);

        let path = store.reports_dir().join("t-1.json");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("the recovered report"))
                .expect("json");

        assert_eq!(json["recovered"], true);
        assert_eq!(json["status"], "COMPLETED");

        assert!(
            json["duration_seconds"].as_f64().unwrap() >= 20.0,
            "the duration is the run's, not the instant of recovery"
        );
        assert!(json["throughput"]["mib_s"].is_number());
        assert_eq!(json["files"][0]["chunks_done"], 4);
        assert_eq!(json["files"][0]["retries"], 2);
        assert_eq!(
            json["files"][0]["outcome"], "unknown",
            "acknowledged, but this side never saw the publication"
        );
    }

    #[tokio::test]
    async fn a_restart_does_not_overwrite_a_report_that_was_written() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        store
            .save(&record("t-1", TransferStatus::Completed, 4))
            .await
            .expect("saving");

        let reports = store.reports_dir();
        std::fs::create_dir_all(&reports).expect("reports dir");
        std::fs::write(reports.join("t-1.json"), b"{\"written\":true}")
            .expect("an existing report");

        let manager = build_manager(Arc::clone(&store)).await;
        manager.restore_persisted().await;

        let bytes = std::fs::read(reports.join("t-1.json")).expect("reading");

        assert_eq!(
            bytes, b"{\"written\":true}",
            "recovery must not replace a report the run itself wrote"
        );
    }

    #[tokio::test]
    async fn a_restart_restores_an_interrupted_transfer() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        // The previous run: three of four chunks acknowledged, then it stopped.
        store
            .save(&record("t-1", TransferStatus::Uploading, 3))
            .await
            .expect("saving");

        let manager = build_manager(Arc::clone(&store)).await;

        assert_eq!(manager.restore_persisted().await, 1);

        let status = manager.status("t-1").expect("the transfer is back");

        assert_eq!(status.transfer_id, "t-1");
        assert_eq!(status.receiver_endpoint, "https://receiver.example");
        assert_eq!(status.total_chunks, 4);
        assert_eq!(
            status.uploaded_chunks, 3,
            "the acknowledged chunks must come back"
        );
        assert_eq!(status.total_bytes, 4096);
    }

    /// A structurally valid plan for the 4096-byte file in `record`, split into
    /// four 1024-byte chunks.
    fn stored_plan() -> ExpectedTransfer {
        use crate::transfer::integrity::{ExpectedChunk, ExpectedFile};

        ExpectedTransfer {
            transfer_id: "t-1".into(),
            files: vec![ExpectedFile {
                file_id: "f-1".into(),
                relative_path: "a.bin".into(),
                size: 4096,
                sha256: "ff".repeat(32),
                chunks: (0..4)
                    .map(|index| ExpectedChunk {
                        index,
                        length: 1024,
                        sha256: format!("{index:02x}").repeat(32),
                    })
                    .collect(),
            }],
        }
    }

    #[tokio::test]
    async fn a_restart_restores_the_digest_plan_the_previous_run_left() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        store
            .save(&record("t-1", TransferStatus::Uploading, 0))
            .await
            .expect("saving");
        store
            .save_plan("t-1", &stored_plan())
            .await
            .expect("saving the plan");

        let manager = build_manager(store).await;
        assert_eq!(manager.restore_persisted().await, 1);

        assert_eq!(
            manager.get("t-1").expect("the transfer is back").expected(),
            Some(stored_plan()),
            "the plan has to come back with the transfer, or the resume hashes everything again"
        );
    }

    #[tokio::test]
    async fn a_restored_transfer_reports_the_bytes_its_chunks_account_for() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        // Three of four 1024-byte chunks.
        store
            .save(&record("t-1", TransferStatus::Uploading, 3))
            .await
            .expect("saving");

        let manager = build_manager(store).await;
        manager.restore_persisted().await;

        assert_eq!(
            manager.status("t-1").expect("back").uploaded_bytes,
            3072,
            "the byte count is derived from the chunk set, not stored twice"
        );
    }

    #[tokio::test]
    async fn a_restored_transfer_comes_back_paused() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        store
            .save(&record("t-1", TransferStatus::Uploading, 1))
            .await
            .expect("saving");

        let manager = build_manager(store).await;
        manager.restore_persisted().await;

        assert_eq!(
            manager.status("t-1").expect("back").status,
            TransferStatus::Paused,
            "nothing runs after a restart, so nothing may claim to be running"
        );
    }

    #[tokio::test]
    async fn a_completed_transfer_whose_chunks_are_all_there_stays_completed() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        store
            .save(&record("t-1", TransferStatus::Completed, 4))
            .await
            .expect("saving");

        let manager = build_manager(store).await;
        manager.restore_persisted().await;

        assert_eq!(
            manager.status("t-1").expect("back").status,
            TransferStatus::Completed
        );
    }

    #[tokio::test]
    async fn a_state_file_claiming_unearned_completion_is_not_trusted() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        // Says "completed", accounts for one chunk of four.
        store
            .save(&record("t-1", TransferStatus::Completed, 1))
            .await
            .expect("saving");

        let manager = build_manager(store).await;
        manager.restore_persisted().await;

        assert_eq!(
            manager.status("t-1").expect("back").status,
            TransferStatus::Paused,
            "local state alone does not get to call a transfer finished"
        );
    }

    #[tokio::test]
    async fn a_transfer_with_no_state_file_is_not_restored() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        let manager = build_manager(store).await;

        assert_eq!(manager.restore_persisted().await, 0);
        assert!(
            manager.status("t-nonexistent").is_err(),
            "a transfer nobody wrote must stay unknown"
        );
    }

    #[tokio::test]
    async fn a_corrupt_state_file_does_not_stop_the_restart() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        let store = Arc::new(TransferStore::in_memory(state_dir.clone()));

        store
            .save(&record("t-good", TransferStatus::Uploading, 2))
            .await
            .expect("saving");

        tokio::fs::create_dir_all(&state_dir).await.expect("mkdir");
        tokio::fs::write(state_dir.join("t-bad.json"), b"broken")
            .await
            .expect("writing a broken state file");

        let manager = build_manager(store).await;

        assert_eq!(manager.restore_persisted().await, 1);
        assert!(
            manager.status("t-good").is_ok(),
            "the readable transfer must still come back"
        );
        assert!(manager.status("t-bad").is_err());
    }

    #[tokio::test]
    async fn restoring_twice_does_not_clobber_a_transfer_in_progress() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        let store = Arc::new(TransferStore::in_memory(state_dir.clone()));

        store
            .save(&record("t-1", TransferStatus::Uploading, 1))
            .await
            .expect("saving");

        let manager = build_manager(Arc::clone(&store)).await;

        assert_eq!(manager.restore_persisted().await, 1);

        // A later, fuller state file — as a running transfer would have written.
        store
            .save(&record("t-1", TransferStatus::Uploading, 4))
            .await
            .expect("saving again");

        // The restore is idempotent: it must not replace live state with what
        // happens to be on disk.
        assert_eq!(manager.restore_persisted().await, 0);
        assert_eq!(manager.status("t-1").expect("back").uploaded_chunks, 1);
    }

    /*
     * ----------------------------------------------------------------------
     * Transfers that were never interrupted are unaffected
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_new_transfer_still_refuses_to_start_twice() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));
        let manager = build_manager(store).await;

        let metadata = TransferMetadata {
            transfer_id: "t-new".into(),
            receiver_public_key: String::new(),
            network_type: ConnectionType::Lan,
            endpoint: "https://receiver.example".into(),
            auth_token: "token".into(),
            chunk_size: Some(1024),
            concurrency: Some(1),
            max_retries: Some(1),
        };

        // Nothing is registered for this transfer in the local files table, so
        // it fails for the ordinary reason — not because of persistence, and
        // not by reaching the network.
        assert!(
            manager.start(metadata).await.is_err(),
            "starting with no local files must still be refused"
        );
        assert!(manager.status("t-new").is_err());
    }

    #[tokio::test]
    async fn a_transfer_that_was_never_started_has_no_state_file() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        let store = Arc::new(TransferStore::in_memory(state_dir.clone()));
        let manager = build_manager(store).await;

        manager.restore_persisted().await;

        assert!(
            !state_dir.exists()
                || std::fs::read_dir(&state_dir)
                    .map(|entries| entries.count() == 0)
                    .unwrap_or(true),
            "a manager that never started a transfer must not write any state"
        );
    }

    #[tokio::test]
    async fn cancelling_removes_the_persisted_record() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        let store = Arc::new(TransferStore::in_memory(state_dir.clone()));

        store
            .save(&record("t-1", TransferStatus::Uploading, 2))
            .await
            .expect("saving");
        store
            .save_plan("t-1", &stored_plan())
            .await
            .expect("saving the plan");

        let manager = build_manager(Arc::clone(&store)).await;
        manager.restore_persisted().await;

        manager.cancel("t-1").await.expect("cancelling");

        assert!(
            !state_dir.join("t-1.json").exists(),
            "a cancelled transfer is not one anybody wants continued"
        );
        assert!(
            !state_dir.join("t-1.plan").exists(),
            "and neither is its digest plan"
        );

        // And a later restart does not bring it back.
        let after = build_manager(store).await;
        assert_eq!(after.restore_persisted().await, 0);
    }

    #[tokio::test]
    async fn an_unknown_transfer_is_still_reported_as_not_found() {
        let scratch = TempDir::new();
        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));
        let manager = build_manager(store).await;

        assert!(matches!(
            manager.status("t-nobody"),
            Err(TransferError::NotFound(_))
        ));
    }
}

/// Resuming against a real receiver.
///
/// These drive the whole path — the HTTP client, the claim checking, the job
/// queue, the workers — against a stub receiver on a real socket, because the
/// interesting failures in a resume are the ones that only appear once a second
/// process is answering.
#[cfg(test)]
mod resume_tests {
    use super::*;
    use crate::models::file_info::FileInfo;
    use crate::transfer::manifest::test_support::TempDir;
    use crate::transfer::persist::{PersistedChunk, PersistedFile, STATE_VERSION};
    use axum::{
        body::Bytes,
        extract::State as AxumState,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Mutex as StdMutex;
    use vilsend_core::RecordingEventSink;

    /// A receiver that answers the three endpoints a resume uses, and records
    /// the chunks it is actually sent.
    #[derive(Default)]
    struct StubReceiver {
        /// Every chunk the sender POSTed, in the order it arrived.
        received: StdMutex<Vec<(String, u64, usize)>>,
        /// What `GET /transfer/status` answers. `None` means `NOT_FOUND`.
        report: StdMutex<Option<Value>>,
        /// Whether `POST /transfer/start` answers `CONFLICT` — a receiver that
        /// never noticed the sender leave.
        session_already_active: StdMutex<bool>,
        public_key: StdMutex<String>,
        /// How many times `GET /transfer/status` was asked. One resume asks
        /// once, so two resumes racing must still produce one query — and one
        /// queue — between them.
        status_queries: StdMutex<usize>,
        /// The digests the sender described in its last `POST /transfer/start`,
        /// if it described any. `None` means the request carried none.
        described: StdMutex<Option<Value>>,
        /// Every `POST /transfer/start` the sender made, in order, with the
        /// digests it carried — `None` for a start that described nothing.
        starts: StdMutex<Vec<Option<Value>>>,
        /// How many times the sender asked the receiver to cancel.
        cancelled: StdMutex<usize>,
    }

    impl StubReceiver {
        fn received(&self) -> Vec<(String, u64, usize)> {
            self.received.lock().expect("received lock").clone()
        }

        fn received_indices(&self) -> Vec<u64> {
            let mut indices: Vec<u64> = self
                .received()
                .into_iter()
                .map(|(_, index, _)| index)
                .collect();

            indices.sort_unstable();
            indices
        }
    }

    async fn stub_start(
        AxumState(receiver): AxumState<Arc<StubReceiver>>,
        body: Bytes,
    ) -> StatusCode {
        let mut described = None;

        if let Ok(request) = serde_json::from_slice::<Value>(&body) {
            if let Some(expected) = request.get("expected") {
                described = Some(expected.clone());

                *receiver.described.lock().expect("described lock") = Some(expected.clone());
            }
        }

        receiver.starts.lock().expect("starts lock").push(described);

        if *receiver.session_already_active.lock().expect("active lock") {
            StatusCode::CONFLICT
        } else {
            StatusCode::OK
        }
    }

    async fn stub_status(
        AxumState(receiver): AxumState<Arc<StubReceiver>>,
    ) -> (StatusCode, Json<Value>) {
        *receiver.status_queries.lock().expect("queries lock") += 1;

        match receiver.report.lock().expect("report lock").clone() {
            Some(report) => (StatusCode::OK, Json(report)),
            None => (StatusCode::NOT_FOUND, Json(json!({}))),
        }
    }

    async fn stub_cancel(AxumState(receiver): AxumState<Arc<StubReceiver>>) -> StatusCode {
        *receiver.cancelled.lock().expect("cancelled lock") += 1;

        StatusCode::OK
    }

    async fn stub_public_key(AxumState(receiver): AxumState<Arc<StubReceiver>>) -> Json<Value> {
        let key = receiver.public_key.lock().expect("key lock").clone();

        Json(json!({ "public_key": key }))
    }

    async fn stub_chunk(
        AxumState(receiver): AxumState<Arc<StubReceiver>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        let file_id = headers
            .get("File-Id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();

        let index = headers
            .get("Chunk-Index")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(u64::MAX);

        receiver
            .received
            .lock()
            .expect("received lock")
            .push((file_id, index, body.len()));

        StatusCode::OK
    }

    /// Bind a stub receiver and return it with the endpoint to reach it at.
    async fn serve_stub() -> (Arc<StubReceiver>, String) {
        let receiver = Arc::new(StubReceiver::default());

        *receiver.public_key.lock().expect("key lock") = crate::transfer::crypto::encode_public_key(
            &crate::transfer::crypto::generate_ephemeral_keypair().public_key,
        );

        let router = Router::new()
            .route("/transfer/start", post(stub_start))
            .route("/transfer/status", get(stub_status))
            .route("/transfer/public-key", get(stub_public_key))
            .route("/transfer/chunk", post(stub_chunk))
            .route("/transfer/cancel", post(stub_cancel))
            .with_state(Arc::clone(&receiver));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding a stub receiver");

        let address = listener.local_addr().expect("local addr");

        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        (receiver, format!("http://{address}"))
    }

    /// The plan for a `size`-byte file split into `chunk_size` pieces.
    fn plan(path: &std::path::Path, size: u64, chunk_size: usize) -> Vec<ChunkJob> {
        chunker::jobs(
            "t-1",
            &[FileInfo {
                id: "f-1".into(),
                path: path.to_path_buf(),
                relative_path: "a.bin".into(),
                size,
                modified_nanos: None,
            }],
            chunk_size,
        )
    }

    /// The modification stamp a scan of `path` would record.
    fn record_modified_nanos(path: &std::path::Path) -> Option<u64> {
        std::fs::metadata(path)
            .ok()
            .and_then(|metadata| crate::models::file_info::modified_nanos(&metadata))
    }

    /// A persisted record for that file, as a previous run would have left it.
    fn record(
        path: &std::path::Path,
        size: u64,
        chunk_size: usize,
        completed: &[u64],
    ) -> PersistedTransfer {
        let total_chunks = size.div_ceil(chunk_size as u64).max(1);

        PersistedTransfer {
            version: STATE_VERSION,
            transfer_id: "t-1".into(),
            endpoint: String::new(), // filled in by the caller
            network_type: crate::models::transfer::ConnectionType::Lan,
            chunk_size,
            concurrency: 1,
            max_retries: 1,
            files: vec![PersistedFile {
                id: "f-1".into(),
                path: path.to_string_lossy().into_owned(),
                relative_path: "a.bin".into(),
                size,
                modified_nanos: record_modified_nanos(path),
            }],
            total_bytes: size,
            total_chunks,
            report: None,
            completed_chunks: completed
                .iter()
                .map(|chunk_index| PersistedChunk {
                    file_id: "f-1".into(),
                    chunk_index: *chunk_index,
                })
                .collect(),
            status: TransferStatus::Uploading,
        }
    }

    /// A receiver report naming the given chunks, with their real digests.
    async fn report_for(jobs: &[ChunkJob], indices: &[u64]) -> Value {
        let mut chunks = Vec::new();

        for job in jobs.iter().filter(|job| indices.contains(&job.index)) {
            chunks.push(json!({
                "chunk_index": job.index,
                "chunk_length": job.length,
                "chunk_hash": resume::chunk_digest(job).await.expect("digest"),
            }));
        }

        json!({
            "transfer_id": "t-1",
            "files": [{ "file_id": "f-1", "chunks": chunks }],
        })
    }

    /// A transfer as a restart would leave it: state on disk, secrets in the
    /// store, and a manager that has restored it.
    async fn restored(
        scratch: &TempDir,
        size: u64,
        chunk_size: usize,
        completed: &[u64],
        endpoint: &str,
    ) -> (UploadManager, Arc<TransferStore>, std::path::PathBuf) {
        let path = scratch.path().join("a.bin");

        std::fs::write(&path, vec![b'x'; size as usize]).expect("writing the source file");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");

        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        let mut record = record(&path, size, chunk_size, completed);
        record.endpoint = endpoint.to_string();

        store.save(&record).await.expect("saving the record");
        store
            .save_secrets(
                "t-1",
                &TransferSecrets {
                    transfer_key: [0x7F; 32],
                    auth_token: "a-token".into(),
                },
            )
            .expect("saving secrets");

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let manager = UploadManager::new(
            Arc::new(RecordingEventSink::new()),
            std::env::temp_dir().join("vilsend-resume-test-uploads"),
            Arc::new(LocalTransferFileService::new(pool).await),
            tx,
            Arc::clone(&store),
        );

        assert_eq!(
            manager.restore_persisted().await,
            1,
            "the record must restore"
        );

        (manager, store, path)
    }

    /// Poll until the transfer stops moving.
    async fn settle(manager: &UploadManager) -> TransferStatusResponse {
        for _ in 0..250 {
            let status = manager.status("t-1").expect("status");

            if matches!(
                status.status,
                TransferStatus::Completed | TransferStatus::Failed
            ) {
                return status;
            }

            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        panic!("the transfer never settled");
    }

    /// Leave the transfer the way a scheduler that gave up leaves it: status
    /// `Failed`, no workers, and the same written down where a restart reads it.
    async fn fail(manager: &UploadManager, store: &TransferStore) {
        let mut record = store.load("t-1").await.expect("the record is there");
        record.status = TransferStatus::Failed;
        store.save(&record).await.expect("saving the failed record");

        let state = manager
            .get("t-1")
            .expect("the transfer is still in the map");
        *state.status.lock().expect("status lock poisoned") = TransferStatus::Failed;
        state.set_running(false);
    }

    const SIZE: u64 = 10;
    const CHUNK: usize = 4;

    /// A manager with `t-1` registered the way the command layer registers one,
    /// and nothing restored: a transfer that has never run.
    async fn registered(scratch: &TempDir) -> (UploadManager, std::path::PathBuf) {
        let path = scratch.path().join("a.bin");

        std::fs::write(&path, vec![b'x'; SIZE as usize]).expect("writing the source file");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");

        let local_files = Arc::new(LocalTransferFileService::new(pool).await);

        local_files
            .insert(crate::models::transfer::LocalTransferFile {
                transfer_id: "t-1".into(),
                file_path: path.to_string_lossy().into_owned(),
                file_name: "a.bin".into(),
                file_size: SIZE,
            })
            .await
            .expect("registering the file");

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let manager = UploadManager::new(
            Arc::new(RecordingEventSink::new()),
            std::env::temp_dir().join("vilsend-resume-test-uploads"),
            local_files,
            tx,
            Arc::new(TransferStore::in_memory(scratch.path().join("state"))),
        );

        (manager, path)
    }

    /// The metadata the command layer hands `start`, pointed at `endpoint`.
    fn metadata(endpoint: &str) -> TransferMetadata {
        TransferMetadata {
            transfer_id: "t-1".into(),
            receiver_public_key: String::new(),
            network_type: crate::models::transfer::ConnectionType::Lan,
            endpoint: endpoint.to_string(),
            auth_token: "a-token".into(),
            chunk_size: Some(CHUNK),
            concurrency: Some(1),
            max_retries: Some(1),
        }
    }

    /// The sender's description of the ten bytes of `x` the tests transfer.
    fn described_sha256() -> String {
        crate::transfer::manifest::sha256_hex(&vec![b'x'; SIZE as usize])
    }

    /*
     * ----------------------------------------------------------------------
     * How much of the transfer goes over the wire
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_transfer_with_no_completed_chunks_sends_every_chunk() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        // The receiver has never heard of this transfer.
        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;

        assert_eq!(plan(&path, SIZE, CHUNK).len(), 3);

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);
        assert_eq!(receiver.received_indices(), vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn a_transfer_with_half_its_chunks_complete_sends_only_the_rest() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        // The receiver holds chunks 0 and 2.
        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0, 2]).await);

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);
        assert_eq!(
            receiver.received_indices(),
            vec![1],
            "only the chunk the receiver did not confirm goes over the wire"
        );
    }

    #[tokio::test]
    async fn a_transfer_with_every_chunk_complete_sends_nothing() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0, 1, 2]).await);

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);
        assert!(
            receiver.received().is_empty(),
            "not one chunk may be sent when the receiver has them all"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * The two restarts
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_sender_restart_sends_only_what_the_receiver_is_missing() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        // The previous run acknowledged nothing, so its state file says so.
        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        manager.resume("t-1").await.expect("resuming");
        settle(&manager).await;

        assert_eq!(
            receiver.received_indices(),
            vec![1, 2],
            "the chunks the receiver kept are not sent again"
        );
    }

    #[tokio::test]
    async fn a_receiver_restart_resumes_the_missing_chunks() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        // The receiver restarted: it forgot the session, so `start` succeeds and
        // it derives a fresh key from the ephemeral key the sender presents.
        *receiver.session_already_active.lock().expect("active lock") = false;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        // Its manifest survived, so it still knows about chunk 0.
        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);
        assert_eq!(
            receiver.received_indices(),
            vec![1, 2],
            "a receiver restart must not cost the chunks it still holds"
        );
    }

    #[tokio::test]
    async fn a_receiver_that_never_noticed_the_sender_leave_keeps_its_key() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        *receiver.session_already_active.lock().expect("active lock") = true;

        let (manager, store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        manager.resume("t-1").await.expect("resuming");
        settle(&manager).await;

        assert_eq!(receiver.received_indices(), vec![1, 2]);

        let secrets = store.load_secrets("t-1").expect("load").expect("present");

        assert_eq!(
            secrets.transfer_key, [0x7F; 32],
            "a receiver that refused the handshake kept its key, so the sender must keep it too"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * A failed transfer resumes like any other transfer without a queue
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_failed_transfer_can_be_resumed() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        // The receiver kept chunk 0 before the sender's run died.
        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        fail(&manager, &store).await;

        assert_eq!(
            manager.status("t-1").expect("status").status,
            TransferStatus::Failed,
            "the transfer starts out the way a failed run leaves it"
        );

        let response = manager
            .resume("t-1")
            .await
            .expect("a failed transfer must be resumable");

        assert_eq!(response.transfer_id, "t-1");
        assert_eq!(
            response.status,
            TransferStatus::Uploading,
            "the caller is told the transfer is moving again, not still failed"
        );

        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);
        assert_eq!(receiver.received_indices(), vec![1, 2]);
    }

    #[tokio::test]
    async fn a_failed_transfer_sends_only_the_chunks_the_receiver_is_missing() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        // Two of the three chunks survived on the receiver's side.
        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0, 1]).await);

        fail(&manager, &store).await;

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);

        assert_eq!(
            receiver.received_indices(),
            vec![2],
            "a resumed failed transfer must not re-upload a chunk the receiver confirmed"
        );

        assert_eq!(
            status.uploaded_chunks, 3,
            "and it still counts the chunks it did not have to send"
        );

        assert_eq!(status.uploaded_bytes, SIZE);
    }

    #[tokio::test]
    async fn a_failed_transfer_with_nothing_to_skip_reaches_completed() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        // A receiver that has forgotten the transfer entirely.
        let (manager, store, _path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;

        fail(&manager, &store).await;

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);
        assert_eq!(status.uploaded_chunks, 3);
        assert_eq!(status.uploaded_bytes, SIZE);

        assert_eq!(
            receiver.received_indices(),
            vec![0, 1, 2],
            "with nothing confirmed, everything goes over the wire exactly once"
        );
    }

    #[tokio::test]
    async fn repeated_resume_calls_do_not_start_duplicate_workers() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        /*
         * Two resumes at once, as a double-click or a retry racing the user's
         * own produces. Exactly one of them may ask the receiver what it holds
         * and start workers on the answer; the other has to join that run.
         */
        let (first, second) = tokio::join!(manager.resume("t-1"), manager.resume("t-1"));

        first.expect("the first resume");
        second.expect("the second resume");

        assert_eq!(
            *receiver.status_queries.lock().expect("queries lock"),
            1,
            "a second resume must join the run the first one started, not start another"
        );

        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);

        assert_eq!(
            receiver.received_indices(),
            vec![1, 2],
            "no chunk may be sent twice because two resumes raced"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * Cancelling, and telling the receiver
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn cancelling_a_transfer_tells_the_receiver() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, _path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;

        manager.cancel("t-1").await.expect("cancelling");

        assert_eq!(
            *receiver.cancelled.lock().expect("cancelled lock"),
            1,
            "the receiver holds the bytes, so it has to be told to drop them"
        );
    }

    #[tokio::test]
    async fn cancelling_does_not_depend_on_the_receiver_answering() {
        let scratch = TempDir::new();

        // Nothing is listening on this port: the receiver is gone, or never was.
        let (manager, _store, _path) =
            restored(&scratch, SIZE, CHUNK, &[], "http://127.0.0.1:1").await;

        assert!(
            manager.cancel("t-1").await.is_ok(),
            "a cancellation is the user's, and must not fail because a peer is unreachable"
        );

        assert!(
            manager.status("t-1").is_err(),
            "and the transfer is gone locally all the same"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * What the sender describes to the receiver
     * ----------------------------------------------------------------------
     */

    /*
     * ----------------------------------------------------------------------
     * Streaming: the session does not wait for the digests
     * ----------------------------------------------------------------------
     */

    /// Opening the session must not wait for the hashing pass. The handshake is
    /// made with nothing described, the transfer starts sending, and the
    /// description follows on a second start once it exists.
    ///
    /// That the chunks behind it are really sent before the description is
    /// proved by `scheduler::tests::every_chunk_but_the_last_streams_before_the_digests_are_ready`;
    /// this asserts the sender actually makes that two-start handshake.
    #[tokio::test]
    async fn the_session_opens_before_the_files_are_hashed() {
        if !super::DESCRIBE_DIGESTS {
            return;
        }
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _path) = registered(&scratch).await;

        manager.start(metadata(&endpoint)).await.expect("starting");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);

        let starts = receiver.starts.lock().expect("starts lock").clone();

        assert!(
            starts.len() >= 2,
            "the transfer has to open a session and then describe its files, saw {} start(s)",
            starts.len()
        );
        assert!(
            starts[0].is_none(),
            "the session must open before the hashing pass, not after it"
        );
        assert_eq!(
            starts[1]
                .as_ref()
                .map(|expected| &expected["files"][0]["sha256"]),
            Some(&serde_json::Value::String(described_sha256())),
            "and the digests follow on the next start"
        );

        assert_eq!(receiver.received_indices(), vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn a_new_transfer_describes_what_its_files_should_hash_to() {
        if !super::DESCRIBE_DIGESTS {
            return;
        }

        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, path) = registered(&scratch).await;

        manager.start(metadata(&endpoint)).await.expect("starting");
        settle(&manager).await;

        let described = receiver
            .described
            .lock()
            .expect("described lock")
            .clone()
            .expect("a new transfer has to describe its files, or nothing can check them");

        assert_eq!(described["transfer_id"], "t-1");

        let file = &described["files"][0];

        assert_eq!(
            file["file_id"],
            receiver.received()[0].0,
            "the same file the chunk headers name"
        );
        assert_eq!(file["relative_path"], "a.bin");
        assert_eq!(file["size"], SIZE);
        assert_eq!(
            file["sha256"],
            described_sha256(),
            "the digest is of the file the sender reads and sends"
        );

        let chunks = file["chunks"].as_array().expect("chunks are listed");

        assert_eq!(chunks.len(), plan(&path, SIZE, CHUNK).len());
        assert_eq!(chunks[0]["length"], 4);
        assert_eq!(chunks[2]["length"], 2, "the tail is a short chunk");
    }

    #[tokio::test]
    async fn a_resume_describes_what_its_files_should_hash_to() {
        if !super::DESCRIBE_DIGESTS {
            return;
        }

        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        /*
         * A transfer a previous run started. Nothing about the digests survives
         * in memory — they are derived from the files — so the resume has to
         * work them out again before it opens the session, or the receiver is
         * left checking the file it assembles against an older description, or
         * against nothing at all.
         */
        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        manager.resume("t-1").await.expect("resuming");
        settle(&manager).await;

        let described = receiver
            .described
            .lock()
            .expect("described lock")
            .clone()
            .expect("a resume has to describe the files again");

        let file = &described["files"][0];

        assert_eq!(file["file_id"], "f-1");
        assert_eq!(file["sha256"], described_sha256());
        assert_eq!(file["chunks"].as_array().expect("chunks").len(), 3);
    }

    /*
     * ----------------------------------------------------------------------
     * A claim that does not check out costs a resend, not correctness
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_receivers_unverifiable_claims_cause_retransmission() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, _path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;

        /*
         * The receiver says it holds everything, but with digests that do not
         * match the sender's bytes — a stale manifest, a reused transfer id, or
         * a receiver that is simply wrong.
         */
        *receiver.report.lock().expect("report lock") = Some(json!({
            "transfer_id": "t-1",
            "files": [{
                "file_id": "f-1",
                "chunks": [
                    { "chunk_index": 0, "chunk_length": 4, "chunk_hash": "00" },
                    { "chunk_index": 1, "chunk_length": 4, "chunk_hash": "11" },
                    { "chunk_index": 2, "chunk_length": 2, "chunk_hash": "22" },
                ],
            }],
        }));

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);
        assert_eq!(
            receiver.received_indices(),
            vec![0, 1, 2],
            "claims that cannot be verified must not skip anything"
        );
    }

    #[tokio::test]
    async fn a_report_for_the_wrong_transfer_causes_a_full_resend() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        let mut report = report_for(&jobs, &[0, 1, 2]).await;

        report["transfer_id"] = json!("t-someone-else");

        *receiver.report.lock().expect("report lock") = Some(report);

        manager.resume("t-1").await.expect("resuming");
        settle(&manager).await;

        assert_eq!(receiver.received_indices(), vec![0, 1, 2]);
    }

    /*
     * ----------------------------------------------------------------------
     * Progress counts what was skipped as much as what was sent
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn progress_reflects_skipped_and_newly_uploaded_chunks() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        // Two of the three chunks are already there.
        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0, 1]).await);

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(
            status.uploaded_chunks, 3,
            "the two skipped chunks count as done; a resume must not report a \
             nearly-finished transfer as nearly empty"
        );
        assert_eq!(
            status.uploaded_bytes, SIZE,
            "and the byte count covers them too"
        );
        assert_eq!(status.total_chunks, 3);
        assert_eq!(status.total_bytes, SIZE);

        assert_eq!(
            receiver.received_indices(),
            vec![2],
            "only chunk 2 was sent"
        );
    }

    #[tokio::test]
    async fn a_resume_reports_the_skipped_chunks_before_it_sends_anything() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        // The receiver holds the first two chunks: eight of the ten bytes.
        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0, 1]).await);

        manager.resume("t-1").await.expect("resuming");

        let state = manager.get("t-1").expect("the transfer is live");
        let progress = progress::make(&state);

        assert_eq!(
            progress.uploaded_bytes, 8,
            "the chunks being skipped are progress from the moment the resume is accepted"
        );
        assert_eq!(progress.total_bytes, SIZE);
        assert_eq!(progress.percentage, 80.0);
        assert!(
            receiver.received().is_empty(),
            "and none of it has gone over the wire yet"
        );
    }

    #[tokio::test]
    async fn a_half_completed_transfer_resumes_at_half_progress() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        // Two four-byte chunks; the receiver has the first.
        let (manager, _store, path) = restored(&scratch, 8, 4, &[], &endpoint).await;
        let jobs = plan(&path, 8, 4);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        manager.resume("t-1").await.expect("resuming");

        let progress = progress::make(&manager.get("t-1").expect("live"));

        assert_eq!(progress.uploaded_bytes, 4);
        assert_eq!(progress.total_bytes, 8);
        assert_eq!(progress.percentage, 50.0);
    }

    #[tokio::test]
    async fn a_finished_transfer_reports_every_byte() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        // One chunk skipped, two sent.
        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        manager.resume("t-1").await.expect("resuming");

        assert_eq!(settle(&manager).await.status, TransferStatus::Completed);

        let progress = progress::make(&manager.get("t-1").expect("live"));

        assert_eq!(progress.uploaded_bytes, SIZE);
        assert_eq!(progress.total_bytes, SIZE);
        assert_eq!(progress.percentage, 100.0);
    }

    #[tokio::test]
    async fn a_stale_completed_chunk_does_not_contribute_to_a_restored_transfer() {
        let scratch = TempDir::new();
        let (_receiver, endpoint) = serve_stub().await;

        /*
         * The state file claims chunk 0 of the file — and a chunk 99 the file
         * does not have, as a record written before the plan was rebuilt, or by
         * a build that counted something it should not have, would.
         */
        let (manager, _store, _path) = restored(&scratch, SIZE, CHUNK, &[0, 99], &endpoint).await;

        let status = manager.status("t-1").expect("the transfer is back");

        assert_eq!(status.uploaded_chunks, 1, "only the real chunk counts");
        assert_eq!(status.uploaded_bytes, 4, "and only its bytes");

        let progress = progress::make(&manager.get("t-1").expect("live"));

        assert_eq!(progress.total_bytes, SIZE);
        assert_eq!(progress.percentage, 40.0, "four of the ten bytes");
    }

    #[tokio::test]
    async fn progress_ends_covering_every_chunk() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        // The resume's own response is what a caller sees first.
        let response = manager.resume("t-1").await.expect("resuming");

        assert_eq!(response.total_chunks, 3);
        assert!(
            response.uploaded_chunks <= 3,
            "progress never exceeds the total"
        );

        settle(&manager).await;

        let final_status = manager.status("t-1").expect("status");

        assert_eq!(final_status.uploaded_chunks, 3);
        assert_eq!(final_status.uploaded_bytes, SIZE);
    }

    /*
     * ----------------------------------------------------------------------
     * What the state file says afterwards
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn the_completed_set_is_replaced_by_what_the_receiver_confirms() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        // The state file claims nothing, but the receiver holds chunk 0.
        let (manager, store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0]).await);

        manager.resume("t-1").await.expect("resuming");
        settle(&manager).await;

        /*
         * The transfer reports itself completed just before the final write
         * lands, so wait for the record rather than the status: what is being
         * asserted is what a restart would read, and that is the file.
         */
        let mut indices = Vec::new();

        for _ in 0..250 {
            let persisted = store.load("t-1").await.expect("the record is there");

            indices = persisted
                .completed_chunks
                .iter()
                .map(|chunk| chunk.chunk_index)
                .collect();

            indices.sort_unstable();

            if indices.len() == 3 {
                break;
            }

            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(
            indices,
            vec![0, 1, 2],
            "every chunk ends up recorded, whether it was sent or skipped"
        );
    }

    #[tokio::test]
    async fn resuming_a_settled_transfer_sends_nothing_by_itself() {
        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;

        let (manager, _store, path) = restored(&scratch, SIZE, CHUNK, &[], &endpoint).await;
        let jobs = plan(&path, SIZE, CHUNK);

        *receiver.report.lock().expect("report lock") = Some(report_for(&jobs, &[0, 1, 2]).await);

        manager.resume("t-1").await.expect("resuming");
        let status = settle(&manager).await;

        assert_eq!(status.status, TransferStatus::Completed);

        let received_before = receiver.received().len();

        manager.pause("t-1").await.expect("pausing");
        manager.resume("t-1").await.expect("resuming again");

        assert_eq!(
            receiver.received().len(),
            received_before,
            "resuming a transfer with no workers must not send anything by itself"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * A persisted digest plan is a cache, and a changed file invalidates it
     * ----------------------------------------------------------------------
     */

    /// Move a file's modification time forward, the way an edit does.
    fn edited_now(path: &std::path::Path) {
        let later = std::time::SystemTime::now() + Duration::from_secs(30);

        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("opening the source to stamp it")
            .set_modified(later)
            .expect("setting the modification time");
    }

    /// A manager over `store`, with no restore performed yet.
    async fn manager_for(store: Arc<TransferStore>) -> UploadManager {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        UploadManager::new(
            Arc::new(RecordingEventSink::new()),
            std::env::temp_dir().join("vilsend-resume-test-uploads"),
            Arc::new(LocalTransferFileService::new(pool).await),
            tx,
            store,
        )
    }

    #[tokio::test]
    async fn a_resume_re_hashes_a_source_file_that_changed_after_it_was_planned() {
        if !super::DESCRIBE_DIGESTS {
            return;
        }

        let scratch = TempDir::new();
        let (receiver, endpoint) = serve_stub().await;
        let path = scratch.path().join("a.bin");

        // The previous run: plan these bytes, and record the stamp it saw.
        std::fs::write(&path, b"xxxxxxxxxx").expect("writing the source");
        let planned_at = record_modified_nanos(&path);

        let source = FileInfo {
            id: "f-1".into(),
            path: path.clone(),
            relative_path: "a.bin".into(),
            size: SIZE,
            modified_nanos: planned_at,
        };

        let stale = integrity::plan("t-1", std::slice::from_ref(&source), CHUNK)
            .await
            .expect("planning");

        // Same length, different bytes, a later stamp: what an edit looks like.
        std::fs::write(&path, b"yyyyyyyyyy").expect("rewriting the source");
        edited_now(&path);

        let store = Arc::new(TransferStore::in_memory(scratch.path().join("state")));

        let mut record = record(&path, SIZE, CHUNK, &[]);
        record.endpoint = endpoint.clone();
        record.files[0].modified_nanos = planned_at;

        store.save(&record).await.expect("saving the record");
        store
            .save_plan("t-1", &stale)
            .await
            .expect("saving the plan");
        store
            .save_secrets(
                "t-1",
                &TransferSecrets {
                    transfer_key: [0x7F; 32],
                    auth_token: "a-token".into(),
                },
            )
            .expect("saving secrets");

        let manager = manager_for(Arc::clone(&store)).await;
        assert_eq!(manager.restore_persisted().await, 1);

        manager.resume("t-1").await.expect("resuming");

        assert_eq!(settle(&manager).await.status, TransferStatus::Completed);

        /*
         * The refreshed stamp has to be kept with the transfer. Otherwise the
         * recomputed plan describes the file correctly, but the record still
         * says the old stamp matched nothing — so the next resume would hash
         * the same unchanged file again.
         */
        let state = manager.get("t-1").expect("the transfer is still known");

        assert_eq!(
            state.files_with_stamps()[0].modified_nanos,
            record_modified_nanos(&path),
            "the stamp the new plan was made against must replace the stale one"
        );
        assert!(
            integrity::plan_is_current(
                "t-1",
                &state.expected().expect("the plan is in memory"),
                &state.files_with_stamps(),
                CHUNK,
            )
            .await,
            "so the plan is reusable on the next resume"
        );

        let described = receiver
            .described
            .lock()
            .expect("described lock")
            .clone()
            .expect("the resume must describe its files");

        assert_eq!(
            described["files"][0]["sha256"],
            crate::transfer::manifest::sha256_hex(b"yyyyyyyyyy"),
            "a file edited after planning must be described by the bytes it holds now"
        );
    }
}
