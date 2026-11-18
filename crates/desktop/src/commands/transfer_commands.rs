use crate::{
    models::transfer::{TransferMetadata, TransferStatusResponse},
    transfer::manager::UploadManager,
};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
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

/// Cancels a transfer this device is *receiving*.
///
/// The upload manager only knows transfers this device sends, so the receiving
/// half is cancelled through the receiver's own state: it stops the download,
/// drops its key and manifest, and removes the staged parts.
#[tauri::command]
pub async fn cancel_received_transfer(
    receiver: State<'_, crate::transfer::writer::ReceiverState>,
    transfer_id: String,
) -> std::result::Result<(), String> {
    receiver
        .cancel_download(&transfer_id, "cancelled by the user")
        .await
        .map_err(|error| error.to_string())?;

    Ok(())
}
/// Probes this machine's transports for the accept request.
///
/// Always returns a manifest, even when no transport is available: the backend
/// records a present-but-empty manifest differently from a missing one, which
/// is what the accept path needs. The probe is bounded, so accept cannot hang
/// on it, and it never returns an error the UI must handle.
#[tauri::command]
pub async fn build_transport_manifest() -> serde_json::Value {
    crate::transfer::transport_manifest::build_manifest().await
}

#[tauri::command]
pub fn get_transfer_status(
    manager: State<'_, Arc<UploadManager>>,
    transfer_id: String,
) -> std::result::Result<TransferStatusResponse, String> {
    manager.status(&transfer_id).map_err(|e| e.to_string())
}

/// Shows a received file in the system file manager, with the file selected.
///
/// It never launches the file itself: the user asked to be taken to where the
/// transfer landed. `reveal_item_in_dir` is the plugin's cross-platform
/// Explorer / Finder / file-manager "show in folder".
///
/// The name comes from the webview, so it is held to the same containment rule
/// as a sender's relative path. A file that is no longer there is reported as
/// such — the transfer record outlives what it wrote.
#[tauri::command]
pub fn open_received_file(app: AppHandle, file_name: String) -> std::result::Result<(), String> {
    if !crate::transfer::chunk_sink::is_contained(&file_name) {
        return Err("File not available".into());
    }

    let root = crate::commands::auth::resolve_download_location(&app)?;
    let path = std::path::Path::new(&root).join(&file_name);

    if !path.exists() {
        return Err(format!("File not available at {}", path.display()));
    }

    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|error| format!("could not open the file location: {error}"))
}
