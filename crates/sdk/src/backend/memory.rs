//! The in-memory backend: a whole transfer inside one process.
//!
//! # Why this exists
//!
//! ADR-0011 §3 makes `VilsendBuilder::in_memory()` a *contract* — "it must
//! touch **nothing external** — no filesystem, no keychain, no network, no real
//! clock" — and the same ADR gives the reason: "if a test needs a real socket
//! or a real keychain, that is an architectural finding, not a testing
//! inconvenience." A facade whose only backend needed a socket would be a
//! facade nobody could test, so the in-memory backend is the one that makes the
//! public API checkable at all.
//!
//! # How a transfer happens
//!
//! There is no wire. The two halves of a transfer are two objects in one
//! process, and the "transport" is a method call that hands a chunk's bytes and
//! its digest from one to the other:
//!
//! 1. `receive` registers a listener and hands back a handle. Its id is
//!    allocated here, not by the sender, because `TransferHandle::id` is
//!    synchronous and a handle must be able to answer before anything has
//!    arrived.
//! 2. `send` claims the earliest listener waiting on that peer — first in,
//!    first served — and adopts its id, so both handles name the same transfer.
//!    A `send` with no listener gets [`vilsend_core::ErrorKind::NoRoute`], which
//!    is the honest answer and is also the first real use of that error kind in
//!    the product.
//! 3. The chunks are cut, digested, handed over, counted and acknowledged.
//! 4. The receiving side assembles each file **in memory** and only writes it
//!    to the sink once every chunk has arrived — the shape the shipped
//!    receiver's `.part`-then-merge has, minus the filesystem.
//!
//! # What is deliberately not modelled
//!
//! **Concurrency and retries.** `Policy::concurrency` and `Policy::max_retries`
//! are resolved and validated but do not change this backend's behaviour,
//! because the link never fails and the chunks go over in order — a retry here
//! would be a retry of a function call. Those knobs become real with the
//! `Transport` port in Phase 8, and pretending otherwise would mean testing a
//! fiction.
//!
//! **Pausing.** `pause` and `resume` move the reported status on the handle
//! they were called on and nothing else; there is no state in which the link is
//! blocked. A sender that parked on a paused receiver would park until a future
//! the caller has no way to poll together with it ran, which is not a feature
//! so much as a deadlock with extra steps.
//!
//! **Cancelling a transfer that is already moving, from the sending side.** The
//! sending handle is not handed out until the transfer is over, so it cannot be
//! acted on while it runs. Cancelling the *receiving* handle is reachable and is
//! honoured at the next chunk boundary — see [`MemoryBackend::run`].

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use vilsend_core::progress::{download_progress, upload_progress};
use vilsend_core::{DomainEvent, EventSink, PeerRef, TransferId, VilsendError};

use super::{Backend, BoxFuture};
use crate::auth::AuthState;
use crate::clock::Clock;
use crate::events::{EventBus, EventStream};
use crate::files::MemoryFiles;
use crate::handle::{Outcome, ReceivedFile, ReceivedOutcome, SentOutcome, TransferHandle};
use crate::ids::Destination;
use crate::progress::TransportKind;
use crate::request::{Policy, ReceiveRequest, SendRequest};
use crate::state::{yield_once, Side, Slot};

/// Which of the three terminal events a transfer just produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terminal {
    Completed,
    Failed,
    Cancelled,
}

/// A listener waiting to be matched with a sender.
#[derive(Debug)]
struct Pending {
    transfer_id: TransferId,
    destination: Destination,
    policy: Policy,
    slot: Arc<Slot>,
    assembly: Mutex<Assembly>,
}

