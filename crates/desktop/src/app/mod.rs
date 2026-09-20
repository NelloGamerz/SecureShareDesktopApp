use std::sync::Arc;

use vilsend_core::EventSink;

use crate::events::EventDispatcher;
use crate::services::local_transfer_file_service::LocalTransferFileService;
use crate::services::oauth_service::OAuthService;
use crate::services::{AuthService, EventService, WebSocketService};
use crate::state::{AuthState, WebSocketState};
use crate::transfer::manager::{TransferEvent, UploadManager};
use crate::utils::config::AppConfig;

/// The state the webview can reach.
///
/// Only the services are held. The `config`, `auth_state`,
/// `websocket_state`, `upload_manager` and `local_transfer_service` values
/// built in `new` are owned by the services below, which are their only
/// readers, so keeping second copies here would be dead weight.
pub struct AppState {
    pub auth_service: Arc<AuthService>,
    pub oauth_service: Arc<OAuthService>,
    pub websocket_service: Arc<WebSocketService>,
    pub event_service: Arc<EventService>,
    pub event_dispatcher: Arc<EventDispatcher>,
}

impl AppState {
    pub fn new(
        app_handle: tauri::AppHandle,
        events: Arc<dyn EventSink>,
        config: AppConfig,
        local_transfer_service: Arc<LocalTransferFileService>,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<TransferEvent>) {
        let config = Arc::new(config);
        let auth_state = Arc::new(AuthState::default());

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let upload_root = std::env::temp_dir().join("transfer_uploads");

        let upload_manager = Arc::new(UploadManager::new(
            events,
            upload_root,
            Arc::clone(&local_transfer_service),
            tx,
        ));

        let event_dispatcher = Arc::new(EventDispatcher::new(Arc::clone(&upload_manager)));

        event_dispatcher.attach(app_handle.clone());

        let websocket_state = Arc::new(WebSocketState::new(
            Arc::clone(&config),
            Arc::clone(&auth_state),
            Arc::clone(&event_dispatcher),
        ));

        let event_service = Arc::new(EventService::new(Arc::clone(&event_dispatcher)));

        let websocket_service = Arc::new(WebSocketService::new(
            Arc::clone(&websocket_state),
            Arc::clone(&auth_state),
            Arc::clone(&event_dispatcher),
        ));

        let oauth_service = Arc::new(OAuthService::new(
            Arc::clone(&auth_state),
            app_handle.clone(),
        ));

        let auth_service = Arc::new(AuthService::new(
            Arc::clone(&auth_state),
            Arc::clone(&websocket_service),
            Arc::clone(&event_service),
            Arc::clone(&oauth_service),
            Arc::clone(&config),
        ));

        let state = Self {
            auth_service,
            oauth_service,
            websocket_service,
            event_service,
            event_dispatcher,
        };

        (state, rx)
    }
}
