use std::sync::Arc;

use crate::error::AppError;
use crate::events::EventDispatcher;
use crate::models::{ConnectionStatus, ServerEvent};

pub struct EventService {
    dispatcher: Arc<EventDispatcher>,
}

impl EventService {
    pub fn new(dispatcher: Arc<EventDispatcher>) -> Self {
        Self { dispatcher }
    }

    // pub async fn emit_connection_status(&self, status: ConnectionStatus) -> Result<(), AppError> {
    //     self.dispatcher
    //         .emit_server_event(ServerEvent::ConnectionStatus(status))
    //         .await
    // }

    pub async fn emit_auth_state(&self) -> Result<(), AppError> {
        self.dispatcher.emit_auth_state().await
    }

    // pub async fn emit_message(&self, payload: serde_json::Value) -> Result<(), AppError> {
    //     self.dispatcher.emit_payload(payload).await
    // }
}
