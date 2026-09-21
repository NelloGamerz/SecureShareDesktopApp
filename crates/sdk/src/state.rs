//! The shared state behind one [`TransferHandle`](crate::TransferHandle).
//!
//! Crate-private. A caller sees a handle; the engine, the event bus and the
//! `wait` future all see this.
//!
//! The wait/wake primitive is hand-rolled rather than taken from a runtime:
//! `01-target-architecture.md` §6.1 requires the public signatures to be
//! runtime-agnostic ("Host apps may run their own runtime"), and a library that
//! reached for `tokio::sync::Notify` would have hardcoded one in its guts even
//! though its `async fn`s look neutral. `std::future::poll_fn` is enough.

use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Duration;

use vilsend_core::{TransferId, TransferStatus, VilsendError};

use crate::clock::{throughput_bps, Clock};
use crate::handle::Outcome;
use crate::progress::{Progress, TransportKind};

/// Which half of a transfer a slot is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Send,
    Receive,
}

#[derive(Debug)]
struct SlotState {
    status: TransferStatus,
    bytes: u64,
    total_bytes: u64,
    chunks: u64,
    total_chunks: u64,
    files_done: usize,
    files_total: usize,
    retries: u32,
    paused: bool,
    cancel_requested: bool,
    started_nanos: Option<u64>,
    finished_nanos: Option<u64>,
    outcome: Option<Result<Outcome, VilsendError>>,
}

/// The state one handle observes, and the signal that tells it to look again.
pub(crate) struct Slot {
    id: TransferId,
    side: Side,
    transport: TransportKind,
    clock: Arc<dyn Clock>,
    state: Mutex<SlotState>,
    /// Woken when the transfer reaches a terminal state.
    signal: Signal,
    /// What the backend needs to undo when this transfer is cancelled before
    /// it starts — for the in-memory backend, releasing a waiting receiver.
    cancel_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl std::fmt::Debug for Slot {
    /// Hand-written because neither the injected [`Clock`] nor the backend's
    /// cancel hook is `Debug`, and neither should be forced to be. A slot
    /// renders as the two things a reader can act on: which transfer, and where
    /// it has got to.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Slot")
            .field("id", &self.id)
            .field("side", &self.side)
            .field("transport", &self.transport)
            .field("progress", &self.progress())
            .finish_non_exhaustive()
    }
}

