use std::sync::Arc;

use crate::models::DeviceInfo;
use futures_util::{SinkExt, StreamExt};
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::JoinHandle,
    time::sleep,
};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use tokio_util::sync::CancellationToken;

use crate::utils::network::has_internet;

use crate::error::AppError;
use crate::events::EventDispatcher;
use crate::models::ConnectionStatus;
use crate::state::AuthState;
use crate::websocket::{
    heartbeat::heartbeat_loop, reconnect::backoff_delay, WebSocketClient, WebSocketSender,
};

/// Counts live manager tasks during tests, so an orphaned task is observable.
#[cfg(test)]
static LIVE_TASKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
struct LiveTask;

#[cfg(test)]
impl LiveTask {
    fn new() -> Self {
        LIVE_TASKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for LiveTask {
    fn drop(&mut self) {
        LIVE_TASKS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
pub struct WebSocketManagerConfig {
    pub url: String,
    pub heartbeat_interval_secs: u64,
    pub reconnect_max_attempts: u32,
    pub reconnect_initial_delay_ms: u64,
    pub reconnect_max_delay_ms: u64,
}

impl From<&crate::utils::config::AppConfig> for WebSocketManagerConfig {
    fn from(config: &crate::utils::config::AppConfig) -> Self {
        Self {
            url: config.websocket_url.clone(),
            heartbeat_interval_secs: config.heartbeat_interval_secs,
            reconnect_max_attempts: config.reconnect_max_attempts,
            reconnect_initial_delay_ms: config.reconnect_initial_delay_ms,
            reconnect_max_delay_ms: config.reconnect_max_delay_ms,
        }
    }
}

pub struct WebSocketManager {
    pub config: WebSocketManagerConfig,
    pub auth_state: Arc<AuthState>,
    pub event_dispatcher: Arc<EventDispatcher>,
    pub connection: Arc<
        Mutex<Option<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>>,
    >,
    pub sender: Arc<Mutex<Option<WebSocketSender>>>,
    // pub shutdown: CancellationToken,
    pub shutdown: Arc<Mutex<CancellationToken>>,
    pub status: Arc<RwLock<ConnectionStatus>>,
    pub reconnect_attempt: Arc<Mutex<u32>>,
    pub task_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Serialises `start` and `stop`, so a concurrent pair cannot spawn two
    /// sockets or overwrite a task handle another call still needs.
    lifecycle: Arc<Mutex<()>>,
}

impl WebSocketManager {
    pub fn new(
        config: WebSocketManagerConfig,
        auth_state: Arc<AuthState>,
        event_dispatcher: Arc<EventDispatcher>,
    ) -> Self {
        Self {
            config,
            auth_state,
            event_dispatcher,
            connection: Arc::new(Mutex::new(None)),
            sender: Arc::new(Mutex::new(None)),
            // shutdown: CancellationToken::new(),
            shutdown: Arc::new(Mutex::new(CancellationToken::new())),
            status: Arc::new(RwLock::new(ConnectionStatus::Disconnected)),
            reconnect_attempt: Arc::new(Mutex::new(0)),
            task_handle: Arc::new(Mutex::new(None)),
            lifecycle: Arc::new(Mutex::new(())),
        }
    }

    pub async fn start(
        &self,
        // token: Option<String>,
        device_info: DeviceInfo,
    ) -> Result<(), AppError> {
        let _lifecycle = self.lifecycle.lock().await;

        println!("WEBSOCKET MANAGER START CALLED");

        tracing::info!(
            target: "websocket",
            event = "start_requested",
            "websocket manager start requested"
        );

        // Idempotent: a live task already owns the socket, so a second start
        // must not spawn a duplicate connection.
        {
            let mut task_guard = self.task_handle.lock().await;

            match task_guard.as_ref() {
                Some(handle) if !handle.is_finished() => {
                    tracing::debug!(
                        target: "websocket",
                        event = "already_running",
                        "websocket manager start ignored: task already running"
                    );

                    return Ok(());
                }

                // A task that ended on its own left its handle behind; it is not
                // the one this start is about to spawn.
                Some(_) => *task_guard = None,

                None => {}
            }
        }

        let state = self.status.clone();
        let dispatcher = self.event_dispatcher.clone();

        // let shutdown = CancellationToken::new();

        // let shutdown = self.shutdown.clone();
        let shutdown = {
            let mut guard = self.shutdown.lock().await;

            if guard.is_cancelled() {
                *guard = CancellationToken::new();
            }

            guard.clone()
        };
        let config = self.config.clone();
        let sender = self.sender.clone();
        let reconnect_attempt = self.reconnect_attempt.clone();
        let task_handle = self.task_handle.clone();
        let device_info = device_info.clone();
        let auth_state = self.auth_state.clone();

        println!("WEBSOCKET URL: {}", config.url);

        let handle = tokio::spawn(async move {
            #[cfg(test)]
            let _live = LiveTask::new();

            println!("WEBSOCKET TOKIO TASK STARTED");

            let device_info_for_loop = device_info.clone();
            let mut attempt = 0_u32;

            loop {
                println!("WEBSOCKET LOOP START - ATTEMPT {}", attempt);

                if shutdown.is_cancelled() {
                    println!("SHUTDOWN DURING INTERNET WAIT");
                    break;
                }

                while !has_internet().await {
                    println!("NO INTERNET. WAITING...");

                    {
                        let mut status_guard = state.write().await;
                        *status_guard = ConnectionStatus::Disconnected;
                    }

                    sleep(std::time::Duration::from_secs(5)).await;

                    if shutdown.is_cancelled() {
                        return;
                    }
                }

                println!("INTERNET AVAILABLE");

                attempt = 0;

                {
                    let mut reconnect_guard = reconnect_attempt.lock().await;
                    *reconnect_guard = 0;
                }

                let token = {
                    let token_guard = auth_state.token.read().await;
                    token_guard.clone()
                };

                let token = match token {
                    Some(token) => token,
                    None => {
                        // The session may still be restoring, so wait rather
                        // than giving up. The reason is logged as an event, not
                        // as a token-presence flag.
                        tracing::debug!(
                            target: "websocket",
                            event = "connect_deferred_no_session",
                            "websocket connect deferred: no desktop session yet"
                        );

                        // Wait for a session, but let `stop` interrupt the wait.
                        tokio::select! {
                            _ = shutdown.cancelled() => break,
                            _ = sleep(std::time::Duration::from_secs(5)) => {}
                        }

                        continue;
                    }
                };

                {
                    let mut status_guard = state.write().await;
                    *status_guard = ConnectionStatus::Connecting;
                }

                println!("TRYING WEBSOCKET CONNECTION: {}", config.url);

                match WebSocketClient::new(
                    config.url.clone(),
                    Some(token),
                    device_info_for_loop.clone(),
                )
                .connect()
                .await
                {
                    Ok(socket) => {
                        println!("WEBSOCKET CONNECT SUCCESS");

                        // Reset reconnect attempts after a successful connection
                        attempt = 0;

                        {
                            let mut reconnect_guard = reconnect_attempt.lock().await;
                            *reconnect_guard = 0;
                        }

                        let (mut write, read) = socket.split();

                        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

                        println!("WEBSOCKET CHANNEL CREATED");

                        let sender_impl = WebSocketSender::new(tx);

                        {
                            let mut sender_guard = sender.lock().await;
                            *sender_guard = Some(sender_impl.clone());
                        }

                        println!("WEBSOCKET SENDER STORED");

                        {
                            let mut status_guard = state.write().await;
                            *status_guard = ConnectionStatus::Connected;
                        }

                        println!("WEBSOCKET STATUS CONNECTED");

                        let dispatcher_loop = dispatcher.clone();

                        let heartbeat_shutdown = shutdown.clone();

                        let heartbeat_sender = Arc::new(sender_impl.clone());

                        let heartbeat_interval = config.heartbeat_interval_secs;

                        println!("STARTING HEARTBEAT TASK");

                        let heartbeat_task = tokio::spawn(async move {
                            println!("HEARTBEAT STARTED");

                            heartbeat_loop(heartbeat_sender, heartbeat_shutdown, heartbeat_interval)
                                .await
                        });

                        println!("STARTING RECEIVER TASK");

                        let receive_dispatch = tokio::spawn(async move {
                            println!("RECEIVER STARTED");

                            crate::websocket::receiver::receive_loop(read, dispatcher_loop).await
                        });

                        println!("WAITING FOR OUTGOING MESSAGES");

                        tokio::pin!(receive_dispatch);
                        tokio::pin!(heartbeat_task);

                        loop {
                            tokio::select! {

                                                                                        Some(message) = rx.recv() => {
                                                                                            println!("SENDING MESSAGE: {:?}", message);

                                                                                            if let Err(err) = write.send(message).await {
                                                                                                println!("SEND ERROR: {:?}", err);
                                                                                                break;
                                                                                            }
                                                                                        }

                                                                                        result = &mut receive_dispatch => {
                                                            match result {
                                                                Ok(Ok(())) => println!("Receiver exited normally"),
                                                                Ok(Err(err)) => println!("Receiver error: {:?}", err),
                                                                Err(err) => println!("Receiver task panicked: {:?}", err),
                                                            }

                                                            break;
                                                        }

                                                                                        result = &mut heartbeat_task => {
                                match result {
                                    Ok(Ok(())) => println!("Heartbeat exited"),
                                    Ok(Err(err)) => println!("Heartbeat error: {:?}", err),
                                    Err(err) => println!("Heartbeat task panicked: {:?}", err),
                                }

                                break;
                            }

                                                                                        _ = shutdown.cancelled() => {
                                                                                            println!("SHUTDOWN");
                                                                                            break;
                                                                                        }
                                                                                    }
                        }

                        println!("MESSAGE LOOP ENDED");

                        heartbeat_task.abort();
                        receive_dispatch.abort();

                        {
                            let mut sender_guard = sender.lock().await;
                            *sender_guard = None;
                        }

                        {
                            let mut status_guard = state.write().await;
                            *status_guard = ConnectionStatus::Disconnected;
                        }

                        println!("WEBSOCKET DISCONNECTED");

                        attempt += 1;

                        if shutdown.is_cancelled() {
                            println!("NOT RECONNECTING BECAUSE SHUTDOWN WAS REQUESTED");
                            break;
                        }

                        {
                            let mut reconnect_guard = reconnect_attempt.lock().await;
                            *reconnect_guard = attempt;
                        }

                        println!("RECONNECTING ATTEMPT {}", attempt);

                        let delay = backoff_delay(
                            attempt,
                            config.reconnect_initial_delay_ms,
                            config.reconnect_max_delay_ms,
                        )
                        .await;

                        sleep(delay).await;

                        continue;
                    }

                    Err(err) => {
                        println!("WEBSOCKET CONNECTION FAILED: {:?}", err);

                        attempt += 1;

                        if attempt >= config.reconnect_max_attempts {
                            println!("MAX RECONNECT ATTEMPTS REACHED");

                            let mut status_guard = state.write().await;

                            *status_guard = ConnectionStatus::Error(err.to_string());

                            break;
                        }

                        {
                            let mut reconnect_guard = reconnect_attempt.lock().await;

                            *reconnect_guard = attempt;
                        }

                        println!("RECONNECTING ATTEMPT {}", attempt);

                        let delay = backoff_delay(
                            attempt,
                            config.reconnect_initial_delay_ms,
                            config.reconnect_max_delay_ms,
                        )
                        .await;

                        sleep(delay).await;
                    }
                }
            }

            println!("WEBSOCKET TASK FINISHED");
        });

        println!("TOKIO TASK SPAWNED");

        let mut task_guard = task_handle.lock().await;

        *task_guard = Some(handle);

        println!("TASK HANDLE STORED");

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), AppError> {
        tracing::info!(target: "websocket", event = "stop_requested", "websocket manager stop requested");

        let _lifecycle = self.lifecycle.lock().await;

        // self.shutdown.cancel();
        {
            let guard = self.shutdown.lock().await;
            guard.cancel();
        }

        // Taken before awaiting: a concurrent caller cannot see this handle,
        // and a task that already finished returns immediately instead of
        // blocking. The task ends once it observes the cancellation.
        let handle = self.task_handle.lock().await.take();

        if let Some(handle) = handle {
            let _ = handle.await;
        }
        {
            let mut sender_guard = self.sender.lock().await;
            *sender_guard = None;
        }
        {
            let mut connection_guard = self.connection.lock().await;
            *connection_guard = None;
        }
        {
            let mut status_guard = self.status.write().await;
            *status_guard = ConnectionStatus::Disconnected;
        }
        tracing::info!(target: "websocket", event = "stopped", "websocket manager stopped");
        Ok(())
    }

    pub async fn send_message(&self, payload: String) -> Result<(), AppError> {
        tracing::info!(target: "websocket", event = "send_attempt", "sending websocket payload");
        let sender_guard = self.sender.lock().await;
        match sender_guard.as_ref() {
            Some(sender) => sender
                .send(Message::Text(payload.into()))
                .map_err(AppError::network),
            None => {
                tracing::warn!(target: "websocket", event = "send_failed", "websocket is not connected; send skipped");
                Err(AppError::not_connected())
            }
        }
    }

    pub async fn is_running(&self) -> bool {
        let guard = self.task_handle.lock().await;

        match guard.as_ref() {
            Some(handle) => !handle.is_finished(),

            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        services::local_transfer_file_service::LocalTransferFileService,
        transfer::{manager::UploadManager, persist::TransferStore},
    };
    use sqlx::sqlite::SqlitePoolOptions;
    use vilsend_core::RecordingEventSink;

    fn device() -> DeviceInfo {
        DeviceInfo {
            device_name: "test".into(),
            device_identifier: "test-device".into(),
            device_type: "desktop".into(),
            operating_system: "test".into(),
            app_version: "0".into(),
        }
    }

    async fn make_manager() -> WebSocketManager {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");

        let local_files = Arc::new(LocalTransferFileService::new(pool).await);
        let (tx, _rx) = mpsc::unbounded_channel();

        let uploads = Arc::new(UploadManager::new(
            Arc::new(RecordingEventSink::new()),
            std::env::temp_dir().join("vilsend-websocket-test-uploads"),
            local_files,
            tx,
            Arc::new(TransferStore::in_memory(
                std::env::temp_dir().join("vilsend-websocket-test-state"),
            )),
        ));

        WebSocketManager::new(
            WebSocketManagerConfig {
                url: "ws://127.0.0.1:9".into(),
                heartbeat_interval_secs: 30,
                reconnect_max_attempts: 1,
                reconnect_initial_delay_ms: 1,
                reconnect_max_delay_ms: 1,
            },
            Arc::new(AuthState::default()),
            Arc::new(EventDispatcher::new(uploads)),
        )
    }

    fn live_tasks() -> usize {
        LIVE_TASKS.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn task_id(manager: &WebSocketManager) -> Option<tokio::task::Id> {
        manager.task_handle.lock().await.as_ref().map(|h| h.id())
    }

    #[tokio::test]
    async fn one_task_per_start_and_a_clean_restart() {
        let manager = make_manager().await;
        let device = device();

        manager.start(device.clone()).await.expect("first start");
        tokio::task::yield_now().await;

        let first = task_id(&manager).await;
        assert!(first.is_some());
        assert_eq!(live_tasks(), 1);

        // A repeated start is a no-op: same task, still one connection.
        manager.start(device.clone()).await.expect("repeated start");
        assert_eq!(task_id(&manager).await, first, "repeated start must reuse the task");
        assert_eq!(live_tasks(), 1);

        manager.stop().await.expect("stop");
        assert!(!manager.is_running().await);
        assert_eq!(*manager.status.read().await, ConnectionStatus::Disconnected);
        assert_eq!(live_tasks(), 0);

        // Stop followed by start reconnects with a fresh task.
        manager.start(device.clone()).await.expect("restart");
        tokio::task::yield_now().await;
        assert!(manager.is_running().await);
        assert_ne!(task_id(&manager).await, first, "a restart must spawn a fresh task");
        assert_eq!(live_tasks(), 1);
        manager.stop().await.expect("final stop");

        // Interleaved start/stop pairs cannot leave a task running.
        for _ in 0..5 {
            let _ = tokio::join!(manager.start(device.clone()), manager.stop());
        }
        manager.stop().await.expect("settle");
        tokio::task::yield_now().await;
        assert_eq!(live_tasks(), 0, "concurrent start/stop must leave no task running");
        assert!(!manager.is_running().await);
    }
}