impl Pending {
    /// Takes one chunk, verifying it first.
    ///
    /// Returns whether it was *newly* stored. A chunk re-sent to an index that
    /// already has one is stored again but not counted twice — the same
    /// tolerance the shipped receiver's `.part` gate has, kept because a
    /// re-sending peer is a real case and not a bug to reject.
    fn accept_chunk(
        &self,
        relative_path: &str,
        index: u64,
        expected_chunks: u64,
        data: &[u8],
        digest: &[u8],
        verify: bool,
    ) -> Result<bool, VilsendError> {
        if verify && Sha256::digest(data).as_slice() != digest {
            return Err(VilsendError::IntegrityMismatch(format!(
                "chunk {index} of {relative_path} did not match its digest"
            )));
        }

        let mut assembly = self.assembly.lock().expect("assembly lock poisoned");

        if !assembly.files.contains_key(relative_path) {
            // Recorded once, on the file's first chunk, so `finalize` can
            // report the files in the order their bytes started arriving.
            assembly.order.push(relative_path.to_owned());
        }

        let file = assembly
            .files
            .entry(relative_path.to_owned())
            .or_insert_with(|| FileAssembly {
                total_bytes: 0,
                expected_chunks,
                chunks: BTreeMap::new(),
            });

        let is_new = !file.chunks.contains_key(&index);

        file.chunks.insert(index, data.to_vec());

        if is_new {
            file.total_bytes = file.total_bytes.saturating_add(data.len() as u64);
        }

        Ok(is_new)
    }

    /// Assembles everything received and writes it to `sink`.
    ///
    /// All or nothing: a file with a chunk missing fails the whole receive
    /// rather than leaving a short file in the sink, because a short file that
    /// looks complete is worse than no file.
    fn finalize(&self, sink: &MemoryFiles) -> Result<Vec<ReceivedFile>, VilsendError> {
        let mut assembly = self.assembly.lock().expect("assembly lock poisoned");

        let order = std::mem::take(&mut assembly.order);
        let files = std::mem::take(&mut assembly.files);

        let mut received = Vec::with_capacity(files.len());

        for relative_path in order {
            let Some(file) = files.get(&relative_path) else {
                continue;
            };

            if file.chunks.len() as u64 != file.expected_chunks {
                return Err(VilsendError::IntegrityMismatch(format!(
                    "{} of {} chunks arrived for {relative_path}",
                    file.chunks.len(),
                    file.expected_chunks
                )));
            }

            let mut bytes = Vec::with_capacity(file.total_bytes as usize);

            // `BTreeMap` iterates in index order, so walking it and asserting
            // the index is what catches a gap or a duplicate. The count check
            // above cannot: 3 chunks numbered 0, 1, 3 have the right count and
            // one hole.
            for (expected, (index, chunk)) in file.chunks.iter().enumerate() {
                if *index != expected as u64 {
                    return Err(VilsendError::IntegrityMismatch(format!(
                        "chunk {index} of {relative_path} arrived where {} was expected",
                        expected
                    )));
                }

                bytes.extend_from_slice(chunk);
            }

            let path = join(&self.destination, &relative_path);

            sink.insert(path.clone(), bytes);

            received.push(ReceivedFile {
                path,
                bytes: file.total_bytes,
                // Every chunk was checked on the way in, and one that failed
                // failed the transfer.
                verified: true,
            });
        }

        Ok(received)
    }
}

/// The chunks of one file, waiting to be assembled.
///
/// Keyed by its path in [`Assembly`], so the path is not repeated here.
#[derive(Debug)]
struct FileAssembly {
    total_bytes: u64,
    expected_chunks: u64,
    chunks: BTreeMap<u64, Vec<u8>>,
}

#[derive(Debug, Default)]
struct Assembly {
    files: BTreeMap<String, FileAssembly>,
    /// File paths in the order their first chunk arrived, so `ReceivedOutcome`
    /// reports them in the order they were sent rather than alphabetically.
    order: Vec<String>,
}

/// The listeners waiting for a sender, by peer.
#[derive(Debug, Default)]
struct Link {
    waiting: Mutex<HashMap<PeerRef, VecDeque<Arc<Pending>>>>,
}

impl Link {
    fn push(&self, peer: PeerRef, pending: Arc<Pending>) {
        self.waiting
            .lock()
            .expect("link lock poisoned")
            .entry(peer)
            .or_default()
            .push_back(pending);
    }

