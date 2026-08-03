// use std::sync::Arc;

// use crate::events::EventDispatcher;
// use crate::services::local_transfer_file_service::LocalTransferFileService;
// use crate::services::{AuthService, EventService, WebSocketService};
// use crate::state::{AuthState, WebSocketState};
// use crate::transfer::manager::UploadManager;
// use crate::utils::config::AppConfig;

// pub struct AppState {
//     pub config: Arc<AppConfig>,
//     pub auth_state: Arc<AuthState>,
//     pub websocket_state: Arc<WebSocketState>,
//     pub auth_service: Arc<AuthService>,
//     pub websocket_service: Arc<WebSocketService>,
//     pub event_service: Arc<EventService>,
//     pub event_dispatcher: Arc<EventDispatcher>,
//     pub upload_manager: Arc<UploadManager>,
//     pub local_transfer_service: Arc<LocalTransferFileService>,
// }

// impl AppState {
//     pub fn new(
//         app_handle: tauri::AppHandle,
//         config: AppConfig,
//         local_transfer_service: Arc<LocalTransferFileService>,
//     ) -> Self {

//         let config = Arc::new(config);
//         let auth_state = Arc::new(AuthState::default());

//         // Create UploadManager first
//         let upload_root = std::env::temp_dir().join("transfer_uploads");
//         let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

//         let upload_manager = Arc::new(UploadManager::new(
//             app_handle.clone(),
//             upload_root,
//             Arc::clone(&local_transfer_service),
//             tx,
//         ));

//         // Now inject UploadManager into EventDispatcher
//         let event_dispatcher = Arc::new(EventDispatcher::new(Arc::clone(&upload_manager)));

//         event_dispatcher.attach(app_handle);

//         let websocket_state = Arc::new(WebSocketState::new(
//             Arc::clone(&config),
//             Arc::clone(&auth_state),
//             Arc::clone(&event_dispatcher),
//         ));

//         let event_service = Arc::new(EventService::new(Arc::clone(&event_dispatcher)));

//         let websocket_service = Arc::new(WebSocketService::new(
//             Arc::clone(&websocket_state),
//             Arc::clone(&auth_state),
//             Arc::clone(&config),
//             Arc::clone(&event_dispatcher),
//         ));

//         let auth_service = Arc::new(AuthService::new(
//             Arc::clone(&auth_state),
//             Arc::clone(&websocket_service),
//             Arc::clone(&event_service),
//             Arc::clone(&config),
//         ));

//         Self {
//             config,
//             auth_state,
//             websocket_state,
//             auth_service,
//             websocket_service,
//             event_service,
//             event_dispatcher,
//             upload_manager,
//             local_transfer_service,
//         }
//     }
// }

// use std::sync::Arc;

// use crate::events::EventDispatcher;
// use crate::services::local_transfer_file_service::LocalTransferFileService;
// use crate::services::{AuthService, EventService, WebSocketService};
// use crate::state::{AuthState, WebSocketState};
// use crate::transfer::manager::{TransferEvent, UploadManager};
// use crate::utils::config::AppConfig;

// pub struct AppState {
//     pub config: Arc<AppConfig>,
//     pub auth_state: Arc<AuthState>,
//     pub websocket_state: Arc<WebSocketState>,
//     pub auth_service: Arc<AuthService>,
//     pub websocket_service: Arc<WebSocketService>,
//     pub event_service: Arc<EventService>,
//     pub event_dispatcher: Arc<EventDispatcher>,
//     pub upload_manager: Arc<UploadManager>,
//     pub local_transfer_service: Arc<LocalTransferFileService>,
// }

// impl AppState {
//     pub fn new(
//         app_handle: tauri::AppHandle,
//         config: AppConfig,
//         local_transfer_service: Arc<LocalTransferFileService>,
//     ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<TransferEvent>) {
//         let config = Arc::new(config);
//         let auth_state = Arc::new(AuthState::default());

//         // Create UploadManager first
//         let upload_root = std::env::temp_dir().join("transfer_uploads");

//         let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

//         let upload_manager = Arc::new(UploadManager::new(
//             app_handle.clone(),
//             upload_root,
//             Arc::clone(&local_transfer_service),
//             tx,
//         ));

//         // Create EventDispatcher
//         let event_dispatcher = Arc::new(EventDispatcher::new(Arc::clone(&upload_manager)));

//         event_dispatcher.attach(app_handle);

//         let websocket_state = Arc::new(WebSocketState::new(
//             Arc::clone(&config),
//             Arc::clone(&auth_state),
//             Arc::clone(&event_dispatcher),
//         ));

//         let event_service = Arc::new(EventService::new(Arc::clone(&event_dispatcher)));

//         let websocket_service = Arc::new(WebSocketService::new(
//             Arc::clone(&websocket_state),
//             Arc::clone(&auth_state),
//             Arc::clone(&config),
//             Arc::clone(&event_dispatcher),
//         ));

//         let auth_service = Arc::new(AuthService::new(
//             Arc::clone(&auth_state),
//             Arc::clone(&websocket_service),
//             Arc::clone(&event_service),
//             Arc::clone(&config),
//         ));

//         let state = Self {
//             config,
//             auth_state,
//             websocket_state,
//             auth_service,
//             websocket_service,
//             event_service,
//             event_dispatcher,
//             upload_manager,
//             local_transfer_service,
//         };

//         (state, rx)
//     }
// }

// use std::sync::Arc;

