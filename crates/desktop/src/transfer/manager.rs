use crate::models::chunk::ChunkJob;
use crate::services::local_transfer_file_service::LocalTransferFileService;
use crate::transfer::crypto;
use crate::transfer::http_client::{HttpClient, SessionStart};
use crate::transfer::integrity;
use crate::transfer::persist::{PersistedTransfer, TransferSecrets, TransferStore};
use crate::transfer::resume;
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

/// How long a cancellation waits for the receiver to acknowledge it.
///
/// Telling the receiver is best-effort: it is worth a moment, because it is what
/// frees the receiver's disk now rather than at the next sweep, and it is not
/// worth waiting on a peer that is gone.
const RECEIVER_NOTICE_TIMEOUT: Duration = Duration::from_secs(5);

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
            let mut transfers = self.transfers.lock().expect("transfer lock poisoned");

            if transfers.contains_key(&record.transfer_id) {
                continue;
            }

            let status = record.resumable_status();

            tracing::info!(
                transfer_id = %record.transfer_id,
                completed_chunks = record.completed_chunks.len(),
                total_chunks = record.total_chunks,
                ?status,
                "restored a persisted transfer"
            );

            transfers.insert(
                record.transfer_id.clone(),
                Arc::new(restored_state(&record)),
            );

            restored += 1;
        }

        restored
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

        let files = scanner::scan(&paths).await?;
        let chunk_size = metadata
            .chunk_size
            .unwrap_or(constants::DEFAULT_CHUNK_SIZE)
            .max(1);
        let jobs = chunker::jobs(&metadata.transfer_id, &files, chunk_size);
        let client = HttpClient::new();

        let receiver_public_key = client
            .get_public_key(&metadata.endpoint, &metadata.auth_token)
            .await?;

        let receiver_public =
            crypto::decode_public_key(&receiver_public_key).map_err(TransferError::Crypto)?;

        let ephemeral = crypto::generate_ephemeral_keypair();

        let shared = crypto::derive_shared_secret(&ephemeral.private_key, &receiver_public);

        let transfer_key = crypto::derive_transfer_key(&shared).map_err(TransferError::Invalid)?;

        let hash = Sha256::digest(transfer_key);

        tracing::info!(
            transfer_key_hash=%hex::encode(hash),
            "sender transfer key fingerprint"
        );

        let total_bytes = files.iter().map(|f| f.size).sum();

        /*
         * What each file hashes to, read from the files themselves before the
         * first chunk moves. The receiver is handed these with the session so
         * that it can tell the file it assembles from one that merely looks
         * assembled — it has no other expectation it did not write itself.
         */
        let expected = integrity::plan(&metadata.transfer_id, &files, chunk_size).await?;

        client
            .start_transfer(
                &metadata.endpoint,
                &metadata.auth_token,
                &metadata.transfer_id,
                &crypto::encode_public_key(&ephemeral.public_key),
                total_bytes,
                Some(&expected),
            )
            .await?;
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
            completed: Mutex::new(std::collections::HashSet::new()),
            running: false.into(),
            resume_lock: tokio::sync::Mutex::new(()),
            expected: Mutex::new(Some(expected)),
        });
        let response = state.snapshot();
        self.transfers
            .lock()
            .expect("transfer lock poisoned")
            .insert(metadata.transfer_id.clone(), state.clone());

        /*
         * Persist before any chunk moves. The plan and the secrets are written
         * now so that a crash during the first chunk still leaves a transfer
         * that can be described and continued, rather than one nobody knows the
         * shape of.
         */
        self.store_secrets(&state);
        self.persist(&state).await;

        self.spawn_scheduler(state, jobs);

        Ok(response)
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
    fn spawn_scheduler(&self, state: Arc<UploadState>, jobs: Vec<ChunkJob>) {
        let manager = self.clone();
        let concurrency = state.concurrency;
        let token = state.auth_token();

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
            let result = scheduler::run(jobs, concurrency, state.clone(), token, on_success).await;

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

                    TransferEvent::Completed(state.transfer_id.clone())
                }
                Ok(()) => manager.fail_transfer(
                    &state,
                    format!(
                        "the send queue drained with only {} of {} chunks acknowledged",
                        state.uploaded_chunks.load(Ordering::Relaxed),
                        state.planned_chunks()
                    ),
                ),
                Err(error) => manager.fail_transfer(&state, error),
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
    fn fail_transfer(&self, state: &Arc<UploadState>, error: String) -> TransferEvent {
        *state.status.lock().expect("status lock poisoned") = TransferStatus::Failed;
        tracing::error!(transfer_id=%state.transfer_id,error=%error,"transfer failed");
        self.events.emit(DomainEvent::TransferFailed {
            progress: progress::make(state),
        });

        TransferEvent::Failed(state.transfer_id.clone(), error)
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

        let client = HttpClient::new();

        /*
         * What does the receiver already have? A receiver that has never heard
         * of this transfer, or that predates the endpoint, answers 404 and is
         * read as holding nothing — the safe interpretation, since it costs a
         * resend rather than a skip.
         */
        let report = client
            .transfer_status(&state.endpoint, &token, &transfer_id)
            .await?;

        let confirmation = resume::confirmed_chunks(report.as_ref(), &transfer_id, &jobs).await;

        if !confirmation.is_trusted() {
            tracing::warn!(
                transfer_id = %transfer_id,
                rejected = confirmation.rejected.len(),
                "receiver reported chunks that do not match local metadata; \
                 those chunks will be sent again rather than skipped"
            );
        }

        /*
         * What the files hash to, if this session does not already know. A
         * transfer a previous run started carries none in memory — the digests
         * are derived from the files rather than written to the state file — so
         * they are worked out here, from the files as they stand now, and
         * described to the receiver along with the new session. A file that
         * changed while the transfer was stopped is a file whose chunks no
         * longer match what was sent before, and the receiver is told the new
         * truth rather than the old one.
         */
        let expected = match state.expected() {
            Some(expected) => expected,

            None => {
                let expected =
                    integrity::plan(&transfer_id, &state.files, state.chunk_size).await?;

                state.set_expected(expected.clone());

                expected
            }
        };

        /*
         * Re-establish the session before sending. A receiver that restarted has
         * forgotten the transfer and needs the handshake again; one that never
         * noticed our absence already has it live and refuses with `CONFLICT`,
         * keeping the key it had.
         */
        let session = self
            .reopen_session(state, &client, &token, &transfer_id, Some(&expected))
            .await?;

        if session == SessionStart::Established {
            // The receiver derived a fresh key from the ephemeral key just sent,
            // so the one in memory is no longer the one it will decrypt with.
            tracing::info!(
                transfer_id = %transfer_id,
                "receiver re-established the session; continuing with a fresh transfer key"
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

            tracing::info!(
                transfer_id = %transfer_id,
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

        self.spawn_scheduler(Arc::clone(state), missing);

        // Taken after the workers are up, so the status the caller is handed is
        // the one the transfer is actually in.
        Ok(state.snapshot())
    }

    /// Open, or re-open, the receiver's session for a transfer, and make sure
    /// the key in memory is the one the receiver will decrypt with.
    async fn reopen_session(
        &self,
        state: &Arc<UploadState>,
        client: &HttpClient,
        token: &str,
        transfer_id: &str,
        expected: Option<&integrity::ExpectedTransfer>,
    ) -> Result<SessionStart> {
        let ephemeral = crypto::generate_ephemeral_keypair();

        let outcome = client
            .start_transfer(
                &state.endpoint,
                token,
                transfer_id,
                &crypto::encode_public_key(&ephemeral.public_key),
                state.total_bytes,
                expected,
            )
            .await?;

        if outcome == SessionStart::Established {
            /*
             * The receiver built its key from the ephemeral public key we just
             * sent, so derive the matching one and adopt it. The chunks it
             * already holds are unaffected: it stores them decrypted.
             */
            let receiver_public_key = client.get_public_key(&state.endpoint, token).await?;

            let receiver_public =
                crypto::decode_public_key(&receiver_public_key).map_err(TransferError::Crypto)?;

            let shared = crypto::derive_shared_secret(&ephemeral.private_key, &receiver_public);

            let transfer_key =
                crypto::derive_transfer_key(&shared).map_err(TransferError::Invalid)?;

            state.set_credentials(TransferCredentials {
                transfer_key,
                auth_token: token.to_string(),
            });

            self.store_secrets(state);
        }

        Ok(outcome)
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

        if let Err(error) = self.store.save(&record).await {
            tracing::warn!(
                transfer_id = %state.transfer_id,
                %error,
                "could not persist transfer state"
            );
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
        let client = HttpClient::new();

        let notice = tokio::time::timeout(
            RECEIVER_NOTICE_TIMEOUT,
            client.cancel_transfer(&s.endpoint, &s.auth_token(), id),
        )
        .await;

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
    fn get(&self, id: &str) -> Result<Arc<UploadState>> {
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
fn restored_state(record: &PersistedTransfer) -> UploadState {
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
        completed: Mutex::new(std::collections::HashSet::new()),
        // Nothing is running after a restart.
        running: false.into(),
        resume_lock: tokio::sync::Mutex::new(()),
        // Derived from the files rather than stored, so it is worked out when
        // the transfer is next continued — by which time the files, not the
        // record, are the authority on what is being sent.
        expected: Mutex::new(None),
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
            }],
            total_bytes: 4096,
            total_chunks: 4,
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

        let manager = build_manager(Arc::clone(&store)).await;
        manager.restore_persisted().await;

        manager.cancel("t-1").await.expect("cancelling");

        assert!(
            !state_dir.join("t-1.json").exists(),
            "a cancelled transfer is not one anybody wants continued"
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
        if let Ok(request) = serde_json::from_slice::<Value>(&body) {
            if let Some(described) = request.get("expected") {
                *receiver.described.lock().expect("described lock") = Some(described.clone());
            }
        }

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

        *receiver.public_key.lock().expect("key lock") =
            crypto::encode_public_key(&crypto::generate_ephemeral_keypair().public_key);

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
            }],
            chunk_size,
        )
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
            }],
            total_bytes: size,
            total_chunks,
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

    #[tokio::test]
    async fn a_new_transfer_describes_what_its_files_should_hash_to() {
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
}