    /// The earliest listener for `peer`, removed from the queue.
    fn take(&self, peer: &PeerRef) -> Option<Arc<Pending>> {
        self.waiting
            .lock()
            .expect("link lock poisoned")
            .get_mut(peer)?
            .pop_front()
    }

    /// Releases a listener that was cancelled before a sender arrived.
    fn remove(&self, peer: &PeerRef, transfer_id: &TransferId) {
        let mut waiting = self.waiting.lock().expect("link lock poisoned");

        let Some(queue) = waiting.get_mut(peer) else {
            return;
        };

        queue.retain(|pending| &pending.transfer_id != transfer_id);
    }

    /// How many listeners are waiting on `peer`.
    #[cfg(test)]
    fn waiting_on(&self, peer: &PeerRef) -> usize {
        self.waiting
            .lock()
            .expect("link lock poisoned")
            .get(peer)
            .map_or(0, VecDeque::len)
    }
}

/// One file to send, resolved to bytes.
struct PlannedFile {
    relative_path: String,
    bytes: Vec<u8>,
}

/// The `in_memory()` backend.
pub(crate) struct MemoryBackend {
    link: Arc<Link>,
    source: MemoryFiles,
    sink: MemoryFiles,
    clock: Arc<dyn Clock>,
    events: EventBus,
    policy: Policy,
    next_transfer: AtomicU64,
}

impl MemoryBackend {
    pub(crate) fn new(
        source: MemoryFiles,
        sink: MemoryFiles,
        clock: Arc<dyn Clock>,
        events: EventBus,
        policy: Policy,
    ) -> Self {
        Self {
            link: Arc::new(Link::default()),
            source,
            sink,
            clock,
            events,
            policy,
            next_transfer: AtomicU64::new(0),
        }
    }

    fn next_id(&self) -> TransferId {
        let index = self.next_transfer.fetch_add(1, Ordering::SeqCst);

        TransferId::new(format!("memory-{index}"))
    }

    async fn do_send(&self, request: SendRequest) -> Result<TransferHandle, VilsendError> {
        let policy = request.effective_policy(&self.policy)?;

        if request.files.is_empty() {
            return Err(VilsendError::InvalidInput(
                "a send request must name at least one file".into(),
            ));
        }

        // Resolve every file **before** claiming a listener: a request that
        // names a file nobody has is the caller's mistake, and it must not cost
        // some other caller their waiting receiver.
        let mut planned = Vec::with_capacity(request.files.len());

        for handle in &request.files {
            let bytes = self.source.get(handle.as_str()).ok_or_else(|| {
                VilsendError::NotFound(format!("the source has no file for {handle}"))
            })?;

            planned.push(PlannedFile {
                relative_path: handle.as_str().to_owned(),
                bytes,
            });
        }

        let receiver = self.link.take(&request.peer).ok_or(VilsendError::NoRoute)?;

        let total_bytes: u64 = planned.iter().map(|file| file.bytes.len() as u64).sum();
        let total_chunks: u64 = planned
            .iter()
            .map(|file| chunk_bounds(file.bytes.len(), policy.chunk_size).len() as u64)
            .sum();

        let sender = Slot::new(
            receiver.transfer_id.clone(),
            Side::Send,
            TransportKind::Memory,
            planned.len(),
            Arc::clone(&self.clock),
        );
        let handle = TransferHandle::new(Arc::clone(&sender));

        sender.start();
        sender.set_totals(total_bytes, total_chunks);
        sender.moving();

        receiver.slot.start();
        receiver.slot.set_totals(total_bytes, total_chunks);
        receiver.slot.moving();

        self.publish(&sender, Side::Send, Terminal::Completed, false);
        self.publish(&receiver.slot, Side::Receive, Terminal::Completed, false);

        match self.run(&sender, &receiver, &planned, &policy).await {
            Ok(files) => {
                let sent = Outcome::Sent(SentOutcome {
                    transfer_id: receiver.transfer_id.clone(),
                    bytes: total_bytes,
                    chunks: total_chunks,
                    files: planned.len(),
                });
                let received = Outcome::Received(ReceivedOutcome {
                    transfer_id: receiver.transfer_id.clone(),
                    bytes: total_bytes,
                    chunks: total_chunks,
                    files,
                });

                sender.finish(Ok(sent));
                receiver.slot.finish(Ok(received));

                self.publish(&sender, Side::Send, Terminal::Completed, true);
                self.publish(&receiver.slot, Side::Receive, Terminal::Completed, true);
            }

            Err(error) => {
                let terminal = if error.kind() == vilsend_core::ErrorKind::Cancelled {
                    Terminal::Cancelled
                } else {
                    Terminal::Failed
                };

                sender.finish(Err(error.clone()));
                receiver.slot.finish(Err(error.clone()));

                self.publish(&sender, Side::Send, terminal, true);
                self.publish(&receiver.slot, Side::Receive, terminal, true);
            }
        }

        Ok(handle)
    }

