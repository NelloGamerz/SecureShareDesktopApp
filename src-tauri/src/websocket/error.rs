use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebSocketError {
    #[error("connection closed")]
    Closed,
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("authentication failed")]
    AuthenticationFailed,
}
