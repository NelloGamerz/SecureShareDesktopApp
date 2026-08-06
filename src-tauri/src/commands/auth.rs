use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use tauri::Emitter;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;

use crate::app::AppState;
use crate::error::AppError;
use crate::models::ConnectionStatus;
use crate::models::DeviceInfo;
use crate::services::keyring_service::KeyringService;
// use crate::services::SecureStorage;

const DEFAULT_DOWNLOAD_LOCATION_KEY: &str = "default_download_location";

fn default_download_dir(app: &AppHandle) -> Result<String, String> {
    app.path()
        .download_dir()
        .map(|path: std::path::PathBuf| path.to_string_lossy().to_string())
        .map_err(|error| format!("failed to resolve downloads folder: {error}"))
}

fn resolve_download_location(app: &AppHandle) -> Result<String, String> {
    let store = app.store("settings.json").map_err(|error| {
        format!("failed to open settings store: {error}")
    })?;

    if let Some(value) = store.get(DEFAULT_DOWNLOAD_LOCATION_KEY) {
        if let Some(path) = value.as_str() {
            if !path.is_empty() {
                return Ok(path.to_string());
            }
        }
    }

    default_download_dir(app)
}

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
pub fn get_default_download_location(app: AppHandle) -> Result<String, String> {
    resolve_download_location(&app)
}

#[tauri::command]
pub fn set_default_download_location(app: AppHandle, path: String) -> Result<(), String> {
    let destination = if path.trim().is_empty() {
        default_download_dir(&app)?
    } else {
        path
    };

    let resolved = PathBuf::from(&destination);
    std::fs::create_dir_all(&resolved).map_err(|error| {
        format!("failed to create download directory: {error}")
    })?;

    let store = app.store("settings.json").map_err(|error| {
        format!("failed to open settings store: {error}")
    })?;
    store.set(
        DEFAULT_DOWNLOAD_LOCATION_KEY.to_string(),
        json!(resolved.to_string_lossy().to_string()),
    );
    store.save().map_err(|error| {
        format!("failed to persist download location: {error}")
    })?;

    Ok(())
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
