use std::sync::Arc;

use crate::error::AppError;
use crate::services::oauth_service::OAuthService;
use crate::services::{EventService, WebSocketService};
use crate::state::AuthState;
use crate::utils::config::AppConfig;

/// Owns the desktop session lifecycle.
///
/// The token itself is obtained by [`OAuthService`] (system-browser
/// Authorization Code + PKCE) and lives in [`AuthState`]; this service is
/// responsible for tearing the session down and telling the UI about it.
pub struct AuthService {
    auth_state: Arc<AuthState>,
    websocket_service: Arc<WebSocketService>,
    event_service: Arc<EventService>,
    oauth_service: Arc<OAuthService>,
    #[allow(dead_code)]
    config: Arc<AppConfig>,
}

impl AuthService {
    pub fn new(
        auth_state: Arc<AuthState>,
        websocket_service: Arc<WebSocketService>,
        event_service: Arc<EventService>,
        oauth_service: Arc<OAuthService>,
        config: Arc<AppConfig>,
    ) -> Self {
        Self {
            auth_state,
            websocket_service,
            event_service,
            oauth_service,
            config,
        }
    }

    /// Announces that a sign-in completed. Called by the OAuth callback handler
    /// after the token has been stored.
    pub async fn notify_signed_in(&self) -> Result<(), AppError> {
        let user_id = self.auth_state.user_id.read().await.clone();

        self.event_service.emit_auth_state(true, user_id).await?;

        tracing::info!(
            target: "auth_service",
            event = "login_completed",
            "user session authenticated"
        );

        Ok(())
    }

    /// Ends the local desktop session.
    ///
    /// Stops dependent services, drops every credential and any in-flight
    /// authorization request, then tells the UI it is signed out.
    pub async fn logout(&self) -> Result<(), AppError> {
        self.websocket_service.stop().await?;

        self.oauth_service.clear().await;

        self.event_service.emit_auth_state(false, None).await?;

        tracing::info!(
            target: "auth_service",
            event = "logout_completed",
            "user session cleared"
        );

        Ok(())
    }
}
