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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublicKeyResponse {
    pub public_key: String,
}

#[derive(Clone)]
pub struct HttpClient(Client);

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
    ) -> Result<()> {
        let base = Self::normalize_endpoint(endpoint);
        let url = format!("{base}/transfer/start");

        let request = StartTransferRequest {
            transfer_id: transfer_id.to_owned(),
            sender_ephemeral_public_key: sender_ephemeral_public_key.to_owned(),
        };

        let response = self
            .0
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(TransferError::Receiver(format!(
                "Handshake failed: HTTP {} - {}",
                status, body
            )))
        }
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
