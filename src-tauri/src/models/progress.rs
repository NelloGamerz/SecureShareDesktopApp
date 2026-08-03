use serde::Serialize;
use std::sync::atomic::Ordering;

use crate::{models::transfer::TransferStatus, transfer::state::DownloadState};
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TransferProgress {
    pub transfer_id: String,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
    pub percentage: f64,
    pub speed: f64,
    pub eta: Option<u64>,
    pub status: TransferStatus,
}


pub fn make_download(state: &DownloadState) -> TransferProgress {
    let received = state.received_bytes.load(Ordering::Relaxed);

    let percentage = if state.total_bytes == 0 {
        0.0
    } else {
        (received as f64 / state.total_bytes as f64) * 100.0
    };

    let elapsed = state.started_at.elapsed().as_secs_f64();

    let speed = if elapsed > 0.0 {
        received as f64 / elapsed
    } else {
        0.0
    };

    let eta = if speed > 0.0 && state.total_bytes > received {
        Some(((state.total_bytes - received) as f64 / speed) as u64)
    } else {
        None
    };

    TransferProgress {
        transfer_id: state.transfer_id.clone(),
        uploaded_bytes: received,
        total_bytes: state.total_bytes,
        percentage,
        speed,
        eta,
        status: state.status.lock().unwrap().clone(),
    }
}