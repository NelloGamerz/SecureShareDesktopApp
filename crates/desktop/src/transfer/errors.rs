#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("transfer not found: {0}")]
    NotFound(String),
    #[error("invalid transfer: {0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("receiver error: {0}")]
    Receiver(String),
    #[error("crypto error: {0}")]
    Crypto(String),
}
pub type Result<T> = std::result::Result<T, TransferError>;