    /// Moves every chunk, stopping at a chunk boundary if the transfer was
    /// cancelled.
    ///
    /// **Only the receiver's cancellation is reachable.** The sender's handle
    /// does not exist until `do_send` returns, and it returns when this loop is
    /// done — so `SenderHandle::cancel` can never reach a running in-memory
    /// transfer, and checking for it here would be a branch that is never
    /// taken. The receiving handle, by contrast, is handed out by `receive`
    /// *before* the sender exists, which makes it the one a caller can act on
    /// while the transfer is in flight.
    ///
    /// `yield_once` between chunks is what makes even that possible: without it
    /// this future never returns `Pending`, so a `cancel` polled beside it
    /// could not run until the transfer was already over, and the whole
    /// contract would be a fiction.
    async fn run(
        &self,
        sender: &Arc<Slot>,
        receiver: &Arc<Pending>,
        planned: &[PlannedFile],
        policy: &Policy,
    ) -> Result<Vec<ReceivedFile>, VilsendError> {
        let mut files_done = 0usize;

        for file in planned {
            let bounds = chunk_bounds(file.bytes.len(), policy.chunk_size);
            let expected_chunks = bounds.len() as u64;

            for (index, (offset, length)) in bounds.iter().copied().enumerate() {
                yield_once().await;

                if receiver.slot.is_cancel_requested() {
                    return Err(VilsendError::Cancelled);
                }

                let data = &file.bytes[offset..offset + length];
                let digest = Sha256::digest(data);

                let accepted = receiver.accept_chunk(
                    &file.relative_path,
                    index as u64,
                    expected_chunks,
                    data,
                    digest.as_slice(),
                    receiver.policy.verify_chunks,
                )?;

                if accepted {
                    sender.confirm(length as u64);
                    receiver.slot.confirm(length as u64);
                }

                self.publish(sender, Side::Send, Terminal::Completed, false);
                self.publish(&receiver.slot, Side::Receive, Terminal::Completed, false);
            }

            files_done += 1;
            sender.set_files_done(files_done);
            receiver.slot.set_files_done(files_done);
        }

        receiver.finalize(&self.sink)
    }

    async fn do_receive(&self, request: ReceiveRequest) -> Result<TransferHandle, VilsendError> {
        let policy = request.effective_policy(&self.policy)?;

        let transfer_id = self.next_id();
        let slot = Slot::new(
            transfer_id.clone(),
            Side::Receive,
            TransportKind::Memory,
            0,
            Arc::clone(&self.clock),
        );

        {
            // A listener cancelled before a sender arrives must stop waiting
            // for one, or its `wait` would never resolve and the link would
            // hold it forever.
            let link = Arc::clone(&self.link);
            let peer = request.peer.clone();
            let id = transfer_id.clone();

            slot.set_cancel_hook(Arc::new(move || link.remove(&peer, &id)));
        }

        self.link.push(
            request.peer,
            Arc::new(Pending {
                transfer_id,
                destination: request.destination,
                policy,
                slot: Arc::clone(&slot),
                assembly: Mutex::new(Assembly::default()),
            }),
        );

        Ok(TransferHandle::new(slot))
    }

