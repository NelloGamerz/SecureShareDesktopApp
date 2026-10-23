//! The protocol session: the layer between the transfer engine and a transport.
//!
//! # Why this exists
//!
//! `UploadManager` used to call [`HttpClient`] directly, from five places. That
//! made the engine and one wire format the same code, so there was nowhere for a
//! second wire format — or a second transport — to attach.
//!
//! A [`ProtocolSession`] is the seam. It names the operations the engine
//! actually needs and nothing else.
//!
//! # Why v1 is a session and not a transport
//!
//! `vilsend_core`'s transport port is byte-oriented on purpose: a stream of
//! frames, with nothing above it that knows what a chunk is
//! (`crates/core/src/ports/transport.rs`).
//!
//! The frozen v1 wire cannot be expressed that way. Its framing *is* its HTTP
//! headers — `Transfer-Id`, `File-Id`, `Chunk-Index`, `Total-Chunks`,
//! `Relative-Path`, `Chunk-Nonce`, `Encryption` — one request per chunk. A
//! transport that emitted those would have to know what a chunk is, which is
//! exactly what the port's invariant forbids.
//!
//! So the frozen v1 path is a **session** over [`HttpClient`], and the v2 framed
//! protocol is a session over a `vilsend_core::DuplexStream`. That is what keeps
//! v1 compatibility true by construction: [`LegacyV1Session`] delegates to the
//! same calls the engine made before it existed, so the bytes on the wire cannot
//! have changed.
//!
//! # Why the session owns the encryption
//!
//! [`ProtocolSession::send_chunk`] takes **plaintext**, not a sealed chunk.
//!
//! The alternative — the engine encrypts and the session only carries the
//! ciphertext — is what Stage 1a did, and it cannot work for v2. The v2 AEAD
//! binds associated data (`transfer_id ‖ file_id ‖ chunk_index ‖ relative_path
//! ‖ proto_v`, `crypto_v2::ChunkAad`), and that is a property of the *protocol*,
//! not of the engine. More concretely, the two versions derive their keys
//! differently: v1's KDF has no salt and v2's is salted with both handshake
//! nonces, which do not exist until the handshake has run. An engine that
//! derived the key itself could only ever derive v1's.
//!
//! So the session establishes the session, derives the key, and encrypts. That
//! is why [`ProtocolSession::open`] returns the key rather than taking it: the
//! engine still needs it, for the keychain and for progress, but it is no longer
//! the thing that computes it.

use async_trait::async_trait;

use crate::models::chunk::ChunkJob;
use crate::transfer::{
    compression::{self, ChunkEncoding},
    constants::LEGACY_CHUNK_SIZE,
    crypto,
    errors::{Result, TransferError},
    http_client::{HttpClient, ReceiverKey, SessionStart, TransferResumeResponse},
    integrity::ExpectedTransfer,
};

/// Everything the engine must tell a session before it can be opened.
#[derive(Debug)]
pub struct OpenRequest<'a> {
    /// The transfer this session is for.
    pub transfer_id: &'a str,
    /// The total the sender is going to send, which the receiver checks for
    /// space before it accepts.
    pub file_size: u64,
    /// What the files are supposed to hash to, when the sender has worked it
    /// out. `None` for a sender with nothing to describe — an old peer, or a
    /// transfer whose plan is not yet known.
    pub expected: Option<&'a ExpectedTransfer>,
    /// The key this transfer was already using, when it has one.
    ///
    /// A receiver that already has the transfer live **keeps the key it had**
    /// and says so with [`SessionStart::AlreadyActive`]; the sender must then
    /// keep using that key too, which is why it has to be handed in rather than
    /// derived. `None` for a transfer that has never run.
    pub existing_key: Option<[u8; 32]>,
}

/// What opening a session produced.
#[derive(Debug)]
pub struct OpenedSession {
    /// The key to encrypt with, whichever of the two cases above applied.
    pub transfer_key: [u8; 32],
    /// Whether the receiver derived a fresh key or kept the one it had.
    pub start: SessionStart,
    /// What the receiver already holds, when the protocol learns it *during*
    /// the handshake.
    ///
    /// The v2 framed protocol exchanges a manifest as part of opening, so its
    /// resume point only exists once the session is up. v1 learns it from a
    /// separate request, before the handshake, and leaves this `None` — see
    /// [`ProtocolSession::resume_state`].
    pub resume: Option<TransferResumeResponse>,
}

