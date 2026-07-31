use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::error::AppError;
use crate::events::EventDispatcher;
use crate::models::{ConnectionStatus, DeviceInfo};
use crate::utils::config::AppConfig;
use crate::websocket::{WebSocketManager, WebSocketManagerConfig};

pub struct WebSocketState {
    pub manager: Arc<Mutex<Option<Arc<WebSocketManager>>>>,
    pub status: Arc<RwLock<ConnectionStatus>>,
    pub config: Arc<AppConfig>,
    pub auth_state: Arc<crate::state::AuthState>,
    pub event_dispatcher: Arc<EventDispatcher>,
}

impl WebSocketState {
    pub fn new(
        config: Arc<AppConfig>,
        auth_state: Arc<crate::state::AuthState>,
        event_dispatcher: Arc<EventDispatcher>,
    ) -> Self {
        Self {
            manager: Arc::new(Mutex::new(None)),
            status: Arc::new(RwLock::new(ConnectionStatus::Disconnected)),
            config,
            auth_state,
            event_dispatcher,
        }
    }

    pub async fn start(&self, device_info: DeviceInfo) -> Result<(), AppError> {
        println!("WEBSOCKET STATE START CALLED");

        let has_token = self.auth_state.token.read().await.is_some();

        println!("TOKEN AVAILABLE IN WEBSOCKET STATE: {}", has_token);

        if !has_token {
            println!("NO AUTH TOKEN AVAILABLE");

            return Err(AppError::not_authenticated());
        }

        let mut guard = self.manager.lock().await;

        println!("WEBSOCKET MANAGER LOCK ACQUIRED");

        if let Some(manager) = guard.as_ref() {
            println!("WEBSOCKET MANAGER ALREADY EXISTS");

            /*
                Important:
                If manager exists but its task died,
                restart it.
            */

            if manager.is_running().await {
                println!("WEBSOCKET MANAGER ALREADY RUNNING");

                return Ok(());
            }

            println!("OLD MANAGER DEAD - REMOVING");
        }

        println!("CREATING NEW WEBSOCKET MANAGER");

        let manager = Arc::new(WebSocketManager::new(
            WebSocketManagerConfig::from(self.config.as_ref()),
            self.auth_state.clone(),
            self.event_dispatcher.clone(),
        ));

        println!("STARTING WEBSOCKET MANAGER");

        manager.start(device_info).await?;

        *guard = Some(manager);

        println!("WEBSOCKET MANAGER STORED");

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), AppError> {
        let mut guard = self.manager.lock().await;

        if let Some(manager) = guard.take() {
            manager.stop().await?;
        }

        Ok(())
    }

    pub async fn send_message(&self, payload: String) -> Result<(), AppError> {
        let guard = self.manager.lock().await;

        match guard.as_ref() {
            Some(manager) => manager.send_message(payload).await,

            None => Err(AppError::not_connected()),
        }
    }

    pub async fn status(&self) -> Result<ConnectionStatus, AppError> {
        if let Some(manager) = self.manager.lock().await.as_ref() {
            return Ok(manager.status.read().await.clone());
        }

        Ok(ConnectionStatus::Disconnected)
    }
}
