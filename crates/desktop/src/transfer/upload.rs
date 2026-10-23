use crate::{
    models::chunk::ChunkJob,
    transfer::{errors::Result, retry, session::ProtocolSession},
};

use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt},
};

/// Read, encrypt and send one chunk, retrying the whole attempt on failure.
///
/// Takes a [`ProtocolSession`] rather than an [`crate::transfer::http_client::HttpClient`]
/// plus an endpoint plus a token, so that which wire carries the chunk is the
/// session's business and not this function's. The retry loop and the read are
/// unchanged — only *where the bytes go*, and *how they are sealed*, moved
/// behind the seam.
pub async fn upload_chunk(
    session: &dyn ProtocolSession,
    job: &ChunkJob,
    max_retries: u32,
) -> Result<u32> {
    let mut attempt = 0;

    loop {
        let result = async {
            let mut file = fs::File::open(&job.path).await?;

            file.seek(std::io::SeekFrom::Start(job.offset)).await?;

            let mut bytes = vec![0u8; job.length];

            file.read_exact(&mut bytes).await?;

            /*
             * Plaintext, not ciphertext.
             *
             * The session seals it, because the AEAD's associated data is a
             * property of the protocol — v2 binds the transfer, file, index,
             * path and protocol version into the tag, and v1 passes none — and
             * because the two versions derive their key differently. Sealing
             * here would mean the engine had to know which protocol it was on.
             */
            session.send_chunk(job, bytes).await
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