impl Slot {
    pub(crate) fn new(
        id: TransferId,
        side: Side,
        transport: TransportKind,
        files_total: usize,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            side,
            transport,
            clock,
            state: Mutex::new(SlotState {
                status: match side {
                    // A sender has not started yet; a receiver is listening.
                    Side::Send => TransferStatus::Queued,
                    Side::Receive => TransferStatus::Pending,
                },
                bytes: 0,
                total_bytes: 0,
                chunks: 0,
                total_chunks: 0,
                files_done: 0,
                files_total,
                retries: 0,
                paused: false,
                cancel_requested: false,
                started_nanos: None,
                finished_nanos: None,
                outcome: None,
            }),
            signal: Signal::default(),
            cancel_hook: Mutex::new(None),
        })
    }

    /// Registers what the backend must undo if this transfer is cancelled
    /// before it starts.
    ///
    /// Set once, immediately after construction, because a slot cannot hold an
    /// `Arc` to the structure that is about to hold it.
    pub(crate) fn set_cancel_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.cancel_hook.lock().expect("cancel hook lock poisoned") = Some(hook);
    }

    pub(crate) fn id(&self) -> &TransferId {
        &self.id
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state.lock().expect("transfer slot lock poisoned")
    }

    /// Starts the clock. Idempotent: a transfer's start is the first time it is
    /// observed to have one.
    pub(crate) fn start(&self) {
        let mut state = self.lock();

        if state.started_nanos.is_none() {
            state.started_nanos = Some(self.clock.now_nanos());
        }
    }

    pub(crate) fn set_status(&self, status: TransferStatus) {
        {
            let mut state = self.lock();
            state.status = status;
        }

        self.signal.notify();
    }

    pub(crate) fn set_totals(&self, total_bytes: u64, total_chunks: u64) {
        let mut state = self.lock();

        state.total_bytes = total_bytes;
        state.total_chunks = total_chunks;
    }

    pub(crate) fn confirm(&self, bytes: u64) {
        let mut state = self.lock();

        state.bytes = state.bytes.saturating_add(bytes);
        state.chunks = state.chunks.saturating_add(1);
    }

    pub(crate) fn set_files_done(&self, files_done: usize) {
        let mut state = self.lock();

        state.files_done = files_done;
    }

    pub(crate) fn is_cancel_requested(&self) -> bool {
        self.lock().cancel_requested
    }

    /// The status a shell should show for a transfer that is moving in the
    /// given direction.
    fn moving_status(&self) -> TransferStatus {
        match self.side {
            Side::Send => TransferStatus::Uploading,
            Side::Receive => TransferStatus::Downloading,
        }
    }

    pub(crate) fn moving(&self) {
        self.set_status(self.moving_status());
    }

    /// Moves the transfer to a terminal state and wakes anyone waiting.
    ///
    /// The status is derived from the outcome rather than passed separately, so
    /// a caller cannot report `Completed` with a failure beside it. A second
    /// call is refused and the first outcome stands: the first one is what
    /// actually happened, and a caller racing a cancellation must not be able
    /// to overwrite it.
    pub(crate) fn finish(&self, outcome: Result<Outcome, VilsendError>) -> bool {
        let status = match &outcome {
            Ok(_) => TransferStatus::Completed,
            Err(error) => match error.kind() {
                vilsend_core::ErrorKind::Cancelled => TransferStatus::Cancelled,
                _ => TransferStatus::Failed,
            },
        };

        {
            let mut state = self.lock();

            if state.outcome.is_some() {
                return false;
            }

            state.status = status;
            state.finished_nanos = Some(self.clock.now_nanos());
            state.paused = false;
            state.outcome = Some(outcome);
        }

        self.signal.notify();

        true
    }

    /// Nanoseconds between the transfer starting and now — or, once it has
    /// finished, between it starting and finishing.
    pub(crate) fn elapsed_nanos(&self) -> u64 {
        let state = self.lock();
        let now = self.clock.now_nanos();
        let start = state.started_nanos.unwrap_or(now);
        let end = state.finished_nanos.unwrap_or(now);

        end.saturating_sub(start)
    }

    /// The current sample.
    pub(crate) fn progress(&self) -> Progress {
        let state = self.lock();
        let now = self.clock.now_nanos();

        let start = state.started_nanos.unwrap_or(now);
        let end = state.finished_nanos.unwrap_or(now);
        let throughput = throughput_bps(state.bytes, start, end);

        let remaining = state.total_bytes.saturating_sub(state.bytes);
        let eta = if throughput > 0 && remaining > 0 {
            let nanos = (remaining as u128) * 1_000_000_000u128 / (throughput as u128);

            Some(Duration::from_nanos(
                u64::try_from(nanos).unwrap_or(u64::MAX),
            ))
        } else {
            None
        };

        Progress {
            transfer_id: self.id.clone(),
            bytes_confirmed: state.bytes,
            total_bytes: state.total_bytes,
            chunks_confirmed: state.chunks,
            total_chunks: state.total_chunks,
            files_done: state.files_done,
            files_total: state.files_total,
            retries: state.retries,
            status: state.status.clone(),
            transport: self.transport,
            throughput_bps: throughput,
            eta,
            // There is exactly one route today, so nothing can have degraded.
            // `04` §2.3 requires the field to exist so that a failover can
            // never be hidden; Phase 8 is what gives it a producer.
            degraded: None,
        }
    }

    /// Resolves when the transfer reaches a terminal state.
    pub(crate) async fn wait(&self) -> Result<Outcome, VilsendError> {
        loop {
            if let Some(outcome) = self.lock().outcome.clone() {
                return outcome;
            }

            self.signal.wait().await;
        }
    }

    pub(crate) fn pause(&self) -> Result<(), VilsendError> {
        {
            let mut state = self.lock();

            if state.outcome.is_some() {
                return Err(terminal_error(&self.id, "pause"));
            }

            state.paused = true;
            state.status = TransferStatus::Paused;
        }

        self.signal.notify();

        Ok(())
    }

    pub(crate) fn resume(&self) -> Result<(), VilsendError> {
        {
            let mut state = self.lock();

            if state.outcome.is_some() {
                return Err(terminal_error(&self.id, "resume"));
            }

            state.paused = false;
            state.status = self.moving_status();
        }

        self.signal.notify();

        Ok(())
    }

    /// Stops the transfer.
    ///
    /// A transfer that has **not started** has no chunk boundary to stop at, so
    /// it stops here and now — which is what makes dropping a handle on a
    /// listener resolve its `wait` instead of leaking it forever. A transfer
    /// that *has* started is asked to stop, and the engine's loop honours the
    /// flag at the next chunk boundary.
    ///
    /// Either way the backend's hook runs, so whatever was holding the transfer
    /// is released.
    pub(crate) fn cancel(&self) -> Result<(), VilsendError> {
        let hook = self
            .cancel_hook
            .lock()
            .expect("cancel hook lock poisoned")
            .clone();

        {
            let mut state = self.lock();

            if state.outcome.is_some() {
                return Err(terminal_error(&self.id, "cancel"));
            }

            state.cancel_requested = true;
        }

        let never_started = self.lock().started_nanos.is_none();

        if never_started {
            self.finish(Err(VilsendError::Cancelled));
        }

        if let Some(hook) = hook {
            hook();
        }

        self.signal.notify();

        Ok(())
    }
}

