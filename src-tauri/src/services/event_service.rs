use std::sync::Arc;

use crate::error::AppError;
use crate::events::EventDispatcher;
// `ConnectionStatus` / `ServerEvent` are only referenced by the commented-out
// helpers below.
#[allow(unused_imports)]
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

    pub async fn emit_auth_state(
        &self,
        is_authenticated: bool,
        user_id: Option<String>,
    ) -> Result<(), AppError> {
        self.dispatcher
            .emit_auth_state(is_authenticated, user_id)
            .await
    }

    pub async fn emit_auth_error(&self, message: String) -> Result<(), AppError> {
        self.dispatcher.emit_auth_error(message).await
    }

    // pub async fn emit_message(&self, payload: serde_json::Value) -> Result<(), AppError> {
    //     self.dispatcher.emit_payload(payload).await
    // }
}
