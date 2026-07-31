use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub environment: String,
    pub websocket_url: String,
    pub api_url: String,
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
    // pub fn development() -> Self {
    //     Self {
    //         environment: "development".to_string(),
    //         websocket_url: env::var("WS_URL")
    //             .unwrap_or_else(|_| "ws://localhost:8080/ws".to_string()),
    //         api_url: env::var("API_URL").unwrap_or_else(|_| "http://localhost:8080".to_string()),
    //         heartbeat_interval_secs: 30,
    //         reconnect_max_attempts: 8,
    //         reconnect_initial_delay_ms: 500,
    //         reconnect_max_delay_ms: 15000,

    //     }
    // }

    pub fn development() -> Self {
        Self {
            environment: "development".to_string(),
            websocket_url: env::var("WS_URL")
                .unwrap_or_else(|_| "wss://secureserver-0-01.onrender.com/ws".to_string()),
            api_url: env::var("API_URL")
                .unwrap_or_else(|_| "https://secureserver-0-01.onrender.com".to_string()),
            heartbeat_interval_secs: 30,
            reconnect_max_attempts: 8,
            reconnect_initial_delay_ms: 500,
            reconnect_max_delay_ms: 15000,
        }
    }

    pub fn production() -> Self {
        Self {
            environment: "production".to_string(),
            websocket_url: env::var("WS_URL")
                .unwrap_or_else(|_| "wss://secureserver-0-01.onrender.com/ws".to_string()),
            api_url: env::var("API_URL")
                .unwrap_or_else(|_| "https://secureserver-0-01.onrender.com".to_string()),
            heartbeat_interval_secs: 30,
            reconnect_max_attempts: 8,
            reconnect_initial_delay_ms: 1000,
            reconnect_max_delay_ms: 30000,
        }
    }
}
