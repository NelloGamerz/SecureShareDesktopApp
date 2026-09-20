use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ServerCommand {
    #[serde(rename = "START_TRANSFER")]
    StartTransfer {
        transfer_id: String,
        // transfer_key: String,
        receiver_public_key: String,
        endpoint: String,
        // file_paths: Vec<String>,
        chunk_size: Option<usize>,
        concurrency: Option<usize>,
        max_retries: Option<u32>,
    },

    #[serde(rename = "CANCEL_TRANSFER")]
    CancelTransfer { transfer_id: String },

    #[serde(rename = "TRANSFER_REQUEST")]
    TransferRequest { payload: TransferRequestPayload },

    #[serde(rename = "PONG")]
    Pong,

    #[serde(rename = "PING")]
    Ping,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransferRequestPayload {
    #[serde(rename = "transferId")]
    pub transfer_id: String,

    #[serde(rename = "senderDeviceId")]
    pub sender_device_id: String,

    #[serde(rename = "receiverDeviceId")]
    pub receiver_device_id: String,

    #[serde(rename = "fileName")]
    pub file_name: String,

    #[serde(rename = "fileSize")]
    pub file_size: u64,
}
