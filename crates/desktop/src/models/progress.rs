use std::sync::atomic::Ordering;

use crate::transfer::state::DownloadState;

/// Re-exported so the rest of the shell keeps its existing import path.
/// The type, and its wire encoding, now live in `vilsend-core`.
pub use vilsend_core::TransferProgress;

/// Samples a download into a progress payload.
///
/// The arithmetic moved to `vilsend_core::download_progress`, which takes the
/// elapsed time as an argument so that it can be tested against a fake clock
/// rather than a real `Instant`. The reads are kept in their original order.
/// The byte count is clamped at the total for the same reason the upload side's
/// is: a chunk the sender offered again after it had already been written is not
/// new progress, and nothing the receiver counts may push the percentage past
/// 100.
pub fn make_download(state: &DownloadState) -> TransferProgress {
    let received = state
        .received_bytes
        .load(Ordering::Relaxed)
        .min(state.total_bytes);
    let elapsed_secs = state.started_at.elapsed().as_secs_f64();

    vilsend_core::download_progress(
        state.transfer_id.clone(),
        received,
        state.total_bytes,
        elapsed_secs,
        state.status.lock().unwrap().clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::transfer::TransferStatus;
    use std::sync::{
        atomic::{AtomicBool, AtomicU64},
        Mutex,
    };

    fn download(received: u64, total: u64) -> DownloadState {
        DownloadState {
            transfer_id: "t-1".into(),
            total_bytes: total,
            received_bytes: AtomicU64::new(received),
            received_chunks: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            started_at: std::time::Instant::now(),
            status: Mutex::new(TransferStatus::Downloading),
            report: crate::transfer::report::Telemetry::new(
                crate::transfer::report::Role::Receiver,
            ),
            phases: Mutex::new(crate::transfer::report::PhaseTimings::default()),
        }
    }

    #[test]
    fn a_recovered_download_reports_the_progress_it_has() {
        // What the receiver seeds at session start is reported as it stands,
        // rather than as an empty transfer.
        let progress = make_download(&download(500, 1_000));

        assert_eq!(progress.uploaded_bytes, 500);
        assert_eq!(progress.total_bytes, 1_000);
        assert_eq!(progress.percentage, 50.0);
    }

    #[test]
    fn a_download_never_reports_more_than_a_hundred_percent() {
        // A sender that announces less than it sends, or a count that was
        // inflated, must not produce a percentage past 100.
        let progress = make_download(&download(1_500, 1_000));

        assert_eq!(progress.uploaded_bytes, 1_000, "clamped to the total");
        assert_eq!(progress.percentage, 100.0);
    }
}
