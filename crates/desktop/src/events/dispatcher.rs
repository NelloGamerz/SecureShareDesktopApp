use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::models::TransferMetadata;
use crate::transfer::manager::UploadManager;
use crate::transfer::transport_manifest::{self, TransportManifest};
use vilsend_core::TransportKind;
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

    /// Tells the webview which path the sender chose.
    ///
    /// Emitted only for a `START_TRANSFER` that carried a manifest, so an old
    /// receiver produces no event that did not exist before.
    pub async fn emit_transport_selected(
        &self,
        transfer_id: &str,
        transport: &str,
    ) -> Result<(), AppError> {
        let payload = serde_json::json!({
            "type": "transfer-transport",
            "transferId": transfer_id,
            "transport": transport,
        });

        self.emit("transfer-transport", payload).await
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

    /// Notifies the webview that the desktop session changed.
    ///
    /// Carries no token material — React reads the token through the
    /// `get_auth_token` command when it needs one for a request.
    pub async fn emit_auth_state(
        &self,
        is_authenticated: bool,
        user_id: Option<String>,
    ) -> Result<(), AppError> {
        let payload = serde_json::json!({
            "type": "auth-state",
            "isAuthenticated": is_authenticated,
            "userId": user_id
        });
        self.emit("auth-state-changed", payload).await
    }

    /// Reports a failed sign-in attempt to the webview so the UI can stop
    /// waiting and show an error.
    pub async fn emit_auth_error(&self, message: String) -> Result<(), AppError> {
        let payload = serde_json::json!({
            "type": "auth-error",
            "message": message
        });
        self.emit("auth-error", payload).await
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
                // Renamed so it does not shadow the module of the same name.
                transport_manifest: raw_manifest,
            } => {
                /*
                 * The manifest is optional. Parsing is deliberately separate
                 * from reading the command so a malformed one costs the
                 * manifest and nothing else — the command still starts over the
                 * tunnel with the fields it arrived with.
                 */
                let manifest_present = raw_manifest.is_some();
                // Kept for the report: this is exactly what the backend sent.
                let manifest_for_report = raw_manifest.clone();

                let selected = match raw_manifest {
                    Some(value) => match serde_json::from_value::<TransportManifest>(value) {
                        Ok(manifest) => transport_manifest::select(&manifest).await,
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "malformed transport manifest; using the tunnel path"
                            );
                            None
                        }
                    },
                    None => None,
                };

                // No manifest, or nothing in it this build can dial: keep the
                // endpoint exactly as it arrived, which is what the sender did
                // before manifests existed.
                let endpoint = selected
                    .as_ref()
                    .and_then(|choice| choice.direct_endpoint.clone())
                    .unwrap_or(endpoint);

                if manifest_present {
                    let kind = selected
                        .as_ref()
                        .map(|choice| choice.kind)
                        .unwrap_or(TransportKind::Tunnel);

                    self.emit_transport_selected(
                        &transfer_id,
                        transport_manifest::transport_label(kind),
                    )
                    .await
                    .ok();
                }

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
                    transport: selected
                        .as_ref()
                        .map(|choice| transport_manifest::transport_label(choice.kind).to_string())
                        .or_else(|| manifest_present.then(|| "Tunnel".to_string())),
                    transport_manifest: manifest_for_report,
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
