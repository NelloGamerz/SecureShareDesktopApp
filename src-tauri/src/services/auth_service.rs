use std::sync::Arc;

use crate::error::AppError;
use crate::models::DeviceInfo;
use crate::models::Session;
use crate::services::{EventService, WebSocketService};
use crate::state::AuthState;
use crate::utils::config::AppConfig;

pub struct AuthService {
    auth_state: Arc<AuthState>,
    websocket_service: Arc<WebSocketService>,
    event_service: Arc<EventService>,
    config: Arc<AppConfig>,
}

impl AuthService {
    pub fn new(
        auth_state: Arc<AuthState>,
        websocket_service: Arc<WebSocketService>,
        event_service: Arc<EventService>,
        config: Arc<AppConfig>,
    ) -> Self {
        Self {
            auth_state,
            websocket_service,
            event_service,
            config,
        }
    }

    pub async fn login(&self, token: String, device_info: DeviceInfo) -> Result<(), AppError> {
        // eprintln!("device info", device_info);
        let session = Session::new(token.clone());

        {
            let mut auth_token = self.auth_state.token.write().await;
            *auth_token = Some(token);
        }

        {
            let mut auth_session = self.auth_state.session.write().await;
            *auth_session = Some(session);
        }

        {
            let mut authenticated = self.auth_state.is_authenticated.write().await;
            *authenticated = true;
        }

        self.event_service.emit_auth_state().await?;

        tracing::info!(
            target: "auth_service",
            event = "login_completed",
            "user session authenticated"
        );

        Ok(())
    }

    pub async fn logout(&self) -> Result<(), AppError> {
        self.websocket_service.stop().await?;

        {
            let mut auth_token = self.auth_state.token.write().await;
            *auth_token = None;
        }

        {
            let mut auth_user = self.auth_state.user_id.write().await;
            *auth_user = None;
        }

        {
            let mut auth_session = self.auth_state.session.write().await;
            *auth_session = None;
        }

        {
            let mut authenticated = self.auth_state.is_authenticated.write().await;
            *authenticated = false;
        }

        self.event_service.emit_auth_state().await?;
        tracing::info!(target: "auth_service", event = "logout_completed", "user session cleared");
        Ok(())
    }

    pub async fn update_token(&self, token: String) -> Result<(), AppError> {
        let mut auth_token = self.auth_state.token.write().await;

        *auth_token = Some(token);

        let mut authenticated = self.auth_state.is_authenticated.write().await;
        *authenticated = true;

        Ok(())
    }
}
