use tauri::{AppHandle, State};

use crate::{
    services::{CloudflaredService, KeyringService},
    state::cloudflared_state::Cloudflared,
};

#[tauri::command]
pub fn start_cloudflared_cmd(
    app: AppHandle,
    state: State<'_, Cloudflared>,
) -> Result<String, String> {
    let hostname = KeyringService::get_hostname(&app)
        .map_err(|e| format!("Failed to load Cloudflare hostname: {e}"))?;

    KeyringService::get_tunnel_token(&app)
        .map_err(|e| format!("Failed to load Cloudflare tunnel token: {e}"))?;

    CloudflaredService::start(&app, state, hostname)
}

#[tauri::command]
pub fn stop_cloudflared_cmd(state: State<Cloudflared>) -> Result<(), String> {
    CloudflaredService::stop(state)
}

#[tauri::command]
pub fn cloudflared_status(state: State<Cloudflared>) -> Result<bool, String> {
    CloudflaredService::is_active(state)
}
