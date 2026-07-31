use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::error::AppError;
use crate::models::{ServerEvent, TransferMetadata};
use crate::transfer::manager::UploadManager;
// use crate::websocket::ServerCommand;
pub use crate::websocket::{ServerCommand, server_command::TransferRequestPayload};

pub struct EventDispatcher {
    app_handle: Mutex<Option<AppHandle>>,
    upload_manager: Arc<UploadManager>,
}

impl EventDispatcher {
    pub fn new(upload_manager: Arc<UploadManager>) -> Self {
        Self {
            app_handle: Mutex::new(None),
            upload_manager,
        }
    }

    pub fn attach(&self, app_handle: AppHandle) {
        if let Ok(mut handle) = self.app_handle.lock() {
            *handle = Some(app_handle);
        }
    }

    pub async fn emit_transfer_request(
        &self,
        payload: TransferRequestPayload,
    ) -> Result<(), AppError> {
        let payload = serde_json::to_value(payload).map_err(AppError::from)?;

        self.emit("transfer-request", payload).await
    }

    pub async fn emit_server_event(&self, event: ServerEvent) -> Result<(), AppError> {
        let payload = serde_json::to_value(event).map_err(AppError::from)?;

        self.emit("server-event", payload).await
    }

    pub async fn emit_payload(&self, payload: Value) -> Result<(), AppError> {
        if let Some(message_type) = payload.get("type").and_then(|v| v.as_str()) {
            match message_type {
                "start_transfer" => {
                    self.handle_start_transfer(payload.clone()).await?;
                }

                "cancel_transfer" => {
                    if let Some(id) = payload.get("transfer_id").and_then(|v| v.as_str()) {
                        self.upload_manager.cancel(id).await.map_err(|e| {
                            AppError::internal(format!("cancel transfer failed: {e}"))
                        })?;
                    }
                }

                _ => {
                    tracing::debug!("Ignoring command: {:?}", message_type);
                }
            }
        }

        self.emit("websocket-message", payload).await
    }

    async fn handle_start_transfer(&self, payload: Value) -> Result<(), AppError> {
        let metadata: TransferMetadata = serde_json::from_value(payload).map_err(AppError::from)?;

        self.upload_manager
            .start(metadata)
            .await
            .map_err(|e| AppError::internal(format!("upload start failed: {e}")))?;

        Ok(())
    }

    async fn emit(&self, event_name: &str, payload: Value) -> Result<(), AppError> {
        if let Some(handle) = self.app_handle.lock().ok().and_then(|guard| guard.clone()) {
            handle.emit(event_name, payload).map_err(AppError::from)?;

            return Ok(());
        }

        Err(AppError::internal(format!(
            "no tauri app handle attached for event: {event_name}"
        )))
    }

    pub async fn emit_auth_state(&self) -> Result<(), AppError> {
        let payload = serde_json::json!({
            "type": "auth-state"
            // "isAuthenticated": is_authenticated,
            // "userId": user_id
        });
        self.emit("auth-state-changed", payload).await
    }

    pub async fn emit_command(&self, command: ServerCommand) -> Result<(), AppError> {
        match command {
            ServerCommand::StartTransfer {
                transfer_id,
                // transfer_key,
                receiver_public_key,
                endpoint,
                // file_paths,
                chunk_size,
                concurrency,
                max_retries,
            } => {
                let metadata = TransferMetadata {
                    transfer_id,
                    // transfer_key,
                    receiver_public_key,
                    network_type: crate::models::ConnectionType::Lan,
                    endpoint,
                    auth_token: String::new(),
                    // file_paths,
                    chunk_size,
                    concurrency,
                    max_retries,
                };

                self.upload_manager
                    .start(metadata)
                    .await
                    .map_err(|e| AppError::internal(format!("upload start failed: {e}")))?;
            }

            ServerCommand::CancelTransfer { transfer_id } => {
                // self.upload_manager.cancel(&transfer_id).await?;
                self.upload_manager
                    .cancel(&transfer_id)
                    .await
                    .map_err(|e| AppError::internal(format!("cancel transfer failed: {e}")))?;
            }

            ServerCommand::TransferRequest { payload } => {
                tracing::info!("Received transfer request: {:?}", payload);

                self.emit_transfer_request(payload).await?;
            }

            _ => {
                tracing::debug!("Ignoring command: {:?}", command);
            }
        }

        Ok(())
    }
}
