//! The handle a running transfer is held by, and what it resolves to.
//!
//! `04-sdk-cli-mobile-build-plan.md` §2.2 specifies the methods. Two details of
//! its specification are load-bearing enough to repeat here:
//!
//! - `wait` "resolves when the transfer reaches a terminal state", and
//! - "Dropping the handle **cancels** the transfer. Documented, not
//!   incidental."
//!
//! The second is why [`TransferHandle`] implements `Drop`, and why `wait` is
//! careful to mark itself detached before it returns — otherwise the ordinary
//! `handle.wait().await` would cancel the transfer it just waited for on the
//! way out.

use std::sync::Arc;

use vilsend_core::{TransferId, VilsendError};

use crate::progress::Progress;
use crate::state::Slot;

/// What a transfer produced.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The transfer was sent.
    Sent(SentOutcome),
    /// The transfer was received.
    Received(ReceivedOutcome),
}

impl Outcome {
    /// The transfer this outcome is about.
    pub fn transfer_id(&self) -> &TransferId {
        match self {
            Self::Sent(sent) => &sent.transfer_id,
            Self::Received(received) => &received.transfer_id,
        }
    }

    /// Total bytes confirmed by the far end.
    pub fn bytes(&self) -> u64 {
        match self {
            Self::Sent(sent) => sent.bytes,
            Self::Received(received) => received.bytes,
        }
    }

    /// Total chunks confirmed by the far end.
    pub fn chunks(&self) -> u64 {
        match self {
            Self::Sent(sent) => sent.chunks,
            Self::Received(received) => received.chunks,
        }
    }
}

/// The result of a completed send.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentOutcome {
    /// The transfer this outcome is about.
    pub transfer_id: TransferId,
    /// Bytes the far end confirmed.
    pub bytes: u64,
    /// Chunks the far end confirmed.
    pub chunks: u64,
    /// How many files were sent.
    pub files: usize,
}

/// The result of a completed receive.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedOutcome {
    /// The transfer this outcome is about.
    pub transfer_id: TransferId,
    /// Bytes written.
    pub bytes: u64,
    /// Chunks written.
    pub chunks: u64,
    /// What arrived, in the order it was sent.
    pub files: Vec<ReceivedFile>,
}

/// One file that was received.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedFile {
    /// The path the file was written to, relative to the destination.
    pub path: String,
    /// How many bytes it holds.
    pub bytes: u64,
    /// Whether every chunk matched the hash the sending side computed for it.
    ///
    /// Always `true` today: a chunk that fails its hash fails the transfer, so
    /// a `ReceivedFile` that exists at all is one whose integrity was checked.
    /// The field is here so that a future backend with integrity at another
    /// layer — or none, under a policy that turns the check off — has an honest
    /// way to say so instead of implying it.
    pub verified: bool,
}

/// A handle on one running or finished transfer.
#[derive(Debug)]
pub struct TransferHandle {
    slot: Arc<Slot>,
    /// Set by `wait` before it returns, so the `Drop` below does not cancel a
    /// transfer that has already been waited on.
    detached: bool,
}

impl TransferHandle {
    pub(crate) fn new(slot: Arc<Slot>) -> Self {
        Self {
            slot,
            detached: false,
        }
    }

    /// The transfer's identifier. Stable for the life of the transfer and
    /// shared by both ends of it.
    pub fn id(&self) -> TransferId {
        self.slot.id().clone()
    }

    /// The current sample.
    ///
    /// Does not block: it reads the last known state rather than waiting for
    /// the next event.
    pub async fn progress(&self) -> Progress {
        self.slot.progress()
    }

    /// Asks the transfer to stop where it is.
    ///
    /// Fails with [`vilsend_core::ErrorKind::InvalidInput`] if the transfer has
    /// already finished.
    pub async fn pause(&self) -> Result<(), VilsendError> {
        self.slot.pause()
    }

    /// Asks a paused transfer to carry on.
    pub async fn resume(&self) -> Result<(), VilsendError> {
        self.slot.resume()
    }

    /// Asks the transfer to stop and be discarded.
    pub async fn cancel(&self) -> Result<(), VilsendError> {
        self.slot.cancel()
    }

