use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransferStatus {
    Queued,
    Uploading,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Pending,
    Downloading,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConnectionType {
    Lan,
    Tunnel,
    REMOTE,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferMetadata {
    pub transfer_id: String,
    // pub transfer_key: String,
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
// #[serde(rename_all = "camelCase")]
pub struct LocalTransferFile {
    pub transfer_id: String,
    pub file_path: String,
    pub file_name: String,
    pub file_size: u64,
}