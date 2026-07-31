pub mod session;
pub mod chunk;
pub mod file_info;
pub mod progress;
pub mod transfer;

pub use session::Session;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error(String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ServerEvent {
    ConnectionStatus(ConnectionStatus),
    Message(String),
    Error(String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub device_name: String,
    pub device_identifier: String,
    pub device_type: String,
    pub operating_system: String,
    pub app_version: String,
}

pub use transfer::{
    ConnectionType,
    TransferMetadata,
    // LocalTransferFile,
    // TransferStatus,
    // TransferStatusResponse,
};
