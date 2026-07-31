use std::sync::Arc;

use tauri::State;

use crate::{
    // error::AppError,
    models::transfer::LocalTransferFile,
    services::local_transfer_file_service::LocalTransferFileService,
};

// #[tauri::command]
// pub async fn save_local_transfer_file(
//     service: State<'_, Arc<LocalTransferFileService>>,
//     file: LocalTransferFile,
// ) -> Result<(), String> {

//     service
//         .insert(file)
//         .await
//         .map_err(|e| e.to_string())
// }

#[tauri::command]
pub async fn save_local_transfer_file(
    service: State<'_, Arc<LocalTransferFileService>>,
    file: LocalTransferFile,
) -> Result<(), String> {
    tracing::info!(
        transfer_id = %file.transfer_id,
        path = %file.file_path,
        "save_local_transfer_file called"
    );

    service.insert(file).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_local_transfer_files(
    service: State<'_, Arc<LocalTransferFileService>>,
    transfer_id: String,
) -> Result<Vec<LocalTransferFile>, String> {
    service
        .get_by_transfer_id(&transfer_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_local_transfer_files(
    service: State<'_, Arc<LocalTransferFileService>>,
    transfer_id: String,
) -> Result<(), String> {
    service
        .delete_by_transfer_id(&transfer_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_local_transfer_file(
    service: State<'_, Arc<LocalTransferFileService>>,
    transfer_id: String,
    file_path: String,
) -> Result<(), String> {
    service
        .delete_file(&transfer_id, &file_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_local_transfer_exists(
    service: State<'_, Arc<LocalTransferFileService>>,
    transfer_id: String,
) -> Result<bool, String> {
    service
        .exists(&transfer_id)
        .await
        .map_err(|e| e.to_string())
}
