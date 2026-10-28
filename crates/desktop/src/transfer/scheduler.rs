use crate::{
    models::chunk::ChunkJob,
    transfer::{errors::TransferError, session::ProtocolSession, state::UploadState, upload},
};

use std::time::Instant;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{sync::Semaphore, task::JoinSet, time::Duration};

/// Holds back each file's final chunk until the sender has worked out what the
/// transfer's files should hash to and told the receiver.
///
/// The receiver assembles a file the moment its last chunk lands, so that one
/// chunk is the only thing that must not arrive before the digests that check
/// it. Everything else streams while the hashing is still running.
///
/// A gate that is never released — or is failed — is how a transfer whose
/// digests could not be worked out stops without any file being completed.
#[derive(Default)]
pub struct PlanGate {
    released: AtomicBool,
    failed: AtomicBool,
}

impl PlanGate {
    /// The digests have been delivered; the final chunks may go.
    pub fn release(&self) {
        self.released.store(true, Ordering::Relaxed);
    }

    /// The digests could not be worked out or delivered. No file may complete.
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Relaxed);
    }

    pub fn is_released(&self) -> bool {
        self.released.load(Ordering::Relaxed)
    }

    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

/// Why a dispatched chunk did not land.
enum ChunkFailure {
    /// Nothing more can be done for this chunk.
    Fatal(String),
    /// The receiver refused the chunk and named the chunks it no longer holds.
    /// Those are resent; this is not a failure of the transfer.
    Missing { file_id: String, indices: Vec<u64> },
}

/// Called once per acknowledged chunk, with the job that was sent and how many
/// retries it took.
///
/// Takes the whole job rather than just its length so that the caller can record
/// *which* chunk landed, not only how much — a byte count cannot tell a resume
/// what is still missing.
pub type ChunkAcknowledged = dyn Fn(&ChunkJob, u32) + Send + Sync;

