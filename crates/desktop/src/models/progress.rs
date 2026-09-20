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
pub fn make_download(state: &DownloadState) -> TransferProgress {
    let received = state.received_bytes.load(Ordering::Relaxed);
    let elapsed_secs = state.started_at.elapsed().as_secs_f64();

    vilsend_core::download_progress(
        state.transfer_id.clone(),
        received,
        state.total_bytes,
        elapsed_secs,
        state.status.lock().unwrap().clone(),
    )
}
