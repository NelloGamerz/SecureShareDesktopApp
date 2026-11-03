use crate::{
    models::transfer::{TransferMetadata, TransferStatusResponse},
    transfer::manager::UploadManager,
};
use std::sync::Arc;
use tauri::State;
#[tauri::command]
pub async fn start_transfer(
    manager: State<'_, Arc<UploadManager>>,
    metadata: TransferMetadata,
) -> std::result::Result<TransferStatusResponse, String> {
    manager.start(metadata).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn pause_transfer(
    manager: State<'_, Arc<UploadManager>>,
    transfer_id: String,
) -> std::result::Result<TransferStatusResponse, String> {
    manager.pause(&transfer_id).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn resume_transfer(
    manager: State<'_, Arc<UploadManager>>,
    transfer_id: String,
) -> std::result::Result<TransferStatusResponse, String> {
    manager
        .resume(&transfer_id)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn cancel_transfer(
    manager: State<'_, Arc<UploadManager>>,
    transfer_id: String,
) -> std::result::Result<TransferStatusResponse, String> {
    manager
        .cancel(&transfer_id)
        .await
        .map_err(|e| e.to_string())
}
/// Probes this machine's transports for the accept request.
///
/// Returns `None` when no manifest should be sent (nothing usable, or the
/// encoded manifest would exceed the backend's limit). The probe is bounded, so
/// accept cannot hang on it, and it never returns an error the UI must handle.
#[tauri::command]
pub async fn build_transport_manifest() -> Option<serde_json::Value> {
    crate::transfer::transport_manifest::build_manifest().await
}

#[tauri::command]
pub fn get_transfer_status(
    manager: State<'_, Arc<UploadManager>>,
    transfer_id: String,
) -> std::result::Result<TransferStatusResponse, String> {
    manager.status(&transfer_id).map_err(|e| e.to_string())
}
