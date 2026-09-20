use crate::transfer::state::UploadState;
use std::sync::atomic::Ordering;
use vilsend_core::TransferProgress;

/// Samples an upload into a progress payload.
///
/// The arithmetic moved to `vilsend_core::upload_progress`, which takes the
/// elapsed time as an argument so that it can be tested against a fake clock
/// rather than a real `Instant`. The reads are kept in their original order:
/// the elapsed time is measured before the byte counter is read.
pub fn make(state: &UploadState) -> TransferProgress {
    let elapsed_secs = state.started_at.elapsed().as_secs_f64();
    let uploaded = state.uploaded_bytes.load(Ordering::Relaxed);

    vilsend_core::upload_progress(
        state.transfer_id.clone(),
        uploaded,
        state.total_bytes,
        elapsed_secs,
        state.status.lock().unwrap().clone(),
    )
}
