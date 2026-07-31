use crate::{models::progress::TransferProgress, transfer::state::UploadState};
use std::sync::atomic::Ordering;
pub fn make(state: &UploadState) -> TransferProgress {
    let elapsed = state.started_at.elapsed().as_secs_f64().max(0.001);
    let uploaded = state.uploaded_bytes.load(Ordering::Relaxed);
    let speed = uploaded as f64 / elapsed;
    let remaining = state.total_bytes.saturating_sub(uploaded);
    TransferProgress {
        transfer_id: state.transfer_id.clone(),
        uploaded_bytes: uploaded,
        total_bytes: state.total_bytes,
        percentage: if state.total_bytes == 0 {
            100.0
        } else {
            uploaded as f64 * 100.0 / state.total_bytes as f64
        },
        speed,
        eta: if speed > 0.0 {
            Some((remaining as f64 / speed).ceil() as u64)
        } else {
            None
        },
        status: state.status.lock().unwrap().clone(),
    }
}
