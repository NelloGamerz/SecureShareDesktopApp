pub mod client;
pub mod heartbeat;
pub mod manager;
pub mod receiver;
pub mod reconnect;
pub mod sender;
pub mod server_command;

pub use client::WebSocketClient;
pub use manager::{WebSocketManager, WebSocketManagerConfig};
pub use sender::WebSocketSender;
pub use server_command::ServerCommand;

/// Heartbeat frames are deliberately kept out of the logs: one every few
/// seconds buries everything else.
pub(crate) fn is_heartbeat(message: &tokio_tungstenite::tungstenite::Message) -> bool {
    use tokio_tungstenite::tungstenite::Message;

    matches!(message, Message::Ping(_) | Message::Pong(_))
        || matches!(message, Message::Text(text) if text.contains("\"PING\"") || text.contains("\"PONG\""))
}