/// Send `jobs` with at most `concurrency` of them in flight.
///
/// Each chunk is dispatched as its own task rather than handed to a fixed pool
/// of workers, so the next chunk starts as soon as a slot frees — not when the
/// task that freed it finishes its bookkeeping. The semaphore is what bounds
/// the dispatch: without it, a 10 GiB file would spawn hundreds of thousands of
/// chunks, each holding its own chunk-sized buffer.
///
/// # Errors and cancellation
///
/// * A failed chunk stops new dispatches and lets the chunks already in flight
///   finish, then returns the first failure. Nothing that has completed is
///   undone; the transfer's own retry/resume path decides what happens next.
/// * A cancelled transfer stops new dispatches and waits for what is already
///   in flight. It returns `Ok`.
/// * A paused transfer parks each task before it reads its chunk, so a paused
///   transfer holds slots but sends nothing.
///
/// When `gate` is given, every chunk except the last of its file is dispatched
/// immediately; the final chunks wait for the gate, because they are what makes
/// a receiver assemble and check a file.
pub async fn run(
    jobs: Vec<ChunkJob>,
    concurrency: usize,
    state: Arc<UploadState>,
    session: Arc<dyn ProtocolSession>,
    on_success: Arc<ChunkAcknowledged>,
    gate: Option<Arc<PlanGate>>,
) -> std::result::Result<(), String> {
    /*
     * The transfer key is read once per run, and it is read *by the session*,
     * which is where it now lives.
     *
     * This used to be a `state.transfer_key()` call here, with the reasoning
     * that the key can change while workers run — a resume that re-establishes a
     * session after the receiver restarted adopts the fresh key it just derived
     * — and a worker encrypting half a transfer with one key and half with
     * another would produce a file the receiver cannot decrypt. The discipline
     * is unchanged and the reason it holds is now stronger: `open` sets the key
     * before this function is called and nothing writes it during the run, so
     * every chunk seals under one key by construction rather than by reading
     * early enough.
     */
    let slots = Arc::new(Semaphore::new(concurrency.max(1)));

    /*
     * One open handle per source file for the whole run — see
     * `upload::FileHandles`. Shared by the tasks, not owned by one.
     */
    let files = Arc::new(upload::FileHandles::default());

    /*
     * The last chunk of a file is what makes the receiver assemble and check it
     * against the digests, so with a gate those go last and wait for it. A
     * resume has its digests already and passes no gate, so its queue is sent in
     * the order it was built.
     */
    // Kept so a missing chunk named by the receiver can be looked up again.
    let all_jobs = jobs.clone();

    let (immediate, deferred): (Vec<ChunkJob>, Vec<ChunkJob>) = match gate.is_some() {
        true => jobs.into_iter().partition(|job| job.index + 1 < job.total),

        false => (jobs, Vec::new()),
    };

    let mut jobs = immediate.into_iter();

    let mut deferred: Option<std::vec::IntoIter<ChunkJob>> =
        (!deferred.is_empty()).then(|| deferred.into_iter());

    let mut in_flight: JoinSet<std::result::Result<(), ChunkFailure>> = JoinSet::new();

    let mut first_error: Option<String> = None;

    /*
     * Chunks the receiver said it no longer holds. They are dispatched as soon
     * as a slot frees rather than at the end of the queue, because the file is
     * waiting on them to assemble. A transfer may bounce a chunk back a bounded
     * number of times; past that the receiver is not progressing and the
     * transfer fails rather than looping.
     */
    let mut retry_queue: VecDeque<ChunkJob> = VecDeque::new();
    let mut requeues_left = all_jobs.len().saturating_mul(2).max(4);

    /*
     * The startup timeline: one line when the scheduler starts, one when the
     * first chunk is actually dispatched, both measured from the request that
     * started the transfer. The two waits a first chunk can lose time in are
     * named separately — queue wait is everything before the job is taken,
     * semaphore wait is the capacity check after it.
     */
    let run_started = Instant::now();
    let mut first_dispatch = true;

    tracing::info!(
        transfer_id = %state.transfer_id,
        startup_ms = state.startup_started.elapsed().as_millis() as u64,
        "startup: scheduler running"
    );

    /*
     * Rounds: one pass dispatches the queue and waits for what it started. If
     * the receiver then names chunks it no longer holds, another round runs
     * with just those; anything else ends the transfer.
     */
    loop {
        loop {
            /*
             * Collect whatever finished before launching anything else. A failure
             * must be seen before another chunk is dispatched, or one bad chunk
             * would let the queue drain past it.
             */
            while let Some(finished) = in_flight.try_join_next() {
                note_outcome(
                    finished,
                    &mut first_error,
                    &mut retry_queue,
                    &mut requeues_left,
                    &all_jobs,
                );
            }

            if first_error.is_some() || state.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            /*
             * Acquire a slot *before* taking the next job, so waiting for capacity
             * cannot grow an unbounded queue of already-spawned tasks. The permit is
             * moved into the task and dropped when it ends, which is what releases
             * the slot.
             */
            let Some(job) = next_job(
                &mut jobs,
                &mut deferred,
                &mut retry_queue,
                gate.as_deref(),
                &state,
                &mut first_error,
            )
            .await
            else {
                break;
            };

            let job_taken = Instant::now();

            let permit_wait_started = Instant::now();

            let permit = match slots.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => break,
            };

            let first_for_this_transfer = if first_dispatch {
                first_dispatch = false;

                tracing::info!(
                    transfer_id = %state.transfer_id,
                    chunk = job.index,
                    queue_wait_ms = job_taken.duration_since(run_started).as_millis() as u64,
                    semaphore_wait_ms = permit_wait_started.elapsed().as_millis() as u64,
                    startup_ms = state.startup_started.elapsed().as_millis() as u64,
                    "startup: first worker scheduled"
                );

                Some(state.startup_started)
            } else {
                None
            };

            let state = Arc::clone(&state);
            let session = Arc::clone(&session);
            let success = Arc::clone(&on_success);
            let files = Arc::clone(&files);

            in_flight.spawn(async move {
                let _permit = permit;

                while state.paused.load(std::sync::atomic::Ordering::Relaxed) {
                    if state.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        return Ok(());
                    }

                    tokio::time::sleep(Duration::from_millis(200)).await;
                }

                if state.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(());
                }

                /*
                 * The chunk's own clock: dispatch to acknowledgement, which is
                 * exactly the wait a user feels and exactly what the report's
                 * latency percentiles describe.
                 */
                let chunk_started = Instant::now();

                state.report.chunk_started(&job.file_id, job.length as u64);

                upload::upload_chunk(
                    session.as_ref(),
                    &job,
                    state.max_retries,
                    &files,
                    first_for_this_transfer,
                )
                .await
                .map(|(retries, timings)| {
                    state.report.chunk_finished(
                        &job.file_id,
                        chunk_started.elapsed(),
                        retries as u64,
                    );

                    state.report.chunk_phases(
                        timings.read,
                        timings.compress,
                        timings.encrypt,
                        timings.encrypt_cpu,
                        timings.http,
                    );

                    state
                        .report
                        .chunk_bytes(timings.plaintext_bytes, timings.body_bytes);

                    /*
                     * The receiver answers the last chunk of a file only after it
                     * has assembled, checked and published that file. So this
                     * acknowledgement is the confirmation the report needs — it
                     * is not inferred from a chunk count.
                     */
                    if job.index + 1 == job.total {
                        state.report.file_completed(&job.file_id);
                    }

                    success(&job, retries)
                })
                .map_err(|error| match error {
                    TransferError::ChunksMissing { missing } => {
                        /*
                         * The refused response was the acknowledgement of *this*
                         * chunk, so this chunk is unacknowledged even though its
                         * bytes were accepted: it goes back with the ones the
                         * receiver named, or the queue drains one ACK short.
                         */
                        let mut indices = missing;

                        if !indices.contains(&job.index) {
                            indices.push(job.index);
                        }

                        tracing::warn!(
                            transfer_id = %state.transfer_id,
                            chunk = job.index,
                            ?indices,
                            "receiver reported chunks it no longer holds; resending them"
                        );

                        ChunkFailure::Missing {
                            file_id: job.file_id.clone(),
                            indices,
                        }
                    }

                    other => {
                        state.report.chunk_failed(&job.file_id);

                        ChunkFailure::Fatal(other.to_string())
                    }
                })
            });
        }

        /*
         * Nothing new will be started. Let what is already in flight reach its own
         * end — cancelling those mid-request would leave the receiver holding a
         * chunk the sender will not count, which is exactly the state resume exists
         * to recover from but there is no reason to create it.
         */
        while let Some(finished) = in_flight.join_next().await {
            note_outcome(
                finished,
                &mut first_error,
                &mut retry_queue,
                &mut requeues_left,
                &all_jobs,
            );
        }

        if first_error.is_some()
            || state.cancelled.load(std::sync::atomic::Ordering::Relaxed)
            || retry_queue.is_empty()
        {
            break;
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The next chunk to dispatch: everything unblocked first, then the final
/// chunks once the digests are delivered.
///
/// Waiting here rather than in the dispatched task is deliberate: a task parked
/// on the gate would hold a concurrency slot, and a transfer of many small files
/// — every chunk of which is a file's last — would stall completely instead of
/// streaming what it can.
async fn next_job(
    jobs: &mut std::vec::IntoIter<ChunkJob>,
    deferred: &mut Option<std::vec::IntoIter<ChunkJob>>,
    retry_queue: &mut VecDeque<ChunkJob>,
    gate: Option<&PlanGate>,
    state: &UploadState,
    first_error: &mut Option<String>,
) -> Option<ChunkJob> {
    /*
     * A requeued chunk goes first: the receiver refused a chunk it no longer
     * holds, and the file cannot assemble until it is back. It bypasses the
     * plan gate deliberately — a refusal can only happen once the transfer is
     * already running, which is after the gate has been released.
     */
    if let Some(job) = retry_queue.pop_front() {
        return Some(job);
    }

    if let Some(job) = jobs.next() {
        return Some(job);
    }

    let rest = deferred.as_mut()?;

    if let Some(gate) = gate {
        while !gate.is_released() {
            if gate.has_failed() {
                if first_error.is_none() {
                    *first_error = Some(
                        "the transfer's digests could not be delivered, so no file could be \
                         completed"
                            .into(),
                    );
                }

                return None;
            }

            if state.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return None;
            }

            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    rest.next()
}

/// Fold one finished task into the run's outcome.
///
/// The first error wins: later failures are the consequences of the first (the
/// receiver refusing every chunk after one was dropped, for instance) and
/// reporting them instead would hide the cause.
fn note_outcome(
    finished: std::result::Result<std::result::Result<(), ChunkFailure>, tokio::task::JoinError>,
    first_error: &mut Option<String>,
    retry_queue: &mut VecDeque<ChunkJob>,
    requeues_left: &mut usize,
    all_jobs: &[ChunkJob],
) {
    let error = match finished {
        Ok(Ok(())) => return,

        Ok(Err(ChunkFailure::Fatal(error))) => error,

        Ok(Err(ChunkFailure::Missing { file_id, indices })) => {
            for index in indices {
                let Some(job) = all_jobs
                    .iter()
                    .find(|job| job.file_id == file_id && job.index == index)
                else {
                    continue;
                };

                if *requeues_left == 0 {
                    if first_error.is_none() {
                        *first_error =
                            Some("the receiver kept reporting chunks it no longer holds".into());
                    }

                    return;
                }

                *requeues_left -= 1;
                retry_queue.push_back(job.clone());
            }

            return;
        }

        Err(join) => join.to_string(),
    };

    if first_error.is_none() {
        *first_error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::transfer::{ConnectionType, TransferStatus},
        transfer::{
            errors::Result,
            session::{OpenRequest, OpenedSession},
            state::TransferCredentials,
        },
    };
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// A session that records how many chunks are being "sent" at once.
    ///
    /// It does no I/O, so the only thing under test is the scheduler's dispatch
    /// — which is exactly the property a worker pool got right by construction
    /// and a task-per-chunk loop has to get right on purpose.
    struct SlowSession {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        sent: AtomicU64,
    }

    #[async_trait::async_trait]
    impl ProtocolSession for SlowSession {
        async fn resume_state(
            &self,
            _transfer_id: &str,
        ) -> Result<Option<crate::transfer::http_client::TransferResumeResponse>> {
            Ok(None)
        }

        async fn open(&self, _request: OpenRequest<'_>) -> Result<OpenedSession> {
            unimplemented!("the scheduler never opens a session")
        }

        async fn send_chunk(
            &self,
            _job: &ChunkJob,
            _plaintext: Vec<u8>,
        ) -> Result<crate::transfer::session::ChunkSendTimings> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;

            self.peak.fetch_max(now, Ordering::SeqCst);

            tokio::time::sleep(Duration::from_millis(20)).await;

            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.sent.fetch_add(1, Ordering::SeqCst);

            Ok(crate::transfer::session::ChunkSendTimings::default())
        }

        async fn cancel(&self, _transfer_id: &str) -> Result<()> {
            Ok(())
        }
    }

    fn state(concurrency: usize) -> Arc<UploadState> {
        Arc::new(UploadState {
            transfer_id: "t-scheduler".into(),
            credentials: std::sync::Mutex::new(TransferCredentials::default()),
            endpoint: "receiver.invalid".into(),
            network_type: ConnectionType::Lan,
            files: Vec::new(),
            chunk_size: 1,
            concurrency,
            total_bytes: 0,
            uploaded_bytes: AtomicU64::new(0),
            total_chunks: 0,
            uploaded_chunks: AtomicU64::new(0),
            paused: false.into(),
            cancelled: false.into(),
            retry_count: 0.into(),
            max_retries: 1,
            status: std::sync::Mutex::new(TransferStatus::Queued),
            started_at: std::time::Instant::now(),
            startup_started: std::time::Instant::now(),
            completed: std::sync::Mutex::new(std::collections::HashSet::new()),
            running: false.into(),
            resume_lock: tokio::sync::Mutex::new(()),
            expected: std::sync::Mutex::new(None),
            plan_stamps: std::sync::Mutex::new(std::collections::HashMap::new()),
            report: crate::transfer::report::Telemetry::new(crate::transfer::report::Role::Sender),
            phases: std::sync::Mutex::new(crate::transfer::report::PhaseTimings::default()),
        })
    }

    fn jobs(count: u32, source: &std::path::Path) -> Vec<ChunkJob> {
        (0..count)
            .map(|index| ChunkJob {
                transfer_id: "t-scheduler".into(),
                file_id: "f".into(),
                index: index as u64,
                total: count as u64,
                relative_path: "f.bin".into(),
                path: source.to_path_buf(),
                offset: index as u64,
                length: 1,
            })
            .collect()
    }

    /// The dispatch is bounded, and it is actually concurrent: a loop that sent
    /// one chunk at a time would pass every correctness test and be eight times
    /// slower.
    #[tokio::test]
    async fn chunks_are_dispatched_concurrently_within_the_limit() {
        let directory = std::env::temp_dir().join(format!(
            "vilsend-scheduler-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is after 1970")
                .as_nanos()
        ));

        std::fs::create_dir_all(&directory).expect("creating the scratch directory");

        let source = directory.join("f.bin");

        std::fs::write(&source, vec![7u8; 24]).expect("writing the source");

        let session = Arc::new(SlowSession {
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            sent: AtomicU64::new(0),
        });

        let acknowledged = Arc::new(AtomicU64::new(0));

        let on_success: Arc<ChunkAcknowledged> = {
            let acknowledged = Arc::clone(&acknowledged);

            Arc::new(move |_job, _retries| {
                acknowledged.fetch_add(1, Ordering::SeqCst);
            })
        };

        let result = run(
            jobs(24, &source),
            3,
            state(3),
            session.clone(),
            on_success,
            None,
        )
        .await;

        let _ = std::fs::remove_dir_all(&directory);

        result.expect("every chunk should be sent");

        assert_eq!(session.sent.load(Ordering::SeqCst), 24);
        assert_eq!(acknowledged.load(Ordering::SeqCst), 24);

        let peak = session.peak.load(Ordering::SeqCst);

        assert!(peak <= 3, "the semaphore let {peak} chunks run at once");
        assert!(peak >= 2, "only {peak} chunk ran at once: not concurrent");
    }

    /// A session that records every chunk index it is sent, in arrival order.
    struct RecordingSession {
        sent: std::sync::Mutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl ProtocolSession for RecordingSession {
        async fn resume_state(
            &self,
            _transfer_id: &str,
        ) -> Result<Option<crate::transfer::http_client::TransferResumeResponse>> {
            Ok(None)
        }

        async fn open(&self, _request: OpenRequest<'_>) -> Result<OpenedSession> {
            unimplemented!("the scheduler never opens a session")
        }

        async fn send_chunk(
            &self,
            job: &ChunkJob,
            _plaintext: Vec<u8>,
        ) -> Result<crate::transfer::session::ChunkSendTimings> {
            self.sent.lock().expect("sent lock").push(job.index);

            Ok(crate::transfer::session::ChunkSendTimings::default())
        }

        async fn cancel(&self, _transfer_id: &str) -> Result<()> {
            Ok(())
        }
    }

    impl RecordingSession {
        fn indices(&self) -> Vec<u64> {
            let mut indices = self.sent.lock().expect("sent lock").clone();

            indices.sort_unstable();

            indices
        }
    }

    fn scratch_file(name: &str, bytes: usize) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "vilsend-scheduler-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is after 1970")
                .as_nanos()
        ));

        std::fs::create_dir_all(&directory).expect("creating the scratch directory");

        let source = directory.join("f.bin");

        std::fs::write(&source, vec![7u8; bytes]).expect("writing the source");

        source
    }

    /// A session that refuses one chunk once, naming it as missing.
    struct BouncesOnceSession {
        bounced: std::sync::Mutex<bool>,
        sent: std::sync::Mutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl ProtocolSession for BouncesOnceSession {
        async fn resume_state(
            &self,
            _transfer_id: &str,
        ) -> Result<Option<crate::transfer::http_client::TransferResumeResponse>> {
            Ok(None)
        }

        async fn open(&self, _request: OpenRequest<'_>) -> Result<OpenedSession> {
            unimplemented!("the scheduler never opens a session")
        }

        async fn send_chunk(
            &self,
            job: &ChunkJob,
            _plaintext: Vec<u8>,
        ) -> Result<crate::transfer::session::ChunkSendTimings> {
            self.sent.lock().expect("sent lock").push(job.index);

            let mut bounced = self.bounced.lock().expect("bounced lock");

            if job.index == 1 && !*bounced {
                *bounced = true;

                return Err(crate::transfer::errors::TransferError::ChunksMissing {
                    missing: vec![1],
                });
            }

            Ok(crate::transfer::session::ChunkSendTimings::default())
        }

        async fn cancel(&self, _transfer_id: &str) -> Result<()> {
            Ok(())
        }
    }

    /// A receiver that says "chunk 1 is gone" gets chunk 1 again, and only it,
    /// and the transfer still succeeds.
    #[tokio::test]
    async fn a_chunk_the_receiver_reports_missing_is_resent_and_the_run_continues() {
        let source = scratch_file("missing", 4);

        let session = Arc::new(BouncesOnceSession {
            bounced: std::sync::Mutex::new(false),
            sent: std::sync::Mutex::new(Vec::new()),
        });

        let acknowledged = Arc::new(AtomicU64::new(0));

        let on_success: Arc<ChunkAcknowledged> = {
            let acknowledged = Arc::clone(&acknowledged);

            Arc::new(move |_job, _retries| {
                acknowledged.fetch_add(1, Ordering::SeqCst);
            })
        };

        let result = run(
            jobs(4, &source),
            4,
            state(4),
            session.clone(),
            on_success,
            None,
        )
        .await;

        result.expect("the transfer continues after a missing chunk");

        let sent = session.sent.lock().expect("sent lock").clone();

        assert_eq!(
            sent.iter().filter(|index| **index == 1).count(),
            2,
            "the refused chunk is sent again: {sent:?}"
        );
        assert_eq!(
            sent.iter().filter(|index| **index != 1).count(),
            3,
            "every other chunk is sent once: {sent:?}"
        );
        assert_eq!(acknowledged.load(Ordering::SeqCst), 4);
    }

    /// The whole point of the gate: a file's chunks go out while the sender is
    /// still hashing, and only the final chunk — the one that makes the receiver
    /// assemble and check the file — waits for the digests.
    #[tokio::test]
    async fn every_chunk_but_the_last_streams_before_the_digests_are_ready() {
        let source = scratch_file("gate", 4);

        let session = Arc::new(RecordingSession {
            sent: std::sync::Mutex::new(Vec::new()),
        });

        let gate = Arc::new(PlanGate::default());
        let acknowledged: Arc<ChunkAcknowledged> = Arc::new(|_job, _retries| {});

        let state = state(4);
        let probe = Arc::clone(&state);

        let sender = tokio::spawn(run(
            jobs(4, &source),
            4,
            state,
            session.clone(),
            acknowledged,
            Some(Arc::clone(&gate)),
        ));

        tokio::time::timeout(Duration::from_secs(5), async {
            while session.sent.lock().expect("sent lock").len() < 3 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("three chunks should be sent without the digests");

        assert!(
            !gate.is_released(),
            "the test has not released the gate yet"
        );
        assert_eq!(
            session.indices(),
            vec![0, 1, 2],
            "the final chunk must not be sent before the digests"
        );

        gate.release();

        sender
            .await
            .expect("the scheduler task")
            .expect("every chunk should be sent");

        assert_eq!(session.indices(), vec![0, 1, 2, 3]);

        /*
         * The final chunk's acknowledgement is the receiver saying it published
         * the file — so the report is allowed to call it complete, and must.
         */
        let report = probe.report.snapshot(&Default::default());

        assert!(
            report.files.iter().any(|file| file.completed),
            "the file whose last chunk was acknowledged has to be marked complete"
        );
    }

    /// A transfer whose digests could not be worked out fails before any file is
    /// completed — the final chunks are never sent, so no receiver can assemble
    /// a file it cannot check.
    #[tokio::test]
    async fn a_failed_plan_sends_no_final_chunk_and_reports_the_failure() {
        let source = scratch_file("gate-failed", 4);

        let session = Arc::new(RecordingSession {
            sent: std::sync::Mutex::new(Vec::new()),
        });

        let gate = Arc::new(PlanGate::default());
        let acknowledged: Arc<ChunkAcknowledged> = Arc::new(|_job, _retries| {});

        let state = state(4);
        let probe = Arc::clone(&state);

        let sender = tokio::spawn(run(
            jobs(4, &source),
            4,
            state,
            session.clone(),
            acknowledged,
            Some(Arc::clone(&gate)),
        ));

        tokio::time::timeout(Duration::from_secs(5), async {
            while session.sent.lock().expect("sent lock").len() < 3 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("three chunks should be sent without the digests");

        gate.fail();

        let result = sender.await.expect("the scheduler task");

        assert!(result.is_err(), "a failed plan has to fail the transfer");
        assert_eq!(
            session.indices(),
            vec![0, 1, 2],
            "the final chunk must never be sent on a failed plan"
        );

        let report = probe.report.snapshot(&Default::default());

        assert!(
            report.files.iter().all(|file| !file.completed),
            "no file may be reported complete when its last chunk was never sent"
        );
    }
}