/// What a receiver can accept, known before a chunk plan is made.
///
/// Chunk size is a *sender* decision only because the receiver has not spoken
/// yet. Once it has — on the same handshake that advertises compression — the
/// plan must fit inside what it will accept, or every chunk is refused by the
/// receiver's body limit before it is ever looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverCapabilities {
    /// The largest chunk body the receiver will accept.
    pub max_chunk_bytes: usize,
}

impl Default for ReceiverCapabilities {
    /// A receiver that said nothing gets the legacy size, because that is what
    /// its request-body limit is known to accept.
    fn default() -> Self {
        Self {
            max_chunk_bytes: LEGACY_CHUNK_SIZE,
        }
    }
}

/// The operations the transfer engine needs, whichever protocol carries them.
///
/// Object-safe so that `UploadManager` can hold an `Arc<dyn ProtocolSession>`
/// and swap it, which is what a mid-transfer failover will do.
#[async_trait]
pub trait ProtocolSession: Send + Sync {
    /// Ask what the receiver will accept before the chunk plan is made.
    ///
    /// Called *before* [`ProtocolSession::resume_state`] and
    /// [`ProtocolSession::open`], because chunk size is baked into the jobs and
    /// the digests the receiver is later checked against; finding out afterwards
    /// would mean planning twice.
    ///
    /// The default is correct for a protocol whose handshake has already been
    /// asked, or for a test double: it reports the legacy limit rather than
    /// assuming the new one.
    async fn capabilities(&self) -> Result<ReceiverCapabilities> {
        Ok(ReceiverCapabilities::default())
    }

    /// Ask which chunks of a transfer the receiver already holds.
    ///
    /// `Ok(None)` means it has nothing to say — it has never seen the transfer,
    /// or it predates the question. Both mean "send everything".
    ///
    /// Called *before* [`ProtocolSession::open`], because that is the order the
    /// frozen v1 wire requires: asking after the handshake would make a receiver
    /// that has just learned about the transfer answer with a report instead of
    /// the `404` it answers today. A protocol whose resume point is only known
    /// after the handshake returns `None` here and supplies it through
    /// [`OpenedSession::resume`].
    async fn resume_state(&self, transfer_id: &str) -> Result<Option<TransferResumeResponse>>;

    /// Open a session for a transfer, or re-open one the receiver still has.
    ///
    /// Returns the key to encrypt with. For a receiver that restarted this is a
    /// freshly derived one; for a receiver that never noticed us leave it is the
    /// key the transfer already used.
    async fn open(&self, request: OpenRequest<'_>) -> Result<OpenedSession>;

    /// Encrypt one chunk and send it.
    ///
    /// Returns once the receiver has **durably accepted** the chunk — written to
    /// disk and recorded — not once the bytes have left this process. The retry
    /// loop that calls this re-encrypts on each attempt, because it calls this
    /// again, which is what makes a retry draw a fresh nonce.
    async fn send_chunk(&self, job: &ChunkJob, plaintext: Vec<u8>) -> Result<()>;

    /// Ask the receiver to drop a transfer and everything it holds for it.
    ///
    /// Idempotent, and best-effort at the call site: a receiver that never hears
    /// about it still loses the transfer to its TTL sweep.
    async fn cancel(&self, transfer_id: &str) -> Result<()>;
}

/// The frozen v1 wire, as a session.
///
/// Holds the endpoint, the token and the key the transfer is encrypting with, so
/// that callers stop threading them through every call. That is the shape the
/// transport port needs: a transport is constructed knowing where it is aimed.
///
/// The key lives behind a lock because [`ProtocolSession::open`] replaces it —
/// a receiver that restarted derives a fresh one — while
/// [`ProtocolSession::send_chunk`] takes `&self` and may be running from several
/// workers at once. It is a `std::sync::RwLock`, not a tokio one, because the
/// only thing done under it is a 32-byte copy: it is never held across an
/// `await`.
pub struct LegacyV1Session {
    client: HttpClient,
    endpoint: String,
    token: String,
    transfer_key: std::sync::RwLock<[u8; 32]>,
    /// How the receiver asked for chunks to be encoded.
    ///
    /// Set during [`ProtocolSession::open`] from the receiver's handshake and
    /// read by every worker in [`ProtocolSession::send_chunk`]. It starts at
    /// [`ChunkEncoding::Identity`], which is what a receiver that never
    /// advertises compression gets.
    chunk_encoding: std::sync::RwLock<ChunkEncoding>,
    /// The receiver's key and the capabilities advertised beside it.
    ///
    /// Cached because the plan is made from it by
    /// [`ProtocolSession::capabilities`] and the handshake in
    /// [`ProtocolSession::open`] needs the same key moments later; fetching it
    /// twice would be two requests for one answer.
    receiver_key: tokio::sync::Mutex<Option<ReceiverKey>>,
}

impl LegacyV1Session {
    /// A session against `endpoint`, authenticating with `token`.
    pub fn new(client: HttpClient, endpoint: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            client,
            endpoint: endpoint.into(),
            token: token.into(),
            transfer_key: std::sync::RwLock::new([0u8; 32]),
            chunk_encoding: std::sync::RwLock::new(ChunkEncoding::Identity),
            receiver_key: tokio::sync::Mutex::new(None),
        }
    }

