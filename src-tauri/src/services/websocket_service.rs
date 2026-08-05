// use std::sync::Arc;

// use crate::error::AppError;
// use crate::events::EventDispatcher;
// use crate::state::{AuthState, WebSocketState};
// use crate::utils::config::AppConfig;
// use crate::models::DeviceInfo;

// pub struct WebSocketService {
//     websocket_state: Arc<WebSocketState>,
//     auth_state: Arc<AuthState>,
//     config: Arc<AppConfig>,
//     event_dispatcher: Arc<EventDispatcher>,
// }

// impl WebSocketService {
//     pub fn new(
//         websocket_state: Arc<WebSocketState>,
//         auth_state: Arc<AuthState>,
//         config: Arc<AppConfig>,
//         event_dispatcher: Arc<EventDispatcher>,
//     ) -> Self {
//         Self {
//             websocket_state,
//             auth_state,
//             config,
//             event_dispatcher,
//         }
//     }

//     pub async fn start(&self, device_info: DeviceInfo) -> Result<(), AppError> {
//         println!("WEBSOCKET SERVICE START CALLED");

//         let token = self.auth_state.token.read().await.clone();

//         println!("AUTH TOKEN AVAILABLE: {}", token.is_some());

//         if token.is_none() {
//             println!("WEBSOCKET START FAILED: NO AUTH TOKEN");
//             return Err(AppError::not_authenticated());
//         }

//         println!("CALLING WEBSOCKET STATE START");

//         let result = self.websocket_state.start(token, device_info).await;

//         match &result {
//             Ok(_) => {
//                 println!("WEBSOCKET STATE START SUCCESS");
//             }
//             Err(err) => {
//                 println!("WEBSOCKET STATE START FAILED: {:?}", err);
//             }
//         }

//         result
//     }

//     pub async fn stop(&self) -> Result<(), AppError> {
//         self.websocket_state.stop().await
//     }

//     pub async fn send_message(&self, payload: String) -> Result<(), AppError> {
//         self.websocket_state.send_message(payload).await
//     }

//     pub async fn status(&self) -> Result<crate::models::ConnectionStatus, AppError> {
//         self.websocket_state.status().await
//     }
// }

use std::sync::Arc;

use crate::error::AppError;
use crate::events::EventDispatcher;
use crate::models::DeviceInfo;
use crate::state::{AuthState, WebSocketState};
use crate::utils::config::AppConfig;

pub struct WebSocketService {
    websocket_state: Arc<WebSocketState>,
    auth_state: Arc<AuthState>,
    config: Arc<AppConfig>,
    event_dispatcher: Arc<EventDispatcher>,
}

impl WebSocketService {
    pub fn new(
        websocket_state: Arc<WebSocketState>,
        auth_state: Arc<AuthState>,
        config: Arc<AppConfig>,
        event_dispatcher: Arc<EventDispatcher>,
    ) -> Self {
        Self {
            websocket_state,
            auth_state,
            config,
            event_dispatcher,
        }
    }

    pub async fn start(&self, device_info: DeviceInfo) -> Result<(), AppError> {
        println!("WEBSOCKET SERVICE START CALLED");

        let has_token = self.auth_state.token.read().await.is_some();

        println!("AUTH TOKEN AVAILABLE: {}", has_token);

        if !has_token {
            println!("WEBSOCKET START FAILED: NO AUTH TOKEN");

            return Err(AppError::not_authenticated());
        }

        println!("CALLING WEBSOCKET STATE START");

        let result = self.websocket_state.start(device_info).await;

        match result {
            Ok(_) => {
                println!("WEBSOCKET STATE START SUCCESS");

                /*
                 * WebSocketManager is now created.
                 * Attach it to EventDispatcher.
                 */
                let manager = {
                    let guard = self.websocket_state.manager.lock().await;
                    guard.clone()
                };

                if let Some(manager) = manager {
                    self.event_dispatcher.attach_websocket_manager(manager);

                    println!("WEBSOCKET MANAGER ATTACHED TO EVENT DISPATCHER");
                } else {
                    println!("WARNING: websocket manager missing after start");
                }

                Ok(())
            }

            Err(err) => {
                println!("WEBSOCKET STATE START FAILED: {:?}", err);

                Err(err)
            }
        }
    }

    pub async fn stop(&self) -> Result<(), AppError> {
        self.websocket_state.stop().await
    }

    pub async fn send_message(&self, payload: String) -> Result<(), AppError> {
        self.websocket_state.send_message(payload).await
    }

    pub async fn status(&self) -> Result<crate::models::ConnectionStatus, AppError> {
        self.websocket_state.status().await
    }
}
