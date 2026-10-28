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
    /// A chunk could not be compressed on the way out, or decompressed on the
    /// way in.
    ///
    /// Distinct from [`TransferError::Crypto`] because it happens outside the
    /// AEAD: the bytes authenticated and then failed to expand. On the sender it
    /// is recoverable (send the chunk uncompressed); on the receiver it means the
    /// peer's body is not the encoding it declared.
    #[error("compression error: {0}")]
    Compression(String),
    /// The receiver refused a chunk and named the chunks it no longer holds.
    ///
    /// Distinct from a receiver error because it is actionable: the sender can
    /// send exactly these again instead of failing the transfer or resending
    /// everything.
    #[error("receiver is missing chunks: {missing:?}")]
    ChunksMissing { missing: Vec<u64> },
    /// A peer sent something that is not the protocol.
    ///
    /// Its own variant because only the v2 framed session can produce it, and
    /// what the receiver does about it differs from every other failure: a
    /// malformed frame is not a chunk to retry and not a receiver to blame, it
    /// is a session to end. Folding it into `Receiver(..)` would lose that.
    #[error("protocol error: {0}")]
    Protocol(String),
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
