use crate::{
    models::chunk::ChunkJob,
    transfer::{http_client::HttpClient, state::UploadState, upload},
};

use std::sync::Arc;
use tokio::sync::mpsc;

/// Called once per acknowledged chunk, with the job that was sent and how many
/// retries it took.
///
/// Takes the whole job rather than just its length so that the caller can record
/// *which* chunk landed, not only how much — a byte count cannot tell a resume
/// what is still missing.
pub type ChunkAcknowledged = dyn Fn(&ChunkJob, u32) + Send + Sync;

pub async fn run(
    jobs: Vec<ChunkJob>,
    workers: usize,
    state: Arc<UploadState>,
    token: String,
    on_success: Arc<ChunkAcknowledged>,
) -> std::result::Result<(), String> {
    /*
     * Read the key once, before any worker starts. It can change while workers
     * run — a resume that re-establishes a session after the receiver restarted
     * adopts the fresh key the receiver just derived — and a worker encrypting
     * half a transfer with one key and half with another would produce a file
     * the receiver cannot decrypt. The key a run starts with is the key that
     * run uses.
     */
    let transfer_key = state.transfer_key();

    let (tx, rx) = mpsc::channel(jobs.len().max(1));

    for job in jobs {
        if tx.send(job).await.is_err() {
            break;
        }
    }

    drop(tx);

    let rx = Arc::new(tokio::sync::Mutex::new(rx));

    let client = HttpClient::new();

    let mut handles = Vec::new();

    for _ in 0..workers.max(1) {
        let rx = rx.clone();
        let state = state.clone();
        let token = token.clone();
        let client = client.clone();
        let success = on_success.clone();

        handles.push(tokio::spawn(async move {
            loop {
                if state.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(());
                }

                while state.paused.load(std::sync::atomic::Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

                    if state.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        return Ok(());
                    }
                }

                let job = {
                    let mut q = rx.lock().await;
                    q.recv().await
                };

                let Some(job) = job else {
                    return Ok(());
                };

                match upload::upload_chunk(
                    &client,
                    &state.endpoint,
                    &token,
                    &job,
                    &transfer_key,
                    state.max_retries,
                )
                .await
                {
                    Ok(retries) => {
                        success(&job, retries);
                    }

                    Err(e) => {
                        return Err(e.to_string());
                    }
                }
            }
        }));
    }

    for h in handles {
        match h.await {
            Ok(Ok(())) => {}

            Ok(Err(e)) => {
                return Err(e);
            }

            Err(e) => {
                return Err(e.to_string());
            }
        }
    }

    Ok(())
}
