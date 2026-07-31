use crate::{
    models::chunk::ChunkJob,
    transfer::{crypto, errors::Result, http_client::HttpClient, retry},
};

use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt},
};

pub async fn upload_chunk(
    client: &HttpClient,
    endpoint: &str,
    token: &str,
    job: &ChunkJob,
    transfer_key: &[u8; 32],
    max_retries: u32,
) -> Result<u32> {
    let mut attempt = 0;

    loop {
        let result = async {
            let mut file = fs::File::open(&job.path).await?;

            file.seek(std::io::SeekFrom::Start(job.offset)).await?;

            let mut bytes = vec![0u8; job.length as usize];

            file.read_exact(&mut bytes).await?;

            let encrypted = crypto::encrypt_chunk(transfer_key, &bytes)
                .map_err(|e| crate::transfer::errors::TransferError::Crypto(e))?;

            client
                .send(endpoint, token, job, encrypted.data, encrypted.nonce)
                .await
        }
        .await;

        match result {
            Ok(()) => {
                return Ok(attempt);
            }

            Err(error) if attempt < max_retries => {
                attempt += 1;

                tokio::time::sleep(retry::delay(attempt - 1)).await;

                tracing::warn!(
                    transfer_id=%job.transfer_id,
                    chunk=job.index,
                    attempt,
                    error=%error,
                    "retrying chunk upload"
                );
            }

            Err(error) => {
                return Err(error);
            }
        }
    }
}