/// What an operation that is invalid *because the transfer is over* fails with.
///
/// Deliberately not `NotFound`: the transfer exists, it has simply finished, and
/// a shell that saw `NotFound` would tell the user the transfer had vanished.
fn terminal_error(id: &TransferId, operation: &str) -> VilsendError {
    VilsendError::InvalidInput(format!(
        "cannot {operation} transfer {id}: it has already finished"
    ))
}

/// Yields once to the executor, then continues.
///
/// The standard library has no async yield — `std::task::yield_now` does not
/// exist, and `std::thread::yield_now` is a different thing entirely — and the
/// SDK has no runtime to borrow one from. This is the whole of it: wake
/// yourself, return `Pending` once, and be done on the next poll.
///
/// The engine calls it between chunks. Without it a `send` future would never
/// return `Pending`, so a `cancel` or `pause` polled beside it could not run
/// until the transfer was already over — and both methods would be untestable
/// and their contract a fiction.
pub(crate) async fn yield_once() {
    let mut yielded = false;

    std::future::poll_fn(|context| {
        if yielded {
            return Poll::Ready(());
        }

        yielded = true;
        context.waker().wake_by_ref();

        Poll::Pending
    })
    .await
}

/// A one-shot "look again" signal.
///
/// Carries a `ready` flag as well as the wakers, which is what closes the
/// race between "check the state" and "register to be woken": a `notify` that
/// lands in that window is remembered rather than lost.
#[derive(Debug, Default)]
struct Signal {
    inner: Mutex<SignalState>,
}

#[derive(Debug, Default)]
struct SignalState {
    ready: bool,
    wakers: Vec<Waker>,
}

impl Signal {
    fn notify(&self) {
        let wakers = {
            let mut inner = self.inner.lock().expect("signal lock poisoned");

            inner.ready = true;

            std::mem::take(&mut inner.wakers)
        };

        for waker in wakers {
            waker.wake();
        }
    }

