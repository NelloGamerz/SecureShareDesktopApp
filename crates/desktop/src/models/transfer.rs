use serde::{Deserialize, Serialize};

/// Re-exported so the rest of the shell keeps its existing import path.
/// The type, and its wire encoding, now live in `vilsend-core`.
pub use vilsend_core::TransferStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConnectionType {
    Lan,
    Tunnel,
    // The variant name is deliberately the serialised value. `rename_all` maps
    // it to `"REMOTE"` on the wire either way, but renaming it to satisfy a
    // style lint would put the wire format one serde release away from a
    // silent change, for no gain.
    #[allow(clippy::upper_case_acronyms)]
    REMOTE,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferMetadata {
    pub transfer_id: String,
    // Part of the `start_transfer` command's payload, so it is kept even
    // though nothing reads it: the command's shape is a public contract and
    // removing a field from it is a breaking change for the webview. The
    // sender in fact ignores this value and fetches the receiver's key from
    // `GET /transfer/public-key` instead — see the phase 1 report.
    #[allow(dead_code)]
    pub receiver_public_key: String,
    // pub receiver_id: String,
    pub network_type: ConnectionType,
    pub endpoint: String,
    pub auth_token: String,
    // pub file_paths: Vec<String>,
    pub chunk_size: Option<usize>,
    pub concurrency: Option<usize>,
    pub max_retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferStatusResponse {
    pub transfer_id: String,
    pub receiver_endpoint: String,
    pub network_type: ConnectionType,
    pub total_bytes: u64,
    pub uploaded_bytes: u64,
    pub total_chunks: u64,
    pub uploaded_chunks: u64,
    pub retry_count: u32,
    pub status: TransferStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTransferFile {
    pub transfer_id: String,
    pub file_path: String,
    pub file_name: String,
    pub file_size: u64,
}