    /// Publishes a progress or terminal event for one side of the transfer.
    ///
    /// The payload is `vilsend_core::TransferProgress` — the *wire* shape — built
    /// with the same arithmetic the shipped app uses, because ADR-0012 gives the
    /// product one event vocabulary and the SDK does not get a second one.
    fn publish(&self, slot: &Arc<Slot>, side: Side, terminal: Terminal, final_event: bool) {
        let sample = slot.progress();
        let elapsed_secs = slot.elapsed_nanos() as f64 / 1_000_000_000.0;

        let payload = match side {
            Side::Send => upload_progress(
                sample.transfer_id.as_str(),
                sample.bytes_confirmed,
                sample.total_bytes,
                elapsed_secs,
                sample.status.clone(),
            ),
            Side::Receive => download_progress(
                sample.transfer_id.as_str(),
                sample.bytes_confirmed,
                sample.total_bytes,
                elapsed_secs,
                sample.status.clone(),
            ),
        };

        if !final_event {
            self.events
                .emit(DomainEvent::TransferProgress { progress: payload });

            return;
        }

        self.events.emit(match terminal {
            Terminal::Completed => DomainEvent::TransferCompleted { progress: payload },
            Terminal::Failed => DomainEvent::TransferFailed { progress: payload },
            Terminal::Cancelled => DomainEvent::TransferCancelled { progress: payload },
        });
    }
}

impl Backend for MemoryBackend {
    fn send<'a>(
        &'a self,
        request: SendRequest,
    ) -> BoxFuture<'a, Result<TransferHandle, VilsendError>> {
        Box::pin(self.do_send(request))
    }

    fn receive<'a>(
        &'a self,
        request: ReceiveRequest,
    ) -> BoxFuture<'a, Result<TransferHandle, VilsendError>> {
        Box::pin(self.do_receive(request))
    }

    fn auth_state(&self) -> AuthState {
        // There is no session, and there cannot be one: a session needs a
        // credential store and a network, and `in_memory()` may touch neither.
        // Phase 6 gives the native backend a real answer; this one's answer is
        // structural.
        AuthState::SignedOut
    }

    fn events(&self) -> EventStream {
        self.events.subscribe()
    }
}

/// `(offset, length)` for each chunk of a file.
///
/// An empty file is **one** empty chunk, not zero chunks — the same `max(1)`
/// `crates/desktop/src/transfer/chunker.rs` applies, and the reason a zero-byte
/// file survives a transfer instead of vanishing.
fn chunk_bounds(total: usize, chunk_size: usize) -> Vec<(usize, usize)> {
    if total == 0 {
        return vec![(0, 0)];
    }

    (0..total.div_ceil(chunk_size))
        .map(|index| {
            let offset = index * chunk_size;

            (offset, (total - offset).min(chunk_size))
        })
        .collect()
}

