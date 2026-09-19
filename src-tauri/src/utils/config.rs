use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub websocket_url: String,
    pub heartbeat_interval_secs: u64,
    pub reconnect_max_attempts: u32,
    pub reconnect_initial_delay_ms: u64,
    pub reconnect_max_delay_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::development()
    }
}

impl AppConfig {
    pub fn development() -> Self {
        Self {
            websocket_url: env::var("WS_URL")
                .unwrap_or_else(|_| "wss://api.vilsend.in/ws".to_string()),
            heartbeat_interval_secs: 30,
            reconnect_max_attempts: 8,
            reconnect_initial_delay_ms: 500,
            reconnect_max_delay_ms: 15000,
        }
    }

    pub fn production() -> Self {
        Self {
            websocket_url: env::var("WS_URL")
                .unwrap_or_else(|_| "wss://api.vilsend.in/ws".to_string()),
            heartbeat_interval_secs: 30,
            reconnect_max_attempts: 8,
            reconnect_initial_delay_ms: 1000,
            reconnect_max_delay_ms: 30000,
        }
    }
}