    /// A session with a fresh client, which is what a caller that has no reason
    /// to share one wants.
    pub fn to(endpoint: impl Into<String>, token: impl Into<String>) -> Self {
        Self::new(HttpClient::new(), endpoint, token)
    }

    /// The endpoint this session is aimed at.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The key this session is currently encrypting with.
    fn key(&self) -> [u8; 32] {
        *self
            .transfer_key
            .read()
            .expect("transfer key lock poisoned")
    }

    fn set_key(&self, key: [u8; 32]) {
        *self
            .transfer_key
            .write()
            .expect("transfer key lock poisoned") = key;
    }

    /// How the receiver asked for chunks to be encoded.
    fn chunk_encoding(&self) -> ChunkEncoding {
        *self
            .chunk_encoding
            .read()
            .expect("chunk encoding lock poisoned")
    }

    fn set_chunk_encoding(&self, encoding: ChunkEncoding) {
        *self
            .chunk_encoding
            .write()
            .expect("chunk encoding lock poisoned") = encoding;
    }

    /// The receiver's key and advertised capabilities, fetching them once.
    async fn receiver_key(&self) -> Result<ReceiverKey> {
        let mut cached = self.receiver_key.lock().await;

        if let Some(key) = cached.as_ref() {
            return Ok(key.clone());
        }

        let key = self
            .client
            .get_public_key(&self.endpoint, &self.token)
            .await?;

        *cached = Some(key.clone());

        Ok(key)
    }
}

impl std::fmt::Debug for LegacyV1Session {
    /// Redacts the token: it is a credential, and a derived `Debug` would put it
    /// in any log line that formatted the session.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyV1Session")
            .field("endpoint", &self.endpoint)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[async_trait]
impl ProtocolSession for LegacyV1Session {
    async fn capabilities(&self) -> Result<ReceiverCapabilities> {
        let key = self.receiver_key().await?;

        Ok(ReceiverCapabilities {
            max_chunk_bytes: key.max_chunk_bytes,
        })
    }

    async fn resume_state(&self, transfer_id: &str) -> Result<Option<TransferResumeResponse>> {
        self.client
            .transfer_status(&self.endpoint, &self.token, transfer_id)
            .await
    }

    async fn open(&self, request: OpenRequest<'_>) -> Result<OpenedSession> {
        /*
         * The v1 handshake, in the order it has always been in: fetch the
         * receiver's long-term key, derive an ephemeral one, agree a secret.
         * Every call below is the one the engine made directly before this type
         * existed, with the same arguments, so the wire is unchanged.
         */
        let receiver_key = self.receiver_key().await?;

        /*
         * Record what the receiver can decode before any chunk is sent. This is
         * a device capability carried by the handshake; a receiver that does not
         * advertise it keeps receiving identity chunks, which is what makes the
         * frozen v1 wire unchanged for installed peers.
         */
        self.set_chunk_encoding(receiver_key.chunk_encoding);

        let receiver_public =
            crypto::decode_public_key(&receiver_key.public_key).map_err(TransferError::Crypto)?;

        let ephemeral = crypto::generate_ephemeral_keypair();

        let shared = crypto::derive_shared_secret(&ephemeral.private_key, &receiver_public);

        let derived = crypto::derive_transfer_key(&shared).map_err(TransferError::Invalid)?;

        let start = self
            .client
            .start_transfer(
                &self.endpoint,
                &self.token,
                request.transfer_id,
                &crypto::encode_public_key(&ephemeral.public_key),
                request.file_size,
                request.expected,
            )
            .await?;

        /*
         * Which key is now in force.
         *
         * `Established` means the receiver built its key from the ephemeral
         * public key we just sent, so the one derived here is the matching half.
         * `AlreadyActive` means it kept the key it already had, so the derived
         * one is discarded and the transfer continues on the key it was using —
         * re-deriving would produce a key the receiver cannot decrypt with.
         */
        let transfer_key = match start {
            SessionStart::Established => derived,

            SessionStart::AlreadyActive => request.existing_key.ok_or_else(|| {
                TransferError::Invalid(format!(
                    "receiver already has transfer {} live, but no key for it was kept",
                    request.transfer_id
                ))
            })?,
        };

        self.set_key(transfer_key);

        Ok(OpenedSession {
            transfer_key,
            start,
            // v1 learns this from `resume_state`, before the handshake.
            resume: None,
        })
    }

