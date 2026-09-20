use std::fs;

use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::events::EventDispatcher;

fn update_flag_path(app: &AppHandle) -> std::path::PathBuf {
    app.path().app_config_dir().unwrap().join("pending_update")
}

pub async fn check_for_update(app: AppHandle, dispatcher: Arc<EventDispatcher>) {
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Updater initialization failed: {}", e);
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();

            tracing::info!("Update available: {}", version);

            let path = update_flag_path(&app);

            if let Err(e) = fs::write(path, &version) {
                tracing::error!("Failed to save update flag: {}", e);

                return;
            }

            if let Err(e) = dispatcher.emit_update_available(version).await {
                tracing::error!("Failed to emit update available event: {}", e);
            }
        }

        Ok(None) => {
            tracing::info!("No update available");
        }

        Err(e) => {
            tracing::error!("Update check failed: {}", e);
        }
    }
}

pub async fn handle_pending_update(app: AppHandle, dispatcher: Arc<EventDispatcher>) {
    let flag = update_flag_path(&app);

    // First launch:
    // only check update
    if !flag.exists() {
        check_for_update(app, dispatcher).await;

        return;
    }

    tracing::info!("Pending update detected");

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Updater initialization failed: {}", e);
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            tracing::info!("Downloading update {}", update.version);

            match update
                .download_and_install(|_chunk, _total| {}, || {})
                .await
            {
                Ok(_) => {
                    tracing::info!("Update installed successfully");

                    fs::remove_file(&flag).ok();

                    app.restart();
                }

                Err(e) => {
                    tracing::error!("Update installation failed: {}", e);
                }
            }
        }

        Ok(None) => {
            tracing::info!("Pending update no longer available");

            fs::remove_file(&flag).ok();
        }

        Err(e) => {
            tracing::error!("Update check failed: {}", e);
        }
    }
}
