use std::sync::Arc;

use tauri::Emitter;
// use tauri::State;
use tauri::{AppHandle, State};

use crate::app::AppState;
use crate::error::AppError;
use crate::models::ConnectionStatus;
use crate::models::DeviceInfo;
use crate::services::keyring_service::KeyringService;
// use crate::services::SecureStorage;

#[tauri::command]
pub async fn login(
    state: State<'_, Arc<AppState>>,
    token: String,
    device_info: DeviceInfo,
    // user_id: Option<String>,
) -> Result<(), AppError> {
    state.auth_service.login(token, device_info).await
}

#[tauri::command]
pub async fn logout(state: State<'_, Arc<AppState>>) -> Result<(), AppError> {
    state.auth_service.logout().await
}

// #[tauri::command]
// pub async fn start_websocket(state: State<'_, Arc<AppState>>) -> Result<(), AppError> {
//     state.websocket_service.start().await
// }

// #[tauri::command]
// pub async fn start_websocket(state: State<'_, Arc<AppState>>) -> Result<(), AppError> {
//     println!("START_WEBSOCKET COMMAND HIT");

//     let result = state.websocket_service.start().await;

//     println!("START_WEBSOCKET RESULT: {:?}", result);

//     result
// }

#[tauri::command]
pub async fn start_websocket(
    state: State<'_, Arc<AppState>>,
    device_info: DeviceInfo,
) -> Result<(), AppError> {
    println!("START_WEBSOCKET COMMAND HIT");

    println!(
        "Device: {} | Identifier: {} | Type: {} | OS: {} | Version: {}",
        device_info.device_name,
        device_info.device_identifier,
        device_info.device_type,
        device_info.operating_system,
        device_info.app_version
    );

    let result = state.websocket_service.start(device_info).await;

    println!("START_WEBSOCKET RESULT: {:?}", result);

    result
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, Arc<AppState>>,
    payload: String,
) -> Result<(), AppError> {
    state.websocket_service.send_message(payload).await
}

#[tauri::command]
pub async fn get_connection_status(
    state: State<'_, Arc<AppState>>,
) -> Result<ConnectionStatus, AppError> {
    state.websocket_service.status().await
}

#[tauri::command]
pub fn save_tunnel_token(app: AppHandle, token: String) -> Result<(), String> {
    KeyringService::save_tunnel_token(&app, &token)
}

#[tauri::command]
pub fn get_tunnel_token(app: AppHandle) -> Result<String, String> {
    KeyringService::get_tunnel_token(&app)
}

#[tauri::command]
pub fn delete_tunnel_token(app: AppHandle) -> Result<(), String> {
    KeyringService::delete_tunnel_token(&app)
}

#[tauri::command]
pub fn save_tunnel_hostname(app: AppHandle, hostname: String) -> Result<(), String> {
    KeyringService::save_hostname(&app, &hostname)
}

#[tauri::command]
pub fn get_tunnel_hostname(app: AppHandle) -> Result<String, String> {
    KeyringService::get_hostname(&app)
}

#[tauri::command]
pub fn delete_tunnel_hostname(app: AppHandle) -> Result<(), String> {
    KeyringService::delete_hostname(&app)
}

#[tauri::command]
pub fn clear_all(app: AppHandle) -> Result<(), String> {
    KeyringService::clear_all(&app)
}

#[tauri::command]
async fn get_clerk_token(app: tauri::AppHandle) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    // store tx somewhere shared (state)
    app.emit("request-clerk-token", ()).unwrap();

    let token = rx.await.map_err(|_| "Token request failed")?;

    Ok(token)
}

#[tauri::command]
pub async fn update_auth_token(
    state: State<'_, Arc<AppState>>,
    token: String,
) -> Result<(), AppError> {
    state.auth_service.update_token(token).await
}