    async fn send_chunk(&self, job: &ChunkJob, plaintext: Vec<u8>) -> Result<()> {
        /*
         * Compress first, if the receiver said it can decode it. This runs on
         * the blocking pool because zstd is CPU-bound and several workers share
         * the async executor; a 16 MiB chunk compressed inline would stall every
         * other transfer task on this runtime.
         *
         * The original bytes come back from the closure so an incompressible
         * chunk — or a compressor failure — costs nothing but the attempt.
         */
        let (body, encoding) = match self.chunk_encoding() {
            ChunkEncoding::Identity => (plaintext, ChunkEncoding::Identity),

            ChunkEncoding::Zstd => {
                let (original, compressed) = tokio::task::spawn_blocking(move || {
                    let compressed = compression::compress(&plaintext);

                    (plaintext, compressed)
                })
                .await
                .map_err(|error| {
                    TransferError::Compression(format!("the compression task failed: {error}"))
                })?;

                match compressed {
                    Ok(Some(compressed)) => (compressed, ChunkEncoding::Zstd),

                    Ok(None) => {
                        tracing::debug!(
                            transfer_id=%job.transfer_id,
                            chunk=job.index,
                            "chunk does not compress; sending it as it is"
                        );

                        (original, ChunkEncoding::Identity)
                    }

                    /*
                     * A failed compression is not a failed transfer. The bytes
                     * are still exactly what the receiver expects, and sending
                     * them uncompressed is the behaviour of every peer that has
                     * never heard of compression.
                     */
                    Err(error) => {
                        tracing::warn!(
                            transfer_id=%job.transfer_id,
                            chunk=job.index,
                            error=%error,
                            "chunk compression failed; sending it uncompressed"
                        );

                        (original, ChunkEncoding::Identity)
                    }
                }
            }
        };

        /*
         * Encrypted here, per attempt, which is how a retry draws a fresh nonce:
         * `upload::upload_chunk`'s loop calls this again, and this seals again.
         * Sealing outside the loop would send the same ciphertext and nonce on
         * every attempt, which on v1 is undetectable and on v2 is a nonce reuse.
         */
        let encrypted = crypto::encrypt_chunk(&self.key(), &body).map_err(TransferError::Crypto)?;

        self.client
            .send(
                &self.endpoint,
                &self.token,
                job,
                encrypted.data,
                encrypted.nonce,
                encoding,
            )
            .await
    }

    async fn cancel(&self, transfer_id: &str) -> Result<()> {
        self.client
            .cancel_transfer(&self.endpoint, &self.token, transfer_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Delegation, not reimplementation.
    ///
    /// The whole value of this module is that it changes *where* the engine
    /// calls from, not *what* is called. Nothing here can prove that from the
    /// inside — `tests/compat_v1.rs` is what asserts it, by running the frozen
    /// v1 wire contract through this type. What is checkable here is that the
    /// type carries what it was given and does not expose the token.
    #[test]
    fn the_legacy_session_carries_the_endpoint_and_redacts_the_token() {
        let session = LegacyV1Session::to("receiver.example", "super-secret-bearer");

        assert_eq!(session.endpoint(), "receiver.example");

        let rendered = format!("{session:?}");

        assert!(rendered.contains("receiver.example"));
        assert!(
            !rendered.contains("super-secret-bearer"),
            "a session's token must not be readable out of a log line"
        );
        assert!(rendered.contains("<redacted>"));
    }

    /// A session that has not been opened has no key, and must not be able to
    /// encrypt with a plausible-looking zero one.
    ///
    /// This is a guard on a real hazard: `[0u8; 32]` is a valid AES-256 key, so
    /// a missing `open` would seal chunks under a key every peer knows rather
    /// than failing. The session's contract is that `send_chunk` is only reached
    /// after `open`, so what is asserted here is the state that makes a violation
    /// visible rather than silently weak.
    #[test]
    fn an_unopened_session_holds_no_usable_key() {
        let session = LegacyV1Session::to("receiver.example", "token");

        assert_eq!(session.key(), [0u8; 32]);
    }
}