    /// Resolves when the transfer reaches a terminal state.
    ///
    /// Consumes the handle, so the `Drop` that would otherwise cancel the
    /// transfer does not fire.
    pub async fn wait(mut self) -> Result<Outcome, VilsendError> {
        self.detached = true;

        self.slot.wait().await
    }
}

impl Drop for TransferHandle {
    fn drop(&mut self) {
        if self.detached {
            return;
        }

        // §2.2: "Dropping the handle cancels the transfer. Documented, not
        // incidental." A finished transfer refuses the cancel, and that
        // refusal is the correct outcome rather than an error to report — the
        // caller dropped a handle on a transfer that no longer exists, which
        // is exactly what they asked for.
        let _ = self.slot.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use crate::progress::TransportKind;
    use crate::state::Side;

    fn slot(side: Side) -> Arc<Slot> {
        Slot::new(
            TransferId::new("t-1"),
            side,
            TransportKind::Memory,
            1,
            Arc::new(FakeClock::new()),
        )
    }

    #[test]
    fn the_handle_reports_the_slots_identifier() {
        let handle = TransferHandle::new(slot(Side::Send));

        assert_eq!(handle.id(), TransferId::new("t-1"));
    }

    #[test]
    fn progress_reads_without_waiting_for_the_transfer_to_end() {
        let slot = slot(Side::Send);

        slot.start();
        slot.set_totals(10, 1);
        slot.confirm(5);

        let handle = TransferHandle::new(Arc::clone(&slot));

        assert_eq!(
            futures::executor::block_on(handle.progress()).bytes_confirmed,
            5
        );
    }

    #[test]
    fn dropping_a_live_handle_cancels_the_transfer() {
        // §2.2: documented, not incidental.
        let slot = slot(Side::Send);

        {
            let _handle = TransferHandle::new(Arc::clone(&slot));
        }

        assert!(
            slot.is_cancel_requested(),
            "dropping the handle must cancel the transfer"
        );
    }

    #[test]
    fn waiting_does_not_cancel_the_transfer_it_waited_for() {
        // The trap this whole arrangement exists to avoid: `wait` consumes the
        // handle, so its `Drop` runs at the end of `wait`, and without the
        // detached flag every successful `wait` would report success and then
        // cancel.
        let slot = slot(Side::Send);

        let handle = TransferHandle::new(Arc::clone(&slot));

        slot.finish(Ok(Outcome::Sent(SentOutcome {
            transfer_id: TransferId::new("t-1"),
            bytes: 1,
            chunks: 1,
            files: 1,
        })));

        let outcome = futures::executor::block_on(handle.wait()).expect("it completed");

        assert_eq!(outcome.bytes(), 1);
        assert!(
            !slot.is_cancel_requested(),
            "waiting must not count as abandoning"
        );
    }

    #[test]
    fn every_outcome_accessor_reads_through_to_the_payload() {
        let sent = Outcome::Sent(SentOutcome {
            transfer_id: TransferId::new("t-sent"),
            bytes: 10,
            chunks: 2,
            files: 3,
        });

        assert_eq!(sent.transfer_id(), &TransferId::new("t-sent"));
        assert_eq!(sent.bytes(), 10);
        assert_eq!(sent.chunks(), 2);

        let received = Outcome::Received(ReceivedOutcome {
            transfer_id: TransferId::new("t-received"),
            bytes: 20,
            chunks: 4,
            files: vec![ReceivedFile {
                path: "a.txt".into(),
                bytes: 20,
                verified: true,
            }],
        });

        assert_eq!(received.transfer_id(), &TransferId::new("t-received"));
        assert_eq!(received.bytes(), 20);
        assert_eq!(received.chunks(), 4);
    }

    #[test]
    fn a_terminal_handles_control_methods_report_invalid_input() {
        let slot = slot(Side::Send);

        slot.finish(Ok(Outcome::Sent(SentOutcome {
            transfer_id: TransferId::new("t-1"),
            bytes: 1,
            chunks: 1,
            files: 1,
        })));

        let handle = TransferHandle::new(Arc::clone(&slot));

        futures::executor::block_on(async {
            for result in [
                handle.pause().await,
                handle.resume().await,
                handle.cancel().await,
            ] {
                assert_eq!(
                    result.expect_err("a finished transfer refuses").kind(),
                    vilsend_core::ErrorKind::InvalidInput
                );
            }
        });
    }
}
