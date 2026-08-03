use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::models::TransferMetadata;
use crate::transfer::manager::UploadManager;
use crate::websocket::WebSocketManager;
use crate::{error::AppError, transfer::manager::TransferEvent};
// use crate::websocket::ServerCommand;
pub use crate::websocket::{server_command::TransferRequestPayload, ServerCommand};

pub struct EventDispatcher {
    app_handle: Mutex<Option<AppHandle>>,
    upload_manager: Arc<UploadManager>,
    websocket_manager: Mutex<Option<Arc<WebSocketManager>>>,
}

impl EventDispatcher {
    pub fn new(
        upload_manager: Arc<UploadManager>,
        // websocket_manager: Arc<WebSocketManager>,
    ) -> Self {
        Self {
            app_handle: Mutex::new(None),
            upload_manager,
            websocket_manager: Mutex::new(None),
        }
    }

    pub fn attach(&self, app_handle: AppHandle) {
        if let Ok(mut handle) = self.app_handle.lock() {
            *handle = Some(app_handle);
        }
    }

    pub fn attach_websocket_manager(&self, manager: Arc<WebSocketManager>) {
        if let Ok(mut websocket) = self.websocket_manager.lock() {
            *websocket = Some(manager);
        }
    }

    pub async fn emit_transfer_request(
        &self,
        payload: TransferRequestPayload,
    ) -> Result<(), AppError> {
        let payload = serde_json::to_value(payload).map_err(AppError::from)?;

        self.emit("transfer-request", payload).await
    }

    // pub async fn emit_server_event(&self, event: ServerEvent) -> Result<(), AppError> {
    //     let payload = serde_json::to_value(event).map_err(AppError::from)?;

    //     self.emit("server-event", payload).await
    // }

    // pub async fn emit_payload(&self, payload: Value) -> Result<(), AppError> {
    //     if let Some(message_type) = payload.get("type").and_then(|v| v.as_str()) {
    //         match message_type {
    //             "start_transfer" => {
    //                 self.handle_start_transfer(payload.clone()).await?;
    //             }

    //             "cancel_transfer" => {
    //                 if let Some(id) = payload.get("transfer_id").and_then(|v| v.as_str()) {
    //                     self.upload_manager.cancel(id).await.map_err(|e| {
    //                         AppError::internal(format!("cancel transfer failed: {e}"))
    //                     })?;
    //                 }
    //             }

    //             _ => {
    //                 tracing::debug!("Ignoring command: {:?}", message_type);
    //             }
    //         }
    //     }

    //     self.emit("websocket-message", payload).await
    // }

    // async fn handle_start_transfer(&self, payload: Value) -> Result<(), AppError> {
    //     let metadata: TransferMetadata = serde_json::from_value(payload).map_err(AppError::from)?;

    //     self.upload_manager
    //         .start(metadata)
    //         .await
    //         .map_err(|e| AppError::internal(format!("upload start failed: {e}")))?;

    //     Ok(())
    // }

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

    async fn send_ws_message(&self, payload: Value) -> Result<(), AppError> {
        let manager = {
            let guard = self
                .websocket_manager
                .lock()
                .map_err(|_| AppError::internal("websocket manager lock poisoned"))?;

            guard.clone()
        };

        match manager {
            Some(manager) => manager.send_message(payload.to_string()).await,

            None => Err(AppError::not_connected()),
        }
    }

    pub async fn notify_transfer_completed(&self, transfer_id: &str) -> Result<(), AppError> {
        self.send_ws_message(serde_json::json!({
            "type": "TRANSFER_COMPLETED",
            "transferId": transfer_id
        }))
        .await
    }

    pub async fn notify_transfer_failed(
        &self,
        transfer_id: &str,
        reason: &str,
    ) -> Result<(), AppError> {
        self.send_ws_message(serde_json::json!({
            "type": "TRANSFER_FAILED",
            "transferId": transfer_id,
            "reason": reason
        }))
        .await
    }

    // pub async fn listen_transfer_events(
    //     self: Arc<Self>,
    //     mut rx: tokio::sync::mpsc::UnboundedReceiver<TransferEvent>,
    // ) {
    //     while let Some(event) = rx.recv().await {
    //         match event {
    //             TransferEvent::Completed(id) => {
    //                 let _ = self.notify_transfer_completed(&id).await;
    //             }

    //             TransferEvent::Failed(id, reason) => {
    //                 let _ = self.notify_transfer_failed(&id, &reason).await;
    //             }
    //         }
    //     }
    // }

    pub async fn listen_transfer_events(
        self: Arc<Self>,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<TransferEvent>,
    ) {
        while let Some(event) = rx.recv().await {
            match event {
                TransferEvent::Completed(id) => {
                    if let Err(e) = self.notify_transfer_completed(&id).await {
                        tracing::error!("failed sending transfer complete websocket event: {}", e);
                    }
                }

                TransferEvent::Failed(id, reason) => {
                    if let Err(e) = self.notify_transfer_failed(&id, &reason).await {
                        tracing::error!("failed sending transfer failed websocket event: {}", e);
                    }
                }
            }
        }
    }

    pub async fn emit_update_available(&self, version: String) -> Result<(), AppError> {
        let payload = serde_json::json!({
            "type": "update-available",
            "version": version
        });

        self.emit("update-available", payload).await
    }
}
