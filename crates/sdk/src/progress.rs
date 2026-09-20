//! A progress sample, as `04-sdk-cli-mobile-build-plan.md` §2.3 defines it.
//!
//! **This is deliberately not `vilsend_core::TransferProgress`.** That type is
//! the *wire* payload the webview reads — seven `snake_case` fields, frozen by
//! the golden fixtures in `crates/core/tests/golden_payloads.rs`. This one is
//! the SDK's own view of the same transfer, and it is richer, because a library
//! consumer has no webview to render a percentage into and does have questions
//! a percentage cannot answer: which transport carried it, how many files are
//! done, and whether the route degraded underneath them.
//!
//! The two are not interchangeable and Phase 5 does not bridge them, because
//! nothing yet consumes both. The bridge is a mapping function, and it belongs
//! to whichever phase first makes the desktop shell call the SDK (§8).

use std::time::Duration;

use vilsend_core::{TransferId, TransferStatus};

/// Which kind of route carried (or is carrying) a transfer.
///
/// **Defined here rather than in `vilsend-core`**, which is a deviation from
/// `01-target-architecture.md` §4.1: the transport families are the
/// `Transport` port's vocabulary and Phase 8 owns it. It is here because
/// `Progress` needs to name one today and only one of the three exists —
/// there is no LAN transport and no tunnel in this crate's reach. Moving the
/// type down to the port, unchanged, is the first thing Phase 8 should do.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// Both ends in one process, over an in-memory link. The only kind the SDK
    /// can currently produce; see [`crate::VilsendBuilder::in_memory`].
    Memory,
    /// A peer reached over the local network. Phase 10.
    Lan,
    /// A peer reached through the tunnel. Phase 8.
    Tunnel,
}

/// Why a transfer is not using the route it started on.
///
/// `04-sdk-cli-mobile-build-plan.md` §2.3 is emphatic that this must be
/// surfaced: "Silently falling back from LAN to tunnel is exactly the behaviour
/// that makes downgrade attacks invisible". Nothing produces one yet — there is
/// no second route to fall back to — so every sample reports `None`, and a test
/// pins that it exists rather than letting the field quietly disappear.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DegradedReason {
    /// The preferred route was not reachable for this peer.
    PreferredRouteUnavailable,
    /// The transfer moved off the preferred route part-way through.
    SwitchedMidTransfer,
    /// The active route is failing and the transfer is being retried on it.
    Retrying,
}

/// One sample of a transfer's state.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The transfer this sample is about.
    pub transfer_id: TransferId,
    /// Chunks the far end has acknowledged, in bytes. "Confirmed" is the
    /// operative word: bytes handed to a transport that has not acknowledged
    /// them do not count, so this never leads the truth.
    pub bytes_confirmed: u64,
    /// Every byte the transfer covers, as planned.
    pub total_bytes: u64,
    /// Chunks the far end has acknowledged.
    pub chunks_confirmed: u64,
    /// Chunks the transfer is made of, as planned.
    pub total_chunks: u64,
    /// How many files have been fully confirmed.
    pub files_done: usize,
    /// How many files the transfer covers.
    pub files_total: usize,
    /// How many retries have been spent so far across the whole transfer.
    pub retries: u32,
    /// The lifecycle state.
    pub status: TransferStatus,
    /// Which kind of route is carrying it.
    pub transport: TransportKind,
    /// Confirmed bytes per second, or `0` when no time has passed yet.
    pub throughput_bps: u64,
    /// Time left, estimated from `throughput_bps`. `None` when throughput is
    /// unknown or nothing is left.
    pub eta: Option<Duration>,
    /// Whether the route degraded, and how. See [`DegradedReason`].
    pub degraded: Option<DegradedReason>,
}

impl Progress {
    /// The fraction confirmed, `0.0..=1.0`, or `None` when the total is not
    /// known yet.
    ///
    /// A transfer whose total is unknown reports `None` rather than `0.0` or
    /// `1.0`: `vilsend_core::progress` documents that the shipped app's two
    /// producers disagree about that case (upload says 100 %, download says
    /// 0 %), and neither answer is a fact about the transfer, so the SDK
    /// declines to pick one.
    pub fn fraction(&self) -> Option<f64> {
        if self.total_bytes == 0 {
            return None;
        }

        Some((self.bytes_confirmed as f64 / self.total_bytes as f64).min(1.0))
    }

    /// `true` once the transfer has reached a state it will not leave.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(bytes: u64, total: u64, status: TransferStatus) -> Progress {
        Progress {
            transfer_id: TransferId::new("t-1"),
            bytes_confirmed: bytes,
            total_bytes: total,
            chunks_confirmed: 1,
            total_chunks: 4,
            files_done: 0,
            files_total: 1,
            retries: 0,
            status,
            transport: TransportKind::Memory,
            throughput_bps: 0,
            eta: None,
            degraded: None,
        }
    }

    #[test]
    fn the_fraction_is_unknown_rather_than_one_when_the_total_is_unknown() {
        // The shipped app answers 100 % for an upload and 0 % for a download.
        // Neither is a fact, so the SDK says so.
        assert_eq!(sample(0, 0, TransferStatus::Uploading).fraction(), None);
        assert_eq!(sample(50, 0, TransferStatus::Uploading).fraction(), None);
    }

    #[test]
    fn the_fraction_is_clamped_to_one() {
        // More confirmed than expected is a real state (a file grew between the
        // scan and the send); reporting 1.4 would be a lie about the whole.
        assert_eq!(
            sample(150, 100, TransferStatus::Uploading).fraction(),
            Some(1.0)
        );
        assert_eq!(
            sample(50, 100, TransferStatus::Uploading).fraction(),
            Some(0.5)
        );
    }

    #[test]
    fn the_terminal_statuses_are_the_three_that_do_not_move() {
        assert!(sample(1, 1, TransferStatus::Completed).is_terminal());
        assert!(sample(1, 1, TransferStatus::Failed).is_terminal());
        assert!(sample(1, 1, TransferStatus::Cancelled).is_terminal());

        for status in [
            TransferStatus::Queued,
            TransferStatus::Uploading,
            TransferStatus::Paused,
            TransferStatus::Pending,
            TransferStatus::Downloading,
        ] {
            assert!(!sample(1, 1, status.clone()).is_terminal(), "{status:?}");
        }
    }

    #[test]
    fn the_degraded_field_exists_and_is_none_when_nothing_degraded() {
        // §2.3 requires the field to exist so that a failover is impossible to
        // hide. There is only one route today, so the only honest value is
        // `None` — and this test is what stops the field being deleted as
        // unused before Phase 8 gives it a producer.
        let sample = sample(1, 2, TransferStatus::Uploading);

        assert_eq!(sample.degraded, None);
        assert_eq!(sample.transport, TransportKind::Memory);
    }
}
