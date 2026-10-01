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
    /// An assembled file is not the file the sender described.
    ///
    /// Its own error rather than a `Receiver(..)` string because the receiver
    /// acts on it: the chunks behind the mismatch are dropped and asked for
    /// again, and the file is never published.
    #[error("integrity check failed for {path}: expected {expected}, got {actual}")]
    IntegrityMismatch {
        path: String,
        expected: String,
        actual: String,
    },
}
pub type Result<T> = std::result::Result<T, TransferError>;
