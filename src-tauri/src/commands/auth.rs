use std::{path::PathBuf, sync::Arc};

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager, State, Url};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_store::StoreExt;

use crate::app::AppState;
use crate::error::AppError;
use crate::models::ConnectionStatus;
use crate::models::DeviceInfo;
use crate::services::keyring_service::KeyringService;
use crate::services::oauth_service::DesktopAuthConfig;

const DEFAULT_DOWNLOAD_LOCATION_KEY: &str = "default_download_location";

fn default_download_dir(app: &AppHandle) -> Result<String, String> {
    app.path()
        .download_dir()
        .map(|path: std::path::PathBuf| path.to_string_lossy().to_string())
        .map_err(|error| format!("failed to resolve downloads folder: {error}"))
}

fn resolve_download_location(app: &AppHandle) -> Result<String, String> {
    let store = app
        .store("settings.json")
        .map_err(|error| format!("failed to open settings store: {error}"))?;

    if let Some(value) = store.get(DEFAULT_DOWNLOAD_LOCATION_KEY) {
        if let Some(path) = value.as_str() {
            if !path.is_empty() {
                return Ok(path.to_string());
            }
        }
    }

    default_download_dir(app)
}

/// Non-sensitive view of the desktop session, returned to the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub is_authenticated: bool,
    pub user_id: Option<String>,
    /// True while a browser sign-in is waiting on its callback.
    pub is_authenticating: bool,
}

/// Starts a system-browser Authorization Code + PKCE sign-in.
///
/// The authorization code and token exchange never touch the webview: Rust
/// owns the PKCE verifier, performs the exchange, and stores the token.
#[tauri::command]
pub async fn start_desktop_auth(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    config: DesktopAuthConfig,
    mode: Option<String>,
) -> Result<(), AppError> {
    let mode = mode.unwrap_or_else(|| "sign_in".to_string());

    let authorize_url = state.oauth_service.begin(&config, &mode).await?;

    // Retain the endpoints so a later refresh does not need the frontend to
    // re-send configuration.
    state
        .oauth_service
        .remember_endpoints(config.token_endpoint(), config.client_id.clone())
        .await;

    app.opener()
        .open_url(authorize_url, None::<&str>)
        .map_err(|error| {
            // A failed launch leaves the pending flow behind; drop it so the
            // user is not locked out of retrying.
            tracing::warn!(
                target: "auth",
                event = "browser_open_failed",
                error = %error,
                "could not open the system browser for sign-in"
            );

            AppError::internal("Could not open your browser to sign in")
        })?;

    tracing::info!(
        target: "auth",
        event = "auth_flow_started",
        "desktop sign-in opened in the system browser"
    );

    Ok(())
}

/// Abandons an in-flight sign-in, e.g. when the user closes the waiting screen.
#[tauri::command]
pub async fn cancel_desktop_auth(state: State<'_, Arc<AppState>>) -> Result<(), AppError> {
    state.oauth_service.cancel().await
}

/// Reads the current session without triggering any network activity.
#[tauri::command]
pub async fn get_auth_status(state: State<'_, Arc<AppState>>) -> Result<AuthStatus, AppError> {
    let (is_authenticated, user_id, is_authenticating) = state.oauth_service.snapshot().await;

    Ok(AuthStatus {
        is_authenticated,
        user_id,
        is_authenticating,
    })
}

/// Returns a token for the `Authorization` header, refreshing it when it is
/// close to expiry.
///
/// This is the only path by which the token reaches the webview, and it is
/// deliberately request-scoped rather than pushed into React state.
#[tauri::command]
pub async fn get_auth_token(state: State<'_, Arc<AppState>>) -> Result<Option<String>, AppError> {
    state.oauth_service.token_for_request().await
}

#[tauri::command]
pub async fn logout(state: State<'_, Arc<AppState>>) -> Result<(), AppError> {
    state.auth_service.logout().await
}

/// Handles a `vilsend://auth/callback` deep link.
///
/// Shared by the running-app listener and the launch-time URL so both paths
/// behave identically.
pub async fn handle_auth_callback(app: AppHandle, url: Url) {
    // Ignore deep links that are not authorization responses, so unrelated
    // links never surface a spurious error.
    let is_auth_response = url
        .query_pairs()
        .any(|(key, _)| key == "code" || key == "error");

    if !is_auth_response {
        tracing::debug!(
            target: "auth",
            event = "deep_link_ignored",
            path = %url.path(),
            "ignoring deep link that is not an OAuth callback"
        );
        return;
    }

    let state = app.state::<Arc<AppState>>().inner().clone();

    match state.oauth_service.complete(&url).await {
        Ok(Some(_outcome)) => {
            if let Err(error) = state.auth_service.notify_signed_in().await {
                tracing::error!(
                    target: "auth",
                    event = "auth_state_emit_failed",
                    error = %error,
                    "signed in but could not notify the UI"
                );
            }
        }
        Ok(None) => {}
        Err(error) => {
            let message = error.to_string();

            // `AppError` for auth failures is constructed from Clerk's
            // `error_description` and never includes the code, verifier or
            // tokens.
            tracing::warn!(
                target: "auth",
                event = "auth_callback_failed",
                error = %message,
                "desktop sign-in callback could not be completed"
            );

            let _ = state.event_service.emit_auth_error(message).await;
        }
    }
}

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

/// Stops the control-plane WebSocket.
///
/// The webview has always invoked `stop_websocket`, on sign-out and on
/// teardown, but no command of that name was registered — the call failed at
/// the IPC boundary and the socket kept running after the session that
/// authenticated it was gone.
#[tauri::command]
pub async fn stop_websocket(state: State<'_, Arc<AppState>>) -> Result<(), AppError> {
    state.websocket_service.stop().await
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
    std::fs::create_dir_all(&resolved)
        .map_err(|error| format!("failed to create download directory: {error}"))?;

    let store = app
        .store("settings.json")
        .map_err(|error| format!("failed to open settings store: {error}"))?;
    store.set(
        DEFAULT_DOWNLOAD_LOCATION_KEY.to_string(),
        json!(resolved.to_string_lossy().to_string()),
    );
    store
        .save()
        .map_err(|error| format!("failed to persist download location: {error}"))?;

    Ok(())
}

#[tauri::command]
pub fn clear_all(app: AppHandle) -> Result<(), String> {
    KeyringService::clear_all(&app)
}