// use crate::events::EventDispatcher;
// use crate::services::local_transfer_file_service::LocalTransferFileService;
// use crate::services::{AuthService, EventService, WebSocketService};
// use crate::state::{AuthState, WebSocketState};
// use crate::transfer::manager::{TransferEvent, UploadManager};
// use crate::utils::config::AppConfig;

// pub struct AppState {
//     pub config: Arc<AppConfig>,
//     pub auth_state: Arc<AuthState>,
//     pub websocket_state: Arc<WebSocketState>,
//     pub auth_service: Arc<AuthService>,
//     pub websocket_service: Arc<WebSocketService>,
//     pub event_service: Arc<EventService>,
//     pub event_dispatcher: Arc<EventDispatcher>,
//     pub upload_manager: Arc<UploadManager>,
//     pub local_transfer_service: Arc<LocalTransferFileService>,
// }

// impl AppState {
//     pub fn new(
//         app_handle: tauri::AppHandle,
//         config: AppConfig,
//         local_transfer_service: Arc<LocalTransferFileService>,
//     ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<TransferEvent>) {
//         let config = Arc::new(config);
//         let auth_state = Arc::new(AuthState::default());

//         /*
//          * Transfer event channel
//          */
//         let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

//         /*
//          * Upload manager
//          */
//         let upload_root = std::env::temp_dir().join("transfer_uploads");

//         let upload_manager = Arc::new(UploadManager::new(
//             app_handle.clone(),
//             upload_root,
//             Arc::clone(&local_transfer_service),
//             tx,
//         ));

//         /*
//          * Event dispatcher
//          */
//         let event_dispatcher = Arc::new(EventDispatcher::new(Arc::clone(&upload_manager)));

//         event_dispatcher.attach(app_handle.clone());

//         /*
//          * Websocket state
//          */
//         let websocket_state = Arc::new(WebSocketState::new(
//             Arc::clone(&config),
//             Arc::clone(&auth_state),
//             Arc::clone(&event_dispatcher),
//         ));

//         /*
//          * Event service
//          */
//         let event_service = Arc::new(EventService::new(Arc::clone(&event_dispatcher)));

//         /*
//          * Websocket service
//          */
//         let websocket_service = Arc::new(WebSocketService::new(
//             Arc::clone(&websocket_state),
//             Arc::clone(&auth_state),
//             Arc::clone(&config),
//             Arc::clone(&event_dispatcher),
//         ));

//         /*
//          * Auth service
//          */
//         let auth_service = Arc::new(AuthService::new(
//             Arc::clone(&auth_state),
//             Arc::clone(&websocket_service),
//             Arc::clone(&event_service),
//             Arc::clone(&config),
//         ));

//         let state = Self {
//             config,
//             auth_state,
//             websocket_state,
//             auth_service,
//             websocket_service,
//             event_service,
//             event_dispatcher,
//             upload_manager,
//             local_transfer_service,
//         };

//         (state, rx)
//     }

//     /*
//      * Call this after websocket connection is established
//      */
//     pub async fn attach_websocket_manager(&self) {
//         let manager = {
//             let guard = self.websocket_state.manager.lock().await;

//             guard.clone()
//         };

//         if let Some(manager) = manager {
//             self.event_dispatcher.attach_websocket_manager(manager);
//         }
//     }
// }

use std::sync::Arc;

use crate::events::EventDispatcher;
use crate::services::local_transfer_file_service::LocalTransferFileService;
use crate::services::{AuthService, EventService, WebSocketService};
use crate::state::{AuthState, WebSocketState};
use crate::transfer::manager::{TransferEvent, UploadManager};
use crate::utils::config::AppConfig;
use crate::websocket::WebSocketManager;

pub struct AppState {
    pub config: Arc<AppConfig>,
    pub auth_state: Arc<AuthState>,
    pub websocket_state: Arc<WebSocketState>,
    pub auth_service: Arc<AuthService>,
    pub websocket_service: Arc<WebSocketService>,
    pub event_service: Arc<EventService>,
    pub event_dispatcher: Arc<EventDispatcher>,
    pub upload_manager: Arc<UploadManager>,
    pub local_transfer_service: Arc<LocalTransferFileService>,
}

impl AppState {
    pub fn new(
        app_handle: tauri::AppHandle,
        config: AppConfig,
        local_transfer_service: Arc<LocalTransferFileService>,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<TransferEvent>) {
        let config = Arc::new(config);
        let auth_state = Arc::new(AuthState::default());

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let upload_root = std::env::temp_dir().join("transfer_uploads");

        let upload_manager = Arc::new(UploadManager::new(
            app_handle.clone(),
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
            Arc::clone(&config),
            Arc::clone(&event_dispatcher),
        ));

        let auth_service = Arc::new(AuthService::new(
            Arc::clone(&auth_state),
            Arc::clone(&websocket_service),
            Arc::clone(&event_service),
            Arc::clone(&config),
        ));

        let state = Self {
            config,
            auth_state,
            websocket_state,
            auth_service,
            websocket_service,
            event_service,
            event_dispatcher,
            upload_manager,
            local_transfer_service,
        };

        (state, rx)
    }

    /*
     * Call this immediately when WebSocketManager is created
     */
    pub async fn set_websocket_manager(&self, manager: Arc<WebSocketManager>) {
        {
            let mut websocket = self.websocket_state.manager.lock().await;

            *websocket = Some(manager.clone());
        }

        self.event_dispatcher.attach_websocket_manager(manager);
    }
}
