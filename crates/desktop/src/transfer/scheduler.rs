use crate::{
    models::chunk::ChunkJob,
    transfer::{session::ProtocolSession, state::UploadState, upload},
};

use std::sync::Arc;
use tokio::{sync::Semaphore, task::JoinSet, time::Duration};

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
pub async fn run(
    jobs: Vec<ChunkJob>,
    concurrency: usize,
    state: Arc<UploadState>,
    session: Arc<dyn ProtocolSession>,
    on_success: Arc<ChunkAcknowledged>,
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

    let mut jobs = jobs.into_iter();

    let mut in_flight: JoinSet<std::result::Result<(), String>> = JoinSet::new();

    let mut first_error: Option<String> = None;

    loop {
        /*
         * Collect whatever finished before launching anything else. A failure
         * must be seen before another chunk is dispatched, or one bad chunk
         * would let the queue drain past it.
         */
        while let Some(finished) = in_flight.try_join_next() {
            note_outcome(finished, &mut first_error);
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
        let permit = match slots.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => break,
        };

        let Some(job) = jobs.next() else {
            break;
        };

        let state = Arc::clone(&state);
        let session = Arc::clone(&session);
        let success = Arc::clone(&on_success);

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

            upload::upload_chunk(session.as_ref(), &job, state.max_retries)
                .await
                .map(|retries| success(&job, retries))
                .map_err(|error| error.to_string())
        });
    }

    /*
     * Nothing new will be started. Let what is already in flight reach its own
     * end — cancelling those mid-request would leave the receiver holding a
     * chunk the sender will not count, which is exactly the state resume exists
     * to recover from but there is no reason to create it.
     */
    while let Some(finished) = in_flight.join_next().await {
        note_outcome(finished, &mut first_error);
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Fold one finished task into the run's outcome.
///
/// The first error wins: later failures are the consequences of the first (the
/// receiver refusing every chunk after one was dropped, for instance) and
/// reporting them instead would hide the cause.
fn note_outcome(
    finished: std::result::Result<std::result::Result<(), String>, tokio::task::JoinError>,
    first_error: &mut Option<String>,
) {
    let error = match finished {
        Ok(Ok(())) => return,
        Ok(Err(error)) => error,
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

        async fn send_chunk(&self, _job: &ChunkJob, _plaintext: Vec<u8>) -> Result<()> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;

            self.peak.fetch_max(now, Ordering::SeqCst);

            tokio::time::sleep(Duration::from_millis(20)).await;

            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.sent.fetch_add(1, Ordering::SeqCst);

            Ok(())
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
            completed: std::sync::Mutex::new(std::collections::HashSet::new()),
            running: false.into(),
            resume_lock: tokio::sync::Mutex::new(()),
            expected: std::sync::Mutex::new(None),
            plan_stamps: std::sync::Mutex::new(std::collections::HashMap::new()),
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

        let result = run(jobs(24, &source), 3, state(3), session.clone(), on_success).await;

        let _ = std::fs::remove_dir_all(&directory);

        result.expect("every chunk should be sent");

        assert_eq!(session.sent.load(Ordering::SeqCst), 24);
        assert_eq!(acknowledged.load(Ordering::SeqCst), 24);

        let peak = session.peak.load(Ordering::SeqCst);

        assert!(peak <= 3, "the semaphore let {peak} chunks run at once");
        assert!(peak >= 2, "only {peak} chunk ran at once: not concurrent");
    }
}
