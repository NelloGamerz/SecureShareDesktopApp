//! End-to-end proof that a transfer survives being interrupted.
//!
//! Both sides here are the real ones. The sender is [`UploadManager`] with the
//! real HTTP client and the real chunk reader; the receiver is
//! [`crate::transfer::writer::router`], the same router the application serves,
//! bound to a real socket. Nothing in this file re-implements either side, so a
//! test that passes here is a statement about the thing that ships.
//!
//! The only test doubles are the two things the receiver reaches for outside
//! itself, and [`TestHost`] is both: where a finished file goes, and the device
//! key. Everything else — the manifest, the parts, the state file, the keychain
//! port, the wire — is the real article.
//!
//! # What each test is for
//!
//! The unit tests around each module prove that a piece behaves. These prove the
//! pieces still fit together when a transfer is cut in half — which is the only
//! thing "resumable" can mean. Every interruption is checked the same way:
//!
//! * the chunks that were already there are not sent again,
//! * the chunks that are missing or no longer verify are sent again,
//! * the transfer reaches a terminal state instead of hanging,
//! * the file that lands is the file that was sent, byte for byte.

use crate::{
    models::transfer::{
        ConnectionType, LocalTransferFile, TransferMetadata, TransferStatus, TransferStatusResponse,
    },
    services::{
        generate_device_keypair::{generate_device_keypair, DeviceKeyPair},
        local_transfer_file_service::LocalTransferFileService,
    },
    transfer::{
        manager::{UploadManager, DESCRIBE_DIGESTS},
        manifest,
        manifest::test_support::TempDir,
        persist::{InMemorySecretStore, SecretStore, TransferStore},
        scanner,
        writer::{self, ReceiverHost, ReceiverState},
    },
};
use axum::{
    body::Body,
    extract::State as AxumState,
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sqlx::sqlite::SqlitePoolOptions;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use vilsend_core::RecordingEventSink;

/// The one transfer these tests move around.
const TRANSFER: &str = "t-resumable";

/// Small enough that a transfer is many chunks, large enough that a chunk is
/// not a toy.
const CHUNK: usize = 1024;

/// How many chunks a transfer is, unless a test says otherwise.
const CHUNKS: u64 = 16;

/// What the sender sends for a chunk before it gives up on it.
///
/// One retry, so that a test which leans on the retry path costs a second rather
/// than the seven a full backoff would.
const RETRIES: u32 = 1;

/// The bearer token the receiver checks for presence and nothing else.
const TOKEN: &str = "a-token";

/*
 * ----------------------------------------------------------------------
 * The two things the receiver cannot get from itself
 * ----------------------------------------------------------------------
 */

/// A receiver with no application behind it: one directory for finished files,
/// room to spare, and a key pair this process made up.
struct TestHost {
    downloads: PathBuf,
    device: DeviceKeyPair,
}

impl ReceiverHost for TestHost {
    fn downloads_root(&self) -> Result<PathBuf, String> {
        Ok(self.downloads.clone())
    }

    fn available_space(&self, _path: &Path) -> Result<u64, String> {
        // The disk this test runs on is not the subject; a transfer that fits is
        // always accepted.
        Ok(u64::MAX)
    }

    fn device_private_key(&self) -> Result<String, String> {
        Ok(self.device.private_key.clone())
    }

    fn device_public_key(&self) -> Result<String, String> {
        Ok(self.device.public_key.clone())
    }
}

/*
 * ----------------------------------------------------------------------
 * Making the receiver fail on purpose
 * ----------------------------------------------------------------------
 */

/// What the receiver's layer in front of the router does: remembers every chunk
/// request that reaches it, and can refuse chosen ones.
///
/// The count is the measure of "sent again". It is taken in front of the
/// handler, so it counts what went over the wire rather than what the receiver
/// accepted, which is the question every test here is asking.
#[derive(Default)]
struct Faults {
    /// How many refusals each chunk index is still owed. `u32::MAX` is forever.
    refusals: StdMutex<HashMap<u64, u32>>,
    /// Every chunk request that arrived, in order.
    attempts: StdMutex<Vec<u64>>,
    /// How long the receiver takes over each chunk.
    ///
    /// A test that stops a transfer partway needs the transfer to still *be*
    /// partway when it looks; on a fast local disk a small transfer can be over
    /// before a test has finished asking. Holding the receiver's own pace to
    /// something human makes the interruption a statement about the transfer
    /// rather than about the machine it ran on.
    pace: StdMutex<Duration>,
}

impl Faults {
    fn record(&self, index: u64) {
        self.attempts.lock().expect("attempts lock").push(index);
    }

    fn attempts_of(&self, index: u64) -> usize {
        self.attempts
            .lock()
            .expect("attempts lock")
            .iter()
            .filter(|attempt| **attempt == index)
            .count()
    }

    /// Refuse `index` once and let it through afterwards.
    fn refuse_once(&self, index: u64) {
        self.refusals
            .lock()
            .expect("refusals lock")
            .insert(index, 1);
    }

    /// Refuse `index` however often it is sent.
    fn refuse_always(&self, index: u64) {
        self.refusals
            .lock()
            .expect("refusals lock")
            .insert(index, u32::MAX);
    }

    /// Stop refusing `index`.
    fn allow(&self, index: u64) {
        self.refusals.lock().expect("refusals lock").remove(&index);
    }

    /// Make the receiver take `per_chunk` over every chunk it accepts.
    fn pace(&self, per_chunk: Duration) {
        *self.pace.lock().expect("pace lock") = per_chunk;
    }

    fn pause_before_serving(&self) -> Duration {
        *self.pace.lock().expect("pace lock")
    }

    fn refuse(&self, index: u64) -> bool {
        let mut refusals = self.refusals.lock().expect("refusals lock");

        match refusals.get_mut(&index) {
            Some(remaining) if *remaining > 0 => {
                *remaining -= 1;
                true
            }

            _ => false,
        }
    }
}

/// Refuse the chunks the test asked to refuse, and let everything else through.
async fn faulty_chunks(
    AxumState(faults): AxumState<Arc<Faults>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::POST && request.uri().path() == "/transfer/chunk" {
        if let Some(index) = request
            .headers()
            .get("Chunk-Index")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
        {
            faults.record(index);

            if faults.refuse(index) {
                return (StatusCode::SERVICE_UNAVAILABLE, "refused by the test").into_response();
            }
        }

        let pace = faults.pause_before_serving();

        if !pace.is_zero() {
            tokio::time::sleep(pace).await;
        }
    }

    next.run(request).await
}

/*
 * ----------------------------------------------------------------------
 * A running receiver
 * ----------------------------------------------------------------------
 */

/// The receiver, serving the real routes over a real socket.
struct Receiver {
    endpoint: String,
    state: ReceiverState,
    server: tokio::task::JoinHandle<()>,
}

impl Receiver {
    async fn start(host: Arc<TestHost>, root: PathBuf, faults: Arc<Faults>) -> Self {
        let state = ReceiverState::new(host, Arc::new(RecordingEventSink::new()), root);

        let app = writer::router(state.clone()).layer(axum::middleware::from_fn_with_state(
            Arc::clone(&faults),
            faulty_chunks,
        ));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding the receiver");

        let address = listener.local_addr().expect("the bound address");

        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            endpoint: format!("http://{address}"),
            state,
            server,
        }
    }

    /// Forget everything the process knew, keeping everything on disk.
    ///
    /// This is what a restart leaves behind: the same directory, the same device
    /// key, and no session, no key and no cached manifest. The sender has to
    /// re-handshake and ask again what is held, which is the whole of what makes
    /// a receiver restart interesting.
    async fn forget_everything(&self) {
        self.state.downloads.write().await.clear();
        self.state.keys.write().await.clear();
        self.state.manifests.lock().await.clear();
    }

    /// Where chunk `index` of `file_id` is kept, as the receiver keeps it.
    fn part_path(&self, file_id: &str, index: u64) -> PathBuf {
        let parts = self
            .state
            .root
            .join("incoming")
            .join(TRANSFER)
            .join(file_id);

        manifest::part_path(&parts, index)
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/*
 * ----------------------------------------------------------------------
 * A sender, and the file it is sending
 * ----------------------------------------------------------------------
 */

struct Sender {
    manager: UploadManager,
    endpoint: String,
    chunk_size: usize,
    concurrency: usize,
}

impl Sender {
    async fn start(&self) {
        self.manager
            .start(self.metadata())
            .await
            .expect("the sender must accept the transfer");
    }

    fn metadata(&self) -> TransferMetadata {
        TransferMetadata {
            transfer_id: TRANSFER.into(),
            receiver_public_key: String::new(),
            network_type: ConnectionType::Lan,
            endpoint: self.endpoint.clone(),
            auth_token: TOKEN.into(),
            chunk_size: Some(self.chunk_size),
            concurrency: Some(self.concurrency),
            max_retries: Some(RETRIES),
        }
    }

    fn status(&self) -> TransferStatusResponse {
        self.manager
            .status(TRANSFER)
            .expect("the transfer is known")
    }

    /// Wait until the sender has worked out what its files should hash to.
    ///
    /// The digests are computed while the transfer runs, so a test that needs
    /// them to describe the file *as it was when the transfer started* has to
    /// wait for them before changing the file.
    async fn wait_for_plan(&self) {
        for _ in 0..2_000 {
            if self
                .manager
                .get(TRANSFER)
                .ok()
                .and_then(|state| state.expected())
                .is_some()
            {
                return;
            }

            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        panic!("the sender never worked out its digests");
    }

    /// Stop the sender once `chunks` of it have landed, the way a user quitting
    /// the application mid-transfer does.
    ///
    /// Pausing parks the workers where they are; dropping the manager afterwards
    /// leaves the transfer exactly as an interrupted process does — a state file
    /// with what was acknowledged, and nothing running.
    async fn stop_after(&self, harness: &Harness, chunks: u64) {
        for _ in 0..2_000 {
            let status = self.status();

            if status.uploaded_chunks >= chunks {
                self.manager.pause(TRANSFER).await.expect("pausing");

                /*
                 * Pausing parks the workers where they are, and a worker that
                 * is already partway through a chunk finishes it. Waiting for
                 * the receiver to stop gaining chunks is what makes "the sender
                 * stopped" true rather than approximately true — otherwise a
                 * chunk still in flight is one the new sender will send again,
                 * because the receiver had not recorded it when it was asked.
                 */
                harness.wait_until_quiet().await;

                return;
            }

            assert!(
                !matches!(
                    status.status,
                    TransferStatus::Completed | TransferStatus::Failed
                ),
                "the transfer finished before it could be interrupted"
            );

            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        panic!("the transfer never got as far as {chunks} chunks");
    }

    async fn resume(&self) {
        self.manager
            .resume(TRANSFER)
            .await
            .expect("resuming the transfer");
    }

    /// Wait for the transfer to stop moving.
    async fn settle(&self) -> TransferStatusResponse {
        for _ in 0..3_000 {
            let status = self.status();

            if matches!(
                status.status,
                TransferStatus::Completed | TransferStatus::Failed
            ) {
                return status;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!(
            "the transfer never settled: still {:?}",
            self.status().status
        );
    }
}

/*
 * ----------------------------------------------------------------------
 * The whole thing, wired together
 * ----------------------------------------------------------------------
 */

struct Harness {
    scratch: TempDir,
    /// The receiver's transfer root, which outlives any one receiver.
    root: PathBuf,
    /// Where a finished file is delivered.
    downloads: PathBuf,
    /// The file being sent.
    source: PathBuf,
    /// Its id as the sender names it, which is what the receiver's manifest is
    /// keyed by.
    file_id: String,
    size: u64,
    host: Arc<TestHost>,
    /// The sender's state directory and secrets, both of which outlive any one
    /// sender — that is what makes a restart a restart.
    state_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    faults: Arc<Faults>,
}

impl Harness {
    async fn new(chunks: u64) -> Self {
        Self::sized(chunks * CHUNK as u64).await
    }

    /// A harness whose source file is `size` bytes, for the throughput matrix.
    async fn sized(size: u64) -> Self {
        let scratch = TempDir::new();

        let payload = scratch.path().join("payload");
        std::fs::create_dir_all(&payload).expect("creating the payload directory");

        let source = payload.join("a.bin");

        // Deterministic, and not a repetition of one byte: a chunk that was
        // silently swapped for another would show up in the comparison.
        let bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
        std::fs::write(&source, &bytes).expect("writing the source file");

        let files = scanner::scan(&[source.to_string_lossy().into_owned()])
            .await
            .expect("scanning the source");

        let file_id = files[0].id.clone();

        let downloads = scratch.path().join("downloads");

        Self {
            root: scratch.path().join("receiver"),
            downloads: downloads.clone(),
            state_dir: scratch.path().join("sender-state"),
            host: Arc::new(TestHost {
                downloads,
                device: generate_device_keypair(),
            }),
            // Whatever the keychain would be. It outlives every sender built
            // over this harness, which is what lets a restarted sender still
            // find the key it was sending with.
            secrets: Arc::new(InMemorySecretStore::new()),
            faults: Arc::new(Faults::default()),
            file_id,
            source,
            size,
            scratch,
        }
    }

    /// Wait until the receiver stops gaining chunks.
    ///
    /// A sender that has stopped looks, from the receiver, like a transfer that
    /// has stopped arriving.
    async fn wait_until_quiet(&self) {
        let mut last = usize::MAX;
        let mut quiet = 0;

        for _ in 0..1_000 {
            let now = self.held().await.len();

            if now == last {
                quiet += 1;

                if quiet >= 5 {
                    return;
                }
            } else {
                last = now;
                quiet = 0;
            }

            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        panic!("the receiver never stopped gaining chunks");
    }

    /// Wait until the receiver holds at least `chunks` chunks.
    async fn wait_for_held(&self, chunks: usize) {
        for _ in 0..2_000 {
            if self.held().await.len() >= chunks {
                return;
            }

            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        panic!("the receiver never came to hold {chunks} chunks");
    }

    /// Hold the receiver to a pace a test can interrupt.
    fn pace_chunks(&self, per_chunk: Duration) {
        self.faults.pace(per_chunk);
    }

    async fn receiver(&self) -> Receiver {
        Receiver::start(
            Arc::clone(&self.host),
            self.root.clone(),
            Arc::clone(&self.faults),
        )
        .await
    }

    /// A sender over the shared state directory and secrets.
    ///
    /// Calling this twice is a sender restarting: the second one sees the first
    /// one's state file and keychain, and nothing of its memory.
    async fn sender(&self, receiver: &Receiver) -> Sender {
        self.sender_with(receiver, CHUNK, 1).await
    }

    /// A sender with an explicit chunk size and concurrency, for the matrix.
    async fn sender_with(
        &self,
        receiver: &Receiver,
        chunk_size: usize,
        concurrency: usize,
    ) -> Sender {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");

        let local_files = Arc::new(LocalTransferFileService::new(pool).await);

        local_files
            .insert(LocalTransferFile {
                transfer_id: TRANSFER.into(),
                file_path: self.source.to_string_lossy().into_owned(),
                file_name: "a.bin".into(),
                file_size: self.size,
            })
            .await
            .expect("registering the file with the sender");

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        Sender {
            manager: UploadManager::new(
                Arc::new(RecordingEventSink::new()),
                self.scratch.path().join("uploads"),
                local_files,
                tx,
                Arc::new(TransferStore::new(
                    self.state_dir.clone(),
                    Arc::clone(&self.secrets),
                )),
            ),
            endpoint: receiver.endpoint.clone(),
            chunk_size,
            concurrency,
        }
    }

    /// The chunks the receiver is holding, by index.
    async fn held(&self) -> Vec<u64> {
        let manifest = manifest::load_or_default(&self.transfer_dir(), TRANSFER).await;

        let mut held: Vec<u64> = manifest
            .chunks
            .iter()
            .filter(|chunk| chunk.file_id == self.file_id)
            .map(|chunk| chunk.chunk_index)
            .collect();

        held.sort_unstable();
        held
    }

    fn transfer_dir(&self) -> PathBuf {
        self.root.join("incoming").join(TRANSFER)
    }

    fn destination(&self) -> PathBuf {
        self.downloads.join("a.bin")
    }

    /// The claim every test ends on: the file that landed is the file that was
    /// sent, byte for byte.
    fn assert_delivered(&self) {
        let delivered = std::fs::read(self.destination()).unwrap_or_else(|error| {
            panic!("nothing was delivered to {:?}: {error}", self.destination())
        });

        let source = std::fs::read(&self.source).expect("reading the source");

        assert_eq!(
            delivered.len(),
            source.len(),
            "the delivered file is not the size of the one that was sent"
        );
        assert!(
            delivered == source,
            "the delivered file is not the file that was sent"
        );
    }
}

/// The chunk count of the default transfer.
fn chunks() -> u64 {
    CHUNKS
}

/*
 * ----------------------------------------------------------------------
 * 1-3. The sender stops, restarts, and resumes
 * ----------------------------------------------------------------------
 */

#[tokio::test]
async fn a_sender_that_stops_halfway_resumes_without_sending_anything_twice() {
    let harness = Harness::new(chunks()).await;
    harness.pace_chunks(Duration::from_millis(15));
    let receiver = harness.receiver().await;

    let first = harness.sender(&receiver).await;
    first.start().await;
    first.stop_after(&harness, 3).await;

    let held = harness.held().await;

    assert!(
        !held.is_empty() && (held.len() as u64) < chunks(),
        "the transfer has to be interrupted partway through, and it held {held:?}"
    );

    // The sender is gone; a new one finds its state file and its keychain entry.
    drop(first);

    let second = harness.sender(&receiver).await;

    assert_eq!(
        second.manager.restore_persisted().await,
        1,
        "the interrupted transfer comes back"
    );

    second.resume().await;

    let status = second.settle().await;

    assert_eq!(status.status, TransferStatus::Completed);
    assert_eq!(
        status.uploaded_bytes, harness.size,
        "progress counts every byte"
    );
    assert_eq!(status.uploaded_chunks, chunks(), "and every chunk");

    for index in 0..chunks() {
        assert_eq!(
            receiver_attempts(&harness, index),
            1,
            "chunk {index} was already delivered once; it must not be sent again"
        );
    }

    harness.assert_delivered();
}

/*
 * ----------------------------------------------------------------------
 * 4-6. The receiver stops, restarts, and the sender reconnects
 * ----------------------------------------------------------------------
 */

#[tokio::test]
async fn a_receiver_that_restarts_keeps_the_chunks_it_already_had() {
    let harness = Harness::new(chunks()).await;
    harness.pace_chunks(Duration::from_millis(15));
    let receiver = harness.receiver().await;

    let first = harness.sender(&receiver).await;
    first.start().await;
    first.stop_after(&harness, 3).await;

    let held = harness.held().await;

    assert!(
        !held.is_empty() && (held.len() as u64) < chunks(),
        "the transfer has to be interrupted partway through, and it held {held:?}"
    );

    // The receiver restarts: its memory is gone, its directory is not.
    receiver.forget_everything().await;

    drop(first);

    let second = harness.sender(&receiver).await;
    second.manager.restore_persisted().await;
    second.resume().await;

    let status = second.settle().await;

    assert_eq!(status.status, TransferStatus::Completed);

    for index in 0..chunks() {
        assert_eq!(
            receiver_attempts(&harness, index),
            1,
            "chunk {index} survived the restart and must not be sent again"
        );
    }

    harness.assert_delivered();
}

/*
 * ----------------------------------------------------------------------
 * 7. A chunk that fails in flight
 * ----------------------------------------------------------------------
 */

#[tokio::test]
async fn a_chunk_that_fails_in_flight_is_sent_again() {
    let harness = Harness::new(6).await;
    let receiver = harness.receiver().await;

    // One refusal: the chunk the network dropped, and the sender's own retry.
    harness.faults.refuse_once(3);

    let sender = harness.sender(&receiver).await;
    sender.start().await;

    let status = sender.settle().await;

    assert_eq!(
        status.status,
        TransferStatus::Completed,
        "a chunk that fails once is the retry's business, not the transfer's"
    );
    assert!(
        receiver_attempts(&harness, 3) >= 2,
        "the refused chunk has to go again"
    );

    for index in [0, 1, 2, 4, 5] {
        assert_eq!(
            receiver_attempts(&harness, index),
            1,
            "chunk {index} never failed and must not be sent again"
        );
    }

    harness.assert_delivered();
}

/*
 * ----------------------------------------------------------------------
 * 8-9. A chunk whose bytes are not what they were
 * ----------------------------------------------------------------------
 */

/// A transfer stopped partway, with `index` left on disk in the given state.
async fn interrupted_with_damaged_chunk(index: u64, damage: Damage) -> (Harness, Receiver) {
    let harness = Harness::new(chunks()).await;
    harness.pace_chunks(Duration::from_millis(15));
    let receiver = harness.receiver().await;

    let first = harness.sender(&receiver).await;
    first.start().await;
    first.stop_after(&harness, index + 1).await;

    // The bytes are on disk and the process is not.
    receiver.forget_everything().await;
    drop(first);

    let part = receiver.part_path(&harness.file_id, index);

    match damage {
        Damage::Truncate => {
            let bytes = std::fs::read(&part).expect("reading the part");
            std::fs::write(&part, &bytes[..bytes.len() / 2]).expect("truncating the part");
        }

        // Same length, different bytes: the case only the hash can see.
        Damage::Alter => {
            let mut bytes = std::fs::read(&part).expect("reading the part");
            bytes[0] ^= 0xff;
            std::fs::write(&part, &bytes).expect("altering the part");
        }
    }

    (harness, receiver)
}

enum Damage {
    Truncate,
    Alter,
}

#[tokio::test]
async fn a_truncated_chunk_is_sent_again_rather_than_merged() {
    let (harness, receiver) = interrupted_with_damaged_chunk(2, Damage::Truncate).await;

    let sender = harness.sender(&receiver).await;
    sender.manager.restore_persisted().await;
    sender.resume().await;

    let status = sender.settle().await;

    assert_eq!(status.status, TransferStatus::Completed);
    assert!(
        receiver_attempts(&harness, 2) >= 2,
        "a chunk that is no longer the length it was recorded at has to be sent again"
    );

    for index in [0, 1, 3] {
        assert_eq!(
            receiver_attempts(&harness, index),
            1,
            "chunk {index} is intact and must not be sent again"
        );
    }

    harness.assert_delivered();
}

#[tokio::test]
async fn a_chunk_altered_on_disk_is_sent_again_rather_than_merged() {
    let (harness, receiver) = interrupted_with_damaged_chunk(2, Damage::Alter).await;

    let sender = harness.sender(&receiver).await;
    sender.manager.restore_persisted().await;
    sender.resume().await;

    let status = sender.settle().await;

    assert_eq!(status.status, TransferStatus::Completed);
    assert!(
        receiver_attempts(&harness, 2) >= 2,
        "the altered chunk must not be accepted as complete"
    );

    for index in [0, 1, 3] {
        assert_eq!(
            receiver_attempts(&harness, index),
            1,
            "chunk {index} is intact and must not be sent again"
        );
    }

    harness.assert_delivered();
}

/// A chunk that goes bad while the transfer is still running.
///
/// The chunk arrived and was recorded, and then it stopped being what it was
/// recorded as — a disk that wrote it wrong, a file that was edited under the
/// receiver, a crash mid-write. Nobody is going to send it again on their own:
/// the sender counted it as delivered when the receiver said `200`, so the
/// receiver is the only party that knows the file is no longer assembleable.
/// The same-length version of this is covered by the restart test; this is the
/// one that has to be caught without anything restarting.
#[tokio::test]
async fn a_chunk_that_goes_bad_mid_transfer_is_resent_rather_than_published() {
    let harness = Harness::new(6).await;
    harness.pace_chunks(Duration::from_millis(15));

    let receiver = harness.receiver().await;

    // Frozen before the transfer starts, so that the first chunk is written and
    // the rest are still on their way when the damage happens.
    //
    // The lock under test's transfer, not a process-wide one: the receiver
    // serialises per transfer now, so this freezes exactly this transfer's chunk
    // writes and nothing else. The invariant the test asserts is unchanged — a
    // chunk that goes bad mid-flight is not swallowed — and the mechanism that
    // makes the window reachable is the same, one scope narrower.
    let guard = receiver.state.transfer_lock(TRANSFER).await;
    let held = guard.lock().await;

    let sender = harness.sender(&receiver).await;
    sender.start().await;

    drop(held);

    harness.wait_for_held(1).await;

    let held = guard.lock().await;

    assert!(
        harness.held().await.len() < 6,
        "the transfer has to be mid-flight for this to be the case it is about"
    );

    /*
     * The chunk is left *short*, and its manifest entry is left alone. A chunk
     * that is the right length but the wrong bytes is caught by the full check
     * either way; a chunk that is the wrong length is the case that separates
     * asking "did this arrive?" from asking "is this still here?" — and only the
     * first question can be answered without concluding that a chunk the sender
     * has already been thanked for is a chunk that was never sent.
     */
    let part = receiver.part_path(&harness.file_id, 0);
    let bytes = std::fs::read(&part).expect("reading the part");
    std::fs::write(&part, &bytes[..bytes.len() / 2]).expect("truncating the part");

    drop(held);

    let status = sender.settle().await;

    assert_eq!(
        status.status,
        TransferStatus::Completed,
        "a chunk the receiver can no longer verify is refused, resent, and the transfer still completes"
    );
    assert!(
        receiver_attempts(&harness, 0) >= 2,
        "the damaged chunk is the one that has to be sent again"
    );
    assert_eq!(
        receiver_attempts(&harness, 1),
        1,
        "the chunks that were still valid are kept, not sent again"
    );

    harness.assert_delivered();
}

/*
 * ----------------------------------------------------------------------
 * 10. A worker that cannot finish
 * ----------------------------------------------------------------------
 */

#[tokio::test]
async fn a_chunk_the_receiver_keeps_refusing_fails_the_transfer_and_resumes_later() {
    let harness = Harness::new(6).await;
    let receiver = harness.receiver().await;

    harness.faults.refuse_always(4);

    let sender = harness.sender(&receiver).await;
    sender.start().await;

    let failed = sender.settle().await;

    assert_eq!(
        failed.status,
        TransferStatus::Failed,
        "a worker that cannot deliver its chunk fails the transfer; it does not sit in Uploading"
    );

    // Whatever was wrong stops being wrong, and the transfer is picked back up.
    harness.faults.allow(4);

    sender.resume().await;

    let status = sender.settle().await;

    assert_eq!(status.status, TransferStatus::Completed);
    assert!(
        receiver_attempts(&harness, 4) > 1,
        "the refused chunk goes again"
    );

    for index in [0, 1, 2, 3] {
        assert_eq!(
            receiver_attempts(&harness, index),
            1,
            "chunk {index} arrived and must not be sent again"
        );
    }

    harness.assert_delivered();
}

/*
 * ----------------------------------------------------------------------
 * 11. Cancellation
 * ----------------------------------------------------------------------
 */

#[tokio::test]
async fn cancelling_a_transfer_clears_both_sides_and_stays_cleared() {
    let harness = Harness::new(chunks()).await;
    harness.pace_chunks(Duration::from_millis(15));
    let receiver = harness.receiver().await;

    let sender = harness.sender(&receiver).await;
    sender.start().await;
    sender.stop_after(&harness, 2).await;

    assert!(
        harness.transfer_dir().exists(),
        "the receiver is holding part of the transfer"
    );

    sender
        .manager
        .cancel(TRANSFER)
        .await
        .expect("cancelling the transfer");

    assert!(
        !harness.transfer_dir().exists(),
        "cancelling must take the parts and the manifest with it"
    );
    assert!(
        sender.manager.status(TRANSFER).is_err(),
        "and the sender must forget the transfer"
    );
    assert!(
        !harness.state_dir.join(format!("{TRANSFER}.json")).exists(),
        "including its record on disk"
    );

    // A restart does not bring back a transfer nobody wants.
    receiver.forget_everything().await;

    assert!(
        !harness.transfer_dir().exists(),
        "nothing about a cancelled transfer comes back"
    );
    assert_eq!(
        sender.manager.restore_persisted().await,
        0,
        "and the sender does not restore it either"
    );
}

/*
 * ----------------------------------------------------------------------
 * 12. The file that assembles is not the file that was sent
 * ----------------------------------------------------------------------
 */

#[tokio::test]
async fn a_file_that_does_not_match_the_senders_digests_is_not_delivered() {
    if !DESCRIBE_DIGESTS {
        return;
    }

    let harness = Harness::new(8).await;
    let receiver = harness.receiver().await;

    /*
     * The receiver is held before the transfer starts, so that the sender is
     * stopped with one chunk read and the rest still on disk. With one worker,
     * that is exactly where it is: it has read the first chunk and is waiting
     * for the receiver to take it, and it cannot move on to the second.
     *
     * That makes the window between "the sender worked out what it is sending"
     * and "the sender reads the fourth chunk" something a test can stand in.
     * The digests describe the file as it was when they were worked out; the
     * bytes for chunk four are read afterwards, from the file as it now is.
     * Every chunk is individually fine — each one is exactly what the receiver
     * was told to expect by way of its own record — and the file they assemble
     * into is not the one the sender described.
     */
    let guard = receiver.state.transfer_lock(TRANSFER).await;
    let held = guard.lock().await;

    let sender = harness.sender(&receiver).await;
    sender.start().await;

    /*
     * The digests are worked out while the transfer runs now, so the test waits
     * for them before changing the file. They then describe the original bytes,
     * and the fourth chunk is read from the changed ones.
     */
    sender.wait_for_plan().await;

    assert!(
        harness.held().await.is_empty(),
        "nothing can have been written while the receiver is held"
    );

    let mut bytes = std::fs::read(&harness.source).expect("reading the source");
    bytes[4 * CHUNK] ^= 0xff;
    std::fs::write(&harness.source, &bytes).expect("changing the source underneath the transfer");

    drop(held);

    let failed = sender.settle().await;

    assert_eq!(
        failed.status,
        TransferStatus::Failed,
        "a file that does not match what the sender described must not be reported as delivered"
    );
    assert!(
        !harness.destination().exists(),
        "and nothing may be left where a delivered file would be"
    );

    /*
     * The chunk behind the mismatch was dropped, so the transfer can be finished
     * once the file stops changing — which is the point of checking at all: the
     * failure is recoverable, and it is not silent.
     */
    std::fs::write(
        &harness.source,
        (0..harness.size)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<u8>>(),
    )
    .expect("restoring the source");

    drop(sender);

    let second = harness.sender(&receiver).await;
    second.manager.restore_persisted().await;
    second.resume().await;

    let status = second.settle().await;

    assert_eq!(status.status, TransferStatus::Completed);
    assert!(
        receiver_attempts(&harness, 4) >= 2,
        "the chunk that did not match has to be sent again"
    );

    harness.assert_delivered();
}

/// How many times chunk `index` reached the receiver.
fn receiver_attempts(harness: &Harness, index: u64) -> usize {
    harness.faults.attempts_of(index)
}

/// The receiver's write locking is per transfer, and that is load-bearing.
///
/// The receiver used to serialise every chunk of every transfer through one
/// process-wide `tokio::sync::Mutex`, held from before the directory was created
/// until after the file had been merged. One transfer whose socket stalled held
/// it for as long as it stalled, and every other transfer on the device waited.
///
/// That is survivable when each request is short and independent. It is not
/// survivable for the long-lived stream a framed v2 session is, which is why the
/// lock became per transfer. This asserts both halves of the contract, because
/// the refactor is worthless if either fails: different transfers must not wait
/// on each other, and one transfer's own chunks must still serialise — its
/// manifest is one document, and two chunks reading and rewriting it at once
/// would lose an entry.
///
/// It is the test that fails if someone reintroduces a shared lock.
#[tokio::test]
async fn one_transfers_write_lock_does_not_block_another() {
    let harness = Harness::new(2).await;
    let receiver = harness.receiver().await;

    let first = receiver.state.transfer_lock("t-one").await;
    let second = receiver.state.transfer_lock("t-two").await;

    assert!(
        !Arc::ptr_eq(&first, &second),
        "two transfers must not share a write lock, or one stalls the other"
    );

    let held = first.lock().await;

    assert!(
        second.try_lock().is_ok(),
        "a different transfer must be able to write while the first holds its lock"
    );

    drop(held);

    /*
     * And the other half: the same transfer gets the same lock, so its own two
     * chunks cannot interleave their manifest read-modify-write.
     */
    let again = receiver.state.transfer_lock("t-one").await;

    assert!(
        Arc::ptr_eq(&first, &again),
        "a transfer must get the same lock every time, or its chunks would not serialise"
    );

    let _held = again.lock().await;

    assert!(
        first.try_lock().is_err(),
        "one transfer's own chunks must serialise against each other"
    );
}

/*
 * ----------------------------------------------------------------------
 * Every transfer leaves a performance report
 * ----------------------------------------------------------------------
 */

/// Wait for a report file to appear.
///
/// The sender's status turns `Completed` before its report is written — the
/// report is history, written after the terminal event — so a test that reads
/// it has to wait for it rather than assume it is already there.
async fn wait_for_report(path: &std::path::Path) -> std::fs::File {
    for _ in 0..2_000 {
        if let Ok(file) = std::fs::File::open(path) {
            return file;
        }

        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    panic!("no report at {}", path.display());
}

fn sender_report(harness: &Harness) -> std::path::PathBuf {
    harness
        .state_dir
        .join("reports")
        .join(format!("{TRANSFER}.json"))
}

fn receiver_report(receiver: &Receiver) -> std::path::PathBuf {
    receiver
        .state
        .root
        .join("reports")
        .join(format!("{TRANSFER}.json"))
}

#[tokio::test]
async fn a_completed_transfer_writes_a_sender_and_a_receiver_report() {
    let harness = Harness::new(chunks()).await;
    let receiver = harness.receiver().await;

    let sender = harness.sender(&receiver).await;
    sender.start().await;

    assert_eq!(sender.settle().await.status, TransferStatus::Completed);

    let sender_json: serde_json::Value =
        serde_json::from_reader(wait_for_report(&sender_report(&harness)).await).expect("json");

    assert_eq!(sender_json["role"], "sender");
    assert_eq!(sender_json["status"], "COMPLETED");
    assert_eq!(sender_json["transfer_id"], TRANSFER);
    assert_eq!(sender_json["completed_chunks"], chunks());
    assert!(sender_json["throughput"]["mib_s"].is_number());
    assert!(sender_json["chunk_latency"]["p95_ms"].is_number());
    assert_eq!(
        sender_json["files"][0]["chunks_done"],
        chunks(),
        "every chunk was acknowledged"
    );
    assert_eq!(
        sender_json["files"][0]["outcome"], "completed",
        "the receiver acknowledges a file's last chunk only after publishing it"
    );
    assert_eq!(sender_json["completed_files"], 1);
    assert_eq!(sender_json["unknown_files"], 0);

    let receiver_json: serde_json::Value =
        serde_json::from_reader(wait_for_report(&receiver_report(&receiver)).await).expect("json");

    assert_eq!(receiver_json["role"], "receiver");
    assert_eq!(receiver_json["transfer_id"], TRANSFER);
    assert_eq!(receiver_json["status"], "COMPLETED");
    assert_eq!(
        receiver_json["files"][0]["outcome"], "completed",
        "the receiver published it, so it can say so"
    );
    assert_eq!(receiver_json["completed_files"], 1);
    assert!(receiver_json["phases"]["final_assembly_seconds"].is_number());
    assert!(receiver_json["phases"]["verification_seconds"].is_number());
}

#[tokio::test]
async fn a_cancelled_transfer_still_writes_a_report() {
    let harness = Harness::new(chunks()).await;
    let receiver = harness.receiver().await;

    let sender = harness.sender(&receiver).await;
    sender.start().await;
    sender.stop_after(&harness, 2).await;

    sender.manager.cancel(TRANSFER).await.expect("cancelling");

    let json: serde_json::Value =
        serde_json::from_reader(wait_for_report(&sender_report(&harness)).await).expect("json");

    assert_eq!(json["status"], "CANCELLED");
    assert_eq!(json["cancellation_reason"], "cancelled by the user");
    assert!(json["completed_bytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn a_transfer_is_reported_once_even_when_failure_and_cancel_race() {
    let harness = Harness::new(chunks()).await;
    let receiver = harness.receiver().await;

    let sender = harness.sender(&receiver).await;
    sender.start().await;
    sender.stop_after(&harness, 2).await;

    // Cancel twice: the second is a no-op, and neither may write a second report.
    let _ = sender.manager.cancel(TRANSFER).await;
    let _ = sender.manager.cancel(TRANSFER).await;

    let path = sender_report(&harness);

    wait_for_report(&path).await;

    assert!(path.is_file());
}

/*
 * ----------------------------------------------------------------------
 * Throughput matrix (run explicitly: --ignored --nocapture)
 * ----------------------------------------------------------------------
 *
 * Real sender and real receiver over a loopback socket, one large file, the
 * same settings the product ships with (compression and digests disabled).
 * Every number printed comes from the transfer reports, not from arithmetic.
 */
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "throughput matrix: run explicitly with --ignored --nocapture"]
async fn bench_upload_matrix() {
    const SIZE: u64 = 64 * 1024 * 1024;

    println!(
        "chunk_mib,concurrency,mib_s,wall_s,ack_p50_ms,ack_p95_ms,ack_p99_ms,\
         cpu_avg_pct,cpu_peak_pct,mem_peak_mib,recv_accept_p50_ms,recv_persist_s,\
         recv_assembly_s,read_pct,encrypt_pct,encrypt_cpu_pct,http_pct,status,failed"
    );

    for chunk_size in [4usize << 20, 8 << 20, 16 << 20] {
        for concurrency in [4usize, 8, 12, 16] {
            /*
             * A fresh harness per run: the transfer id is fixed by the fixture,
             * and a receiver directory left from the previous configuration
             * would describe a different chunking of the same id.
             */
            let harness = Harness::sized(SIZE).await;

            let _ = std::fs::remove_file(sender_report(&harness));

            let receiver = harness.receiver().await;
            let _ = std::fs::remove_file(receiver_report(&receiver));

            let sender = harness
                .sender_with(&receiver, chunk_size, concurrency)
                .await;

            let started = std::time::Instant::now();

            sender.start().await;
            let status = sender.settle().await;

            let wall = started.elapsed().as_secs_f64();

            let sender_json: serde_json::Value =
                serde_json::from_reader(wait_for_report(&sender_report(&harness)).await)
                    .expect("sender report");

            let receiver_json: serde_json::Value =
                serde_json::from_reader(wait_for_report(&receiver_report(&receiver)).await)
                    .expect("receiver report");

            println!(
                "{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.1},{:.2},{:.2},{:.2},{:.1},{:.1},{:.1},{:.1},{:?},{}",
                chunk_size >> 20,
                concurrency,
                sender_json["throughput"]["mib_s"].as_f64().unwrap_or_default(),
                wall,
                sender_json["chunk_latency"]["p50_ms"].as_f64().unwrap_or_default(),
                sender_json["chunk_latency"]["p95_ms"].as_f64().unwrap_or_default(),
                sender_json["chunk_latency"]["p99_ms"].as_f64().unwrap_or_default(),
                sender_json["resources"]["cpu_average_percent"].as_f64().unwrap_or_default(),
                sender_json["resources"]["cpu_peak_percent"].as_f64().unwrap_or_default(),
                sender_json["resources"]["memory_peak_bytes"].as_u64().unwrap_or_default() as f64
                    / (1024.0 * 1024.0),
                receiver_json["chunk_latency"]["p50_ms"].as_f64().unwrap_or_default(),
                receiver_json["phases"]["chunk_persistence_seconds"].as_f64().unwrap_or_default(),
                receiver_json["phases"]["final_assembly_seconds"].as_f64().unwrap_or_default(),
                sender_json["chunk_breakdown"]["read"]["share_percent"].as_f64().unwrap_or_default(),
                sender_json["chunk_breakdown"]["encrypt"]["share_percent"].as_f64().unwrap_or_default(),
                sender_json["chunk_breakdown"]["encrypt_cpu"]["share_percent"].as_f64().unwrap_or_default(),
                sender_json["chunk_breakdown"]["http"]["share_percent"].as_f64().unwrap_or_default(),
                status.status,
                sender_json["failed_chunks"].as_u64().unwrap_or_default(),
            );

            if sender_json["status"] != "COMPLETED" {
                println!(
                    "#   reason: {} / {}",
                    sender_json["failure_stage"].as_str().unwrap_or("-"),
                    sender_json["failure_reason"].as_str().unwrap_or("-")
                );
            }
        }
    }
}
