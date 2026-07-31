use crate::models::transfer::{ConnectionType, TransferStatus, TransferStatusResponse};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Mutex,
    },
    time::Instant,
};
use x25519_dalek::PublicKey;

pub struct UploadState {
    pub transfer_id: String,
    pub transfer_key: [u8; 32],
    pub sender_ephemeral_public_key: PublicKey,
    pub endpoint: String,
    pub network_type: ConnectionType,
    pub total_bytes: u64,
    pub uploaded_bytes: AtomicU64,
    pub total_chunks: u64,
    pub uploaded_chunks: AtomicU64,
    pub paused: AtomicBool,
    pub cancelled: AtomicBool,
    pub retry_count: AtomicU32,
    pub max_retries: u32,
    pub status: std::sync::Mutex<TransferStatus>,
    pub started_at: Instant,
}
impl UploadState {
    pub fn snapshot(&self) -> TransferStatusResponse {
        TransferStatusResponse {
            transfer_id: self.transfer_id.clone(),
            receiver_endpoint: self.endpoint.clone(),
            network_type: self.network_type.clone(),
            total_bytes: self.total_bytes,
            uploaded_bytes: self.uploaded_bytes.load(Ordering::Relaxed),
            total_chunks: self.total_chunks,
            uploaded_chunks: self.uploaded_chunks.load(Ordering::Relaxed),
            retry_count: self.retry_count.load(Ordering::Relaxed),
            status: self.status.lock().expect("status lock poisoned").clone(),
        }
    }
}

pub struct DownloadState {
    pub transfer_id: String,

    pub total_bytes: u64,
    pub received_bytes: AtomicU64,

    pub total_chunks: u64,
    pub received_chunks: AtomicU64,

    pub paused: AtomicBool,
    pub cancelled: AtomicBool,

    pub started_at: Instant,

    pub status: Mutex<TransferStatus>,
}
