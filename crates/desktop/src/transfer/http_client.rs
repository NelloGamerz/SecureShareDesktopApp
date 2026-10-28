use crate::{
    models::chunk::ChunkJob,
    transfer::{
        compression::{ChunkEncoding, CHUNK_ENCODING_HEADER, NEGOTIATION_HEADER},
        constants::{LEGACY_CHUNK_SIZE, MAX_CHUNK_BYTES_HEADER, MAX_CHUNK_SIZE},
        errors::{Result, TransferError},
    },
};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::Client;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct StartTransferRequest {
    pub transfer_id: String,
    pub sender_ephemeral_public_key: String,
    pub file_size: u64,
    /// What this transfer's files are supposed to hash to. See
    /// [`crate::transfer::integrity`].
    ///
    /// Additive and optional, which is the only kind of change the transfer
    /// metadata may take: a receiver that predates the field ignores it, and a
    /// sender that has nothing to describe omits it entirely, leaving the
    /// request byte-for-byte what it always was. It travels here because the
    /// start request is the transfer's metadata message — digests that arrive
    /// separately from the session they describe would have to be matched to it,
    /// and the receiver would have to guess whether they were coming at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<crate::transfer::integrity::ExpectedTransfer>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublicKeyResponse {
    pub public_key: String,
}

/// What `GET /transfer/public-key` told us: the key, and what the receiver can
/// decode.
///
/// The encoding is a capability of the receiver, not of this transfer, which is
/// why it rides on the handshake that every session performs. A receiver that
/// predates chunk compression sends no [`NEGOTIATION_HEADER`], and the sender
/// then keeps sending identity chunks.
#[derive(Debug, Clone)]
pub struct ReceiverKey {
    pub public_key: String,
    pub chunk_encoding: ChunkEncoding,
    /// The largest chunk body the receiver said it will accept.
    ///
    /// Absent means an installed receiver: its body limit is 10 MiB, so the
    /// sender plans [`LEGACY_CHUNK_SIZE`] chunks rather than assuming the new
    /// default and being refused with `413`.
    pub max_chunk_bytes: usize,
}

/// The receiver's answer to "which chunks of this transfer do you already
/// hold?".
///
/// The sender's own copy of the shape the receiver serves; neither side shares
/// a type, as with [`StartTransferRequest`] and [`PublicKeyResponse`].
#[derive(Debug, Clone, Deserialize)]
pub struct TransferResumeResponse {
    pub transfer_id: String,
    pub files: Vec<FileResumeState>,
}

/// One file's completed chunks, as reported.
#[derive(Debug, Clone, Deserialize)]
pub struct FileResumeState {
    pub file_id: String,
    pub chunks: Vec<ChunkResumeState>,
}

/// One chunk the receiver says it holds.
///
/// Every field is a claim, not a fact: nothing here may be acted on until it has
/// been checked against what the sender actually has. See
/// `crate::transfer::resume`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChunkResumeState {
    pub chunk_index: u64,
    pub chunk_length: u64,
    pub chunk_hash: String,
}

/// What happened when the sender asked the receiver to open a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStart {
    /// The receiver accepted the handshake and derived a key from the ephemeral
    /// public key we just sent. Used by a transfer whose receiver restarted.
    Established,
    /// The receiver already had this transfer live, so it kept the key it
    /// already had. The sender must continue with the key it used before.
    AlreadyActive,
}