/// `relative_path` under `destination`.
fn join(destination: &Destination, relative_path: &str) -> String {
    if destination.is_root() {
        return relative_path.to_owned();
    }

    format!(
        "{}/{}",
        destination.as_str().trim_end_matches('/'),
        relative_path
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;

    fn backend() -> MemoryBackend {
        MemoryBackend::new(
            MemoryFiles::new(),
            MemoryFiles::new(),
            Arc::new(FakeClock::new()),
            EventBus::new(None),
            Policy::default(),
        )
    }

    #[test]
    fn a_zero_byte_file_is_one_empty_chunk() {
        assert_eq!(chunk_bounds(0, 1024), vec![(0, 0)]);
    }

    #[test]
    fn a_file_that_divides_evenly_produces_no_trailing_empty_chunk() {
        assert_eq!(chunk_bounds(2048, 1024), vec![(0, 1024), (1024, 1024)]);
    }

    #[test]
    fn a_file_that_does_not_divide_evenly_produces_a_short_last_chunk() {
        assert_eq!(
            chunk_bounds(2500, 1024),
            vec![(0, 1024), (1024, 1024), (2048, 452)]
        );
    }

    #[test]
    fn the_chunk_bounds_cover_every_byte_exactly_once() {
        for total in [0usize, 1, 1023, 1024, 1025, 4097] {
            let bounds = chunk_bounds(total, 1024);

            let covered: usize = bounds.iter().map(|(_, length)| length).sum();

            assert_eq!(covered, total, "total {total}");

            for (index, (offset, _)) in bounds.iter().enumerate() {
                assert_eq!(*offset, index * 1024, "total {total}, chunk {index}");
            }
        }
    }

    #[test]
    fn a_path_is_joined_under_its_destination() {
        assert_eq!(join(&Destination::root(), "a.txt"), "a.txt");
        assert_eq!(
            join(&Destination::named("downloads"), "a.txt"),
            "downloads/a.txt"
        );
        assert_eq!(
            join(&Destination::named("downloads/"), "a/b.txt"),
            "downloads/a/b.txt"
        );
    }

    #[test]
    fn the_backend_hands_out_ids_in_a_deterministic_order() {
        let backend = backend();

        assert_eq!(backend.next_id().as_str(), "memory-0");
        assert_eq!(backend.next_id().as_str(), "memory-1");
    }

    #[test]
    fn the_in_memory_backend_has_no_session() {
        // Structural, not a policy: a session needs a credential store and a
        // network, and `in_memory()` may touch neither.
        assert_eq!(backend().auth_state(), AuthState::SignedOut);
    }

    #[test]
    fn a_chunk_whose_digest_does_not_match_is_refused() {
        let pending = pending(&Policy::default());

        let error = pending
            .accept_chunk(
                "a.txt",
                0,
                1,
                b"real",
                Sha256::digest(b"forged").as_slice(),
                true,
            )
            .expect_err("a forged chunk must not be accepted");

        assert_eq!(error.kind(), vilsend_core::ErrorKind::IntegrityMismatch);
        assert!(error.to_string().contains("a.txt"));
    }

    #[test]
    fn a_chunk_with_the_right_digest_is_accepted() {
        let pending = pending(&Policy::default());

        assert!(pending
            .accept_chunk(
                "a.txt",
                0,
                1,
                b"real",
                Sha256::digest(b"real").as_slice(),
                true
            )
            .expect("an honest chunk is accepted"));
    }

    #[test]
    fn verifying_can_be_turned_off_by_the_receivers_policy() {
        // The receiving side owns the check, so it is the receiving side's
        // policy that decides whether it happens.
        let pending = pending(&Policy {
            verify_chunks: false,
            ..Policy::default()
        });

        assert!(pending
            .accept_chunk(
                "a.txt",
                0,
                1,
                b"real",
                Sha256::digest(b"forged").as_slice(),
                false
            )
            .expect("the check was turned off"));
    }

    #[test]
    fn a_file_with_a_missing_chunk_is_refused_rather_than_written_short() {
        let pending = pending(&Policy::default());
        let sink = MemoryFiles::new();

        pending
            .accept_chunk(
                "a.txt",
                0,
                3,
                b"aaa",
                Sha256::digest(b"aaa").as_slice(),
                true,
            )
            .expect("accepted");

        let error = pending.finalize(&sink).expect_err("two chunks are missing");

        assert_eq!(error.kind(), vilsend_core::ErrorKind::IntegrityMismatch);
        assert!(sink.is_empty(), "nothing partial reached the sink");
    }

    #[test]
    fn a_file_with_a_hole_in_its_indices_is_refused_even_when_the_count_matches() {
        // 0 and 2 have the right *count* for 2 expected chunks and a hole where
        // 1 should be. Only walking the indices catches it.
        let pending = pending(&Policy::default());
        let sink = MemoryFiles::new();

        for (index, data) in [(0u64, b"aaa"), (2, b"ccc")] {
            pending
                .accept_chunk(
                    "a.txt",
                    index,
                    2,
                    data,
                    Sha256::digest(data).as_slice(),
                    true,
                )
                .expect("accepted");
        }

        assert!(pending.finalize(&sink).is_err());
        assert!(sink.is_empty());
    }

    #[test]
    fn a_complete_file_is_assembled_in_index_order_and_written_once() {
        let pending = pending(&Policy::default());
        let sink = MemoryFiles::new();

        // Deliberately out of order.
        for (index, data) in [(1u64, &b"world"[..]), (0, &b"hello "[..])] {
            pending
                .accept_chunk(
                    "a.txt",
                    index,
                    2,
                    data,
                    Sha256::digest(data).as_slice(),
                    true,
                )
                .expect("accepted");
        }

        let received = pending.finalize(&sink).expect("complete");

        assert_eq!(received.len(), 1);
        assert_eq!(received[0].path, "a.txt");
        assert_eq!(received[0].bytes, 11);
        assert!(received[0].verified);
        assert_eq!(sink.get("a.txt"), Some(b"hello world".to_vec()));
    }

    #[test]
    fn re_sending_a_chunk_does_not_count_it_twice() {
        let pending = pending(&Policy::default());
        let sink = MemoryFiles::new();

        let first = pending
            .accept_chunk(
                "a.txt",
                0,
                1,
                b"aaa",
                Sha256::digest(b"aaa").as_slice(),
                true,
            )
            .expect("accepted");
        let again = pending
            .accept_chunk(
                "a.txt",
                0,
                1,
                b"aaa",
                Sha256::digest(b"aaa").as_slice(),
                true,
            )
            .expect("accepted");

        assert!(first);
        assert!(!again, "the second copy is not new");

        let received = pending.finalize(&sink).expect("complete");

        assert_eq!(received[0].bytes, 3, "three bytes, not six");
    }

    #[test]
    fn a_received_file_lands_under_its_destination() {
        let pending = pending_with_destination(&Policy::default(), Destination::named("inbox"));
        let sink = MemoryFiles::new();

        pending
            .accept_chunk(
                "docs/a.txt",
                0,
                1,
                b"x",
                Sha256::digest(b"x").as_slice(),
                true,
            )
            .expect("accepted");
        pending.finalize(&sink).expect("complete");

        assert_eq!(sink.get("inbox/docs/a.txt"), Some(b"x".to_vec()));
    }

    fn pending(policy: &Policy) -> Pending {
        pending_with_destination(policy, Destination::root())
    }

    fn pending_with_destination(policy: &Policy, destination: Destination) -> Pending {
        Pending {
            transfer_id: TransferId::new("memory-0"),
            destination,
            policy: policy.clone(),
            slot: Slot::new(
                TransferId::new("memory-0"),
                Side::Receive,
                TransportKind::Memory,
                1,
                Arc::new(FakeClock::new()),
            ),
            assembly: Mutex::new(Assembly::default()),
        }
    }

    #[test]
    fn the_link_is_first_in_first_out_and_empties_when_drained() {
        let link = Link::default();
        let peer = PeerRef::from("memory");

        link.push(peer.clone(), Arc::new(pending(&Policy::default())));
        link.push(peer.clone(), Arc::new(pending(&Policy::default())));

        assert_eq!(link.waiting_on(&peer), 2);

        assert!(link.take(&peer).is_some());
        assert_eq!(link.waiting_on(&peer), 1);

        assert!(link.take(&peer).is_some());
        assert_eq!(link.waiting_on(&peer), 0);

        assert!(link.take(&peer).is_none(), "a drained link has no route");
    }

    #[test]
    fn a_cancelled_listener_is_removed_from_the_link() {
        let link = Link::default();
        let peer = PeerRef::from("memory");
        let id = TransferId::new("memory-0");

        link.push(peer.clone(), Arc::new(pending(&Policy::default())));
        link.remove(&peer, &id);

        assert_eq!(link.waiting_on(&peer), 0);
    }

    #[test]
    fn removing_a_listener_that_is_not_there_is_not_an_error() {
        let link = Link::default();

        link.remove(&PeerRef::from("nobody"), &TransferId::new("memory-9"));

        assert_eq!(link.waiting_on(&PeerRef::from("nobody")), 0);
    }
}
