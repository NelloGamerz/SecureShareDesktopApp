use crate::{
    models::chunk::ChunkJob,
    transfer::{http_client::HttpClient, state::UploadState, upload},
};

use std::sync::Arc;
use tokio::sync::mpsc;

pub async fn run(
    jobs: Vec<ChunkJob>,
    workers: usize,
    state: Arc<UploadState>,
    token: String,
    on_success: Arc<dyn Fn(usize, u32) + Send + Sync>,
) -> std::result::Result<(), String> {
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
                    &state.transfer_key,
                    state.max_retries,
                )
                .await
                {
                    Ok(retries) => {
                        success(job.length, retries);
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