/// What a receiver advertised beside its public key.
///
/// Absent or unparseable headers are not errors: they are how an installed
/// receiver answers, and the safe reading of silence is always the old
/// behaviour — identity chunks at the legacy size.
fn advertised_limits(headers: &reqwest::header::HeaderMap) -> (ChunkEncoding, usize) {
    let encoding = headers
        .get(NEGOTIATION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(ChunkEncoding::parse)
        .unwrap_or_default();

    /*
     * Clamped on this side as well: a peer cannot raise this sender's memory
     * bound past `MAX_CHUNK_SIZE`, whatever number it puts in the header.
     */
    let max_chunk_bytes = headers
        .get(MAX_CHUNK_BYTES_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(MAX_CHUNK_SIZE))
        .unwrap_or(LEGACY_CHUNK_SIZE);

    (encoding, max_chunk_bytes)
}

#[derive(Clone)]
pub struct HttpClient(Client);

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// The response header the receiver puts on a refused chunk to name the chunks
/// it no longer holds.
pub const MISSING_CHUNKS_HEADER: &str = "vilsend-missing-chunks";

impl HttpClient {
    /// One client per session, shared by every chunk.
    ///
    /// The pool is what makes a transfer one connection (or a few) instead of
    /// one connect and TLS handshake per chunk; the idle timeout is long enough
    /// that a slow chunk cannot evict the connection the next one will use.
    pub fn new() -> Self {
        let mut builder = Client::builder()
            // Enough idle connections for a transfer's own concurrency, and a
            // pool that outlives the gap between two chunks of a slow link.
            .pool_max_idle_per_host(crate::transfer::constants::DEFAULT_CONCURRENCY * 2)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true);

        /*
         * HTTP/1.1 by default, deliberately.
         *
         * Over TLS, reqwest negotiates HTTP/2 with the receiver's edge, which
         * multiplexes every chunk onto *one* TCP connection. That connection's
         * throughput is then the transfer's ceiling no matter how wide the
         * scheduler's window is. HTTP/1.1 lets the pool open one connection per
         * in-flight chunk, so the window is backed by real connections.
         *
         * Plaintext LAN receivers speak HTTP/1.1 either way; this only changes
         * the TLS path. `VILSEND_HTTP2=negotiate` restores the old ALPN
         * behaviour so the two can be benchmarked against the same receiver.
         */
        match std::env::var("VILSEND_HTTP2").as_deref() {
            Ok("prior-knowledge") => {
                builder = builder.http2_prior_knowledge().http2_adaptive_window(true);
            }

            Ok("negotiate") => {
                builder = builder.http2_adaptive_window(true);
            }

            _ => {
                builder = builder.http1_only();
            }
        }

        Self(
            builder
                .build()
                .expect("the HTTP client configuration is valid"),
        )
    }

    fn normalize_endpoint(endpoint: &str) -> String {
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            endpoint.trim_end_matches('/').to_string()
        } else {
            format!("https://{}", endpoint.trim_end_matches('/'))
        }
    }

    pub async fn start_transfer(
        &self,
        endpoint: &str,
        token: &str,
        transfer_id: &str,
        sender_ephemeral_public_key: &str,
        file_size: u64,
        expected: Option<&crate::transfer::integrity::ExpectedTransfer>,
    ) -> Result<SessionStart> {
        let base = Self::normalize_endpoint(endpoint);
        let url = format!("{base}/transfer/start");

        let request = StartTransferRequest {
            transfer_id: transfer_id.to_owned(),
            sender_ephemeral_public_key: sender_ephemeral_public_key.to_owned(),
            file_size,
            expected: expected.cloned(),
        };

        let response = self
            .0
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&request)
            .send()
            .await?;

        let status = response.status();

        if status.is_success() {
            return Ok(SessionStart::Established);
        }

        /*
         * The receiver answers `CONFLICT` when it already has this transfer
         * live, which is what a sender that died and came back finds: the
         * receiver never noticed. It kept the key it already had, so the sender
         * has to keep using the key it already had too.
         */
        if status == reqwest::StatusCode::CONFLICT {
            return Ok(SessionStart::AlreadyActive);
        }

        let body = response.text().await.unwrap_or_default();

        Err(TransferError::Receiver(format!(
            "Handshake failed: HTTP {} - {}",
            status, body
        )))
    }

    /// Ask the receiver which chunks of a transfer it already holds.
    ///
    /// `Ok(None)` means the receiver has nothing to say about this transfer —
    /// it answered `NOT_FOUND`, either because it has never seen the transfer or
    /// because it predates this endpoint. Both mean the same thing to the
    /// caller: nothing can be skipped, so send everything.
    pub async fn transfer_status(
        &self,
        endpoint: &str,
        token: &str,
        transfer_id: &str,
    ) -> Result<Option<TransferResumeResponse>> {
        let base = Self::normalize_endpoint(endpoint);
        let url = format!("{base}/transfer/status");

        tracing::debug!(transfer_id, "requesting receiver resume state");

        let response = self
            .0
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Transfer-Id", transfer_id)
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            tracing::info!(
                transfer_id,
                "receiver has no record of this transfer; every chunk will be sent"
            );

            return Ok(None);
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();

            tracing::error!(transfer_id, %status, error = %body, "receiver refused a resume query");

            return Err(TransferError::Receiver(format!(
                "resume query failed: HTTP {status} - {body}"
            )));
        }

        response
            .json::<TransferResumeResponse>()
            .await
            .map(Some)
            .map_err(|error| TransferError::Receiver(format!("invalid resume response: {error}")))
    }

    /// Tell a receiver to drop a transfer and everything it holds for it.
    ///
    /// `Ok(())` whether or not the receiver knew the transfer: cancelling is
    /// idempotent, and a receiver with nothing to remove has already done what
    /// was asked. A receiver that predates the endpoint answers `404`, which is
    /// the same outcome and is why the sender treats this as best-effort — see
    /// [`crate::transfer::manager::UploadManager::cancel`].
    pub async fn cancel_transfer(
        &self,
        endpoint: &str,
        token: &str,
        transfer_id: &str,
    ) -> Result<()> {
        let base = Self::normalize_endpoint(endpoint);
        let url = format!("{base}/transfer/cancel");

        tracing::debug!(transfer_id, "asking the receiver to cancel the transfer");

        let response = self
            .0
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Transfer-Id", transfer_id)
            .send()
            .await?;

        let status = response.status();

        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }

        let body = response.text().await.unwrap_or_default();

        tracing::error!(transfer_id, %status, error = %body, "receiver refused a cancellation");

        Err(TransferError::Receiver(format!(
            "cancel failed: HTTP {status} - {body}"
        )))
    }

    pub async fn send(
        &self,
        endpoint: &str,
        token: &str,
        job: &ChunkJob,
        body: Vec<u8>,
        nonce: Vec<u8>,
        encoding: ChunkEncoding,
    ) -> Result<()> {
        let base = Self::normalize_endpoint(endpoint);
        let url = format!("{base}/transfer/chunk");

        let nonce_encoded = STANDARD.encode(&nonce);

        tracing::debug!(
            transfer_id=%job.transfer_id,
            chunk=job.index,
            body_size=body.len(),
            nonce_size=nonce.len(),
            encoding=encoding.as_str(),
            "sending encrypted chunk"
        );

        let mut request = self
            .0
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Transfer-Id", &job.transfer_id)
            .header("File-Id", &job.file_id)
            .header("Chunk-Index", job.index)
            .header("Total-Chunks", job.total)
            .header("Relative-Path", &job.relative_path)
            .header("Chunk-Nonce", nonce_encoded)
            .header("Encryption", "AES-256-GCM");

        /*
         * Only present when the body is compressed. An identity chunk carries
         * no header, so the request is byte-for-byte what a receiver without
         * compression support has always received.
         */
        if encoding != ChunkEncoding::Identity {
            request = request.header(CHUNK_ENCODING_HEADER, encoding.as_str());
        }

        let response = request.body(body).send().await?;

        let status = response.status();

        /*
         * A refused chunk that names the chunks the receiver no longer holds.
         * The sender resends exactly those; the rest of the file is kept.
         */
        if status == reqwest::StatusCode::CONFLICT {
            let missing: Vec<u64> = response
                .headers()
                .get(MISSING_CHUNKS_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(|value| {
                    value
                        .split(',')
                        .filter_map(|index| index.trim().parse::<u64>().ok())
                        .collect()
                })
                .unwrap_or_default();

            if !missing.is_empty() {
                return Err(TransferError::ChunksMissing { missing });
            }
        }

        if status.is_success() {
            tracing::debug!(
                transfer_id=%job.transfer_id,
                chunk=job.index,
                "chunk accepted by receiver"
            );

            return Ok(());
        }

        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read receiver error".into());

        tracing::error!(
            transfer_id=%job.transfer_id,
            chunk=job.index,
            status=%status,
            error=%error_body,
            "receiver rejected chunk"
        );

        Err(TransferError::Receiver(format!(
            "receiver HTTP {}: {}",
            status, error_body
        )))
    }

    pub async fn get_public_key(&self, endpoint: &str, token: &str) -> Result<ReceiverKey> {
        let base = Self::normalize_endpoint(endpoint);
        let url = format!("{base}/transfer/public-key");

        tracing::debug!(
            endpoint = %endpoint,
            "fetching receiver public key"
        );

        let response = self
            .0
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();

            tracing::error!(
                status = %status,
                error = %body,
                "failed to fetch receiver public key"
            );

            return Err(TransferError::Receiver(format!(
                "failed to fetch receiver public key: HTTP {} - {}",
                status, body
            )));
        }

        let (chunk_encoding, max_chunk_bytes) = advertised_limits(response.headers());

        let response: PublicKeyResponse = response
            .json()
            .await
            .map_err(|e| TransferError::Receiver(format!("invalid public key response: {}", e)))?;

        tracing::debug!(
            chunk_encoding = chunk_encoding.as_str(),
            max_chunk_bytes,
            "receiver public key fetched successfully"
        );

        Ok(ReceiverKey {
            public_key: response.public_key,
            chunk_encoding,
            max_chunk_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    /// An installed receiver sends neither header. Reading that as anything but
    /// the old behaviour would break every peer that has not been updated.
    #[test]
    fn silence_is_identity_at_the_legacy_size() {
        assert_eq!(
            advertised_limits(&HeaderMap::new()),
            (ChunkEncoding::Identity, LEGACY_CHUNK_SIZE)
        );
    }

    #[test]
    fn advertised_limits_are_read_off_the_handshake() {
        let mut headers = HeaderMap::new();

        headers.insert(
            NEGOTIATION_HEADER,
            HeaderValue::from_static(crate::transfer::compression::ZSTD_NAME),
        );

        headers.insert(MAX_CHUNK_BYTES_HEADER, HeaderValue::from_static("16777216"));

        assert_eq!(
            advertised_limits(&headers),
            (ChunkEncoding::Zstd, 16 * 1024 * 1024)
        );
    }

    /// A peer cannot talk this build into a larger memory bound than it allows,
    /// and a nonsense value is ignored rather than adopted.
    #[test]
    fn an_oversized_or_malformed_chunk_limit_is_clamped_or_ignored() {
        for (value, expected) in [
            ("999999999999", MAX_CHUNK_SIZE),
            ("0", LEGACY_CHUNK_SIZE),
            ("not a number", LEGACY_CHUNK_SIZE),
        ] {
            let mut headers = HeaderMap::new();

            headers.insert(MAX_CHUNK_BYTES_HEADER, HeaderValue::from_static(value));

            assert_eq!(
                advertised_limits(&headers).1,
                expected,
                "header value {value:?} was not handled as expected"
            );
        }
    }
}
