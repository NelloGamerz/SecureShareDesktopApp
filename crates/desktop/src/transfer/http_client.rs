use crate::{
    models::chunk::ChunkJob,
    transfer::errors::{Result, TransferError},
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

#[derive(Clone)]
pub struct HttpClient(Client);

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    pub fn new() -> Self {
        Self(Client::new())
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
    ) -> Result<()> {
        let base = Self::normalize_endpoint(endpoint);
        let url = format!("{base}/transfer/chunk");

        let nonce_encoded = STANDARD.encode(&nonce);

        tracing::debug!(
            transfer_id=%job.transfer_id,
            chunk=job.index,
            body_size=body.len(),
            nonce_size=nonce.len(),
            "sending encrypted chunk"
        );

        let response = self
            .0
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Transfer-Id", &job.transfer_id)
            .header("File-Id", &job.file_id)
            .header("Chunk-Index", job.index)
            .header("Total-Chunks", job.total)
            .header("Relative-Path", &job.relative_path)
            .header("Chunk-Nonce", nonce_encoded)
            .header("Encryption", "AES-256-GCM")
            .body(body)
            .send()
            .await?;

        let status = response.status();

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

    pub async fn get_public_key(&self, endpoint: &str, token: &str) -> Result<String> {
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

        let response: PublicKeyResponse = response
            .json()
            .await
            .map_err(|e| TransferError::Receiver(format!("invalid public key response: {}", e)))?;

        tracing::debug!("receiver public key fetched successfully");

        Ok(response.public_key)
    }
}
