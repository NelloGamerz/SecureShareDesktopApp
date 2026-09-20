pub mod session;
pub mod chunk;
pub mod file_info;
pub mod progress;
pub mod transfer;

pub use session::Session;

/// Re-exported so the rest of the shell keeps its existing import path.
/// The type, and its wire encoding, now live in `vilsend-core`.
pub use vilsend_core::ConnectionStatus;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub device_name: String,
    pub device_identifier: String,
    pub device_type: String,
    pub operating_system: String,
    pub app_version: String,
}

pub use transfer::{ConnectionType, TransferMetadata};