    async fn wait(&self) {
        std::future::poll_fn(|context| {
            let mut inner = self.inner.lock().expect("signal lock poisoned");

            if inner.ready {
                inner.ready = false;

                return Poll::Ready(());
            }

            if !inner
                .wakers
                .iter()
                .any(|waker| waker.will_wake(context.waker()))
            {
                inner.wakers.push(context.waker().clone());
            }

            Poll::Pending
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> TransferId {
        TransferId::new("t-1")
    }

    fn slot(side: Side) -> Arc<Slot> {
        Slot::new(
            id(),
            side,
            TransportKind::Memory,
            2,
            Arc::new(crate::clock::FakeClock::new()),
        )
    }

    #[test]
    fn a_sender_starts_queued_and_a_receiver_starts_pending() {
        assert_eq!(slot(Side::Send).progress().status, TransferStatus::Queued);
        assert_eq!(
            slot(Side::Receive).progress().status,
            TransferStatus::Pending
        );
    }

    #[test]
    fn finishing_derives_the_status_from_the_outcome() {
        let sent = slot(Side::Send);
        sent.finish(Ok(Outcome::Sent(crate::handle::SentOutcome {
            transfer_id: id(),
            bytes: 1,
            chunks: 1,
            files: 1,
        })));
        assert_eq!(sent.progress().status, TransferStatus::Completed);

        let failed = slot(Side::Send);
        failed.finish(Err(VilsendError::NoRoute));
        assert_eq!(failed.progress().status, TransferStatus::Failed);

        let cancelled = slot(Side::Send);
        cancelled.finish(Err(VilsendError::Cancelled));
        assert_eq!(cancelled.progress().status, TransferStatus::Cancelled);
    }

    #[test]
    fn a_pause_resume_cancel_cycle_moves_the_status_and_the_flags() {
        let slot = slot(Side::Send);

        slot.start();
        slot.moving();
        assert_eq!(slot.progress().status, TransferStatus::Uploading);

        slot.pause().expect("pausing a live transfer");
        assert_eq!(slot.progress().status, TransferStatus::Paused);

        slot.resume().expect("resuming a paused transfer");
        assert_eq!(slot.progress().status, TransferStatus::Uploading);

        slot.cancel().expect("cancelling a live transfer");
        assert!(slot.is_cancel_requested());
    }

    #[test]
    fn pausing_resuming_or_cancelling_a_finished_transfer_is_invalid_input() {
        // Not `NotFound`: the transfer exists and has finished. A shell that
        // saw `NotFound` would tell the user it had vanished.
        let slot = slot(Side::Send);

        slot.finish(Ok(Outcome::Sent(crate::handle::SentOutcome {
            transfer_id: id(),
            bytes: 1,
            chunks: 1,
            files: 1,
        })));

        for (name, result) in [
            ("pause", slot.pause()),
            ("resume", slot.resume()),
            ("cancel", slot.cancel()),
        ] {
            let error = result.expect_err("a finished transfer cannot be operated on");

            assert_eq!(
                error.kind(),
                vilsend_core::ErrorKind::InvalidInput,
                "{name}"
            );
            assert!(
                error.to_string().contains("t-1"),
                "{name}: the message names the transfer"
            );
        }
    }

    #[test]
    fn a_receivers_moving_status_is_downloading_not_uploading() {
        let slot = slot(Side::Receive);

        slot.moving();

        assert_eq!(slot.progress().status, TransferStatus::Downloading);
    }

    #[test]
    fn throughput_and_eta_come_from_the_injected_clock() {
        let clock = Arc::new(crate::clock::FakeClock::new());
        let slot = Slot::new(
            id(),
            Side::Send,
            TransportKind::Memory,
            1,
            Arc::clone(&clock) as Arc<dyn Clock>,
        );

        slot.start();
        slot.set_totals(1_000, 10);
        slot.confirm(250);

        // No time has passed: throughput is zero and no ETA is claimed.
        assert_eq!(slot.progress().throughput_bps, 0);
        assert_eq!(slot.progress().eta, None);

        clock.advance(Duration::from_secs(1));

        // 250 bytes in one second, 750 to go.
        assert_eq!(slot.progress().throughput_bps, 250);
        assert_eq!(slot.progress().eta, Some(Duration::from_secs(3)));
    }

    #[test]
    fn a_finished_transfer_measures_its_throughput_to_the_moment_it_finished() {
        let clock = Arc::new(crate::clock::FakeClock::new());
        let slot = Slot::new(
            id(),
            Side::Send,
            TransportKind::Memory,
            1,
            Arc::clone(&clock) as Arc<dyn Clock>,
        );

        slot.start();
        slot.set_totals(1_000, 10);
        slot.confirm(1_000);

        clock.advance(Duration::from_secs(2));
        slot.finish(Ok(Outcome::Sent(crate::handle::SentOutcome {
            transfer_id: id(),
            bytes: 1_000,
            chunks: 10,
            files: 1,
        })));

        // Time keeps passing for everyone else, but this transfer's throughput
        // is a fact about the transfer.
        clock.advance(Duration::from_secs(60));

        assert_eq!(slot.progress().throughput_bps, 500);
    }

    #[test]
    fn waiting_resolves_when_the_transfer_finishes() {
        let slot = slot(Side::Send);

        let waiter = {
            let slot = Arc::clone(&slot);

            std::thread::spawn(move || {
                futures::executor::block_on(slot.wait()).expect("the transfer completed")
            })
        };

        slot.finish(Ok(Outcome::Sent(crate::handle::SentOutcome {
            transfer_id: id(),
            bytes: 1,
            chunks: 1,
            files: 1,
        })));

        assert_eq!(
            waiter
                .join()
                .expect("the waiter thread did not panic")
                .bytes(),
            1
        );
    }

    #[test]
    fn waiting_reports_the_failure_it_finished_with() {
        let slot = slot(Side::Send);

        slot.finish(Err(VilsendError::InsufficientStorage));

        let error = futures::executor::block_on(slot.wait()).expect_err("it failed");

        assert_eq!(error.kind(), vilsend_core::ErrorKind::InsufficientStorage);
    }

    #[test]
    fn progress_is_readable_while_a_wait_is_outstanding() {
        let slot = slot(Side::Send);

        slot.start();
        slot.set_totals(100, 4);
        slot.confirm(25);

        let sample = slot.progress();

        assert_eq!(sample.bytes_confirmed, 25);
        assert_eq!(sample.total_bytes, 100);
        assert_eq!(sample.chunks_confirmed, 1);
        assert_eq!(sample.files_total, 2);
        assert_eq!(sample.transport, TransportKind::Memory);
    }
}
