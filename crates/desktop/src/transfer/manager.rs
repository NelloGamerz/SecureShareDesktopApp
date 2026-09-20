use crate::services::local_transfer_file_service::LocalTransferFileService;
use crate::transfer::crypto;
use crate::transfer::http_client::HttpClient;
use crate::{
    models::transfer::{TransferMetadata, TransferStatus, TransferStatusResponse},
    transfer::{
        chunker, constants,
        errors::{Result, TransferError},
        progress, scanner, scheduler,
        state::UploadState,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::Ordering,
    sync::{Arc, Mutex},
};
use vilsend_core::{DomainEvent, EventSink};

#[derive(Debug)]
pub enum TransferEvent {
    Completed(String),
    Failed(String, String),
}

#[derive(Clone)]
pub struct UploadManager {
    transfers: Arc<Mutex<HashMap<String, Arc<UploadState>>>>,
    /// The shell's [`EventSink`]. Replaces the `AppHandle` this struct used to
    /// hold: emitting progress was the only thing the handle was for.
    events: Arc<dyn EventSink>,
    upload_root: PathBuf,
    local_files: Arc<LocalTransferFileService>,
    transfer_events: tokio::sync::mpsc::UnboundedSender<TransferEvent>,
}
impl UploadManager {
    pub fn new(
        events: Arc<dyn EventSink>,
        upload_root: PathBuf,
        local_files: Arc<LocalTransferFileService>,
        transfer_events: tokio::sync::mpsc::UnboundedSender<TransferEvent>,
    ) -> Self {
        Self {
            transfers: Arc::new(Mutex::new(HashMap::new())),
            events,
            upload_root,
            local_files,
            transfer_events,
        }
    }

    pub async fn start(&self, metadata: TransferMetadata) -> Result<TransferStatusResponse> {
        if metadata.transfer_id.trim().is_empty() || metadata.endpoint.trim().is_empty()
        // || metadata.file_paths.is_empty()
        {
            return Err(TransferError::Invalid(
                "transferId, endpoint, and filePaths are required".into(),
            ));
        }
        if self
            .transfers
            .lock()
            .expect("transfer lock poisoned")
            .contains_key(&metadata.transfer_id)
        {
            return Err(TransferError::Invalid("transfer already active".into()));
        }
        // let files = scanner::scan(&metadata.file_paths).await?;
        let local_files = self
            .local_files
            .get_by_transfer_id(&metadata.transfer_id)
            .await
            .map_err(|e| {
                TransferError::Invalid(format!("failed loading local transfer files: {}", e))
            })?;

        if local_files.is_empty() {
            return Err(TransferError::Invalid(
                "no local files found for transfer".into(),
            ));
        }

        let paths = local_files
            .iter()
            .map(|f| f.file_path.clone())
            .collect::<Vec<_>>();

        let files = scanner::scan(&paths).await?;
        let chunk_size = metadata
            .chunk_size
            .unwrap_or(constants::DEFAULT_CHUNK_SIZE)
            .max(1);
        let jobs = chunker::jobs(&metadata.transfer_id, &files, chunk_size);
        let client = HttpClient::new();

        let receiver_public_key = client
            .get_public_key(&metadata.endpoint, &metadata.auth_token)
            .await?;

        let receiver_public =
            crypto::decode_public_key(&receiver_public_key).map_err(TransferError::Crypto)?;

        let ephemeral = crypto::generate_ephemeral_keypair();

        let shared = crypto::derive_shared_secret(&ephemeral.private_key, &receiver_public);

        let transfer_key =
            crypto::derive_transfer_key(&shared).map_err(|e| TransferError::Invalid(e))?;

        let hash = Sha256::digest(&transfer_key);

        tracing::info!(
            transfer_key_hash=%hex::encode(hash),
            "sender transfer key fingerprint"
        );

        let total_bytes = files.iter().map(|f| f.size).sum();

        client
            .start_transfer(
                &metadata.endpoint,
                &metadata.auth_token,
                &metadata.transfer_id,
                &crypto::encode_public_key(&ephemeral.public_key),
                total_bytes,
            )
            .await?;
        let state = Arc::new(UploadState {
            transfer_id: metadata.transfer_id.clone(),
            transfer_key,
            endpoint: metadata.endpoint.clone(),
            network_type: metadata.network_type.clone(),
            total_bytes,
            uploaded_bytes: 0.into(),
            total_chunks: jobs.len() as u64,
            uploaded_chunks: 0.into(),
            paused: false.into(),
            cancelled: false.into(),
            retry_count: 0.into(),
            max_retries: metadata
                .max_retries
                .unwrap_or(constants::DEFAULT_MAX_RETRIES),
            status: Mutex::new(TransferStatus::Queued),
            started_at: std::time::Instant::now(),
        });
        let response = state.snapshot();
        self.transfers
            .lock()
            .expect("transfer lock poisoned")
            .insert(metadata.transfer_id.clone(), state.clone());
        let manager = self.clone();
        let concurrency = metadata
            .concurrency
            .unwrap_or(constants::DEFAULT_CONCURRENCY);
        tokio::spawn(async move {
            *state.status.lock().expect("status lock poisoned") = TransferStatus::Uploading;
            let s = state.clone();
            let events = Arc::clone(&manager.events);
            let on_success = Arc::new(move |length: usize, retries: u32| {
                s.uploaded_bytes.fetch_add(length as u64, Ordering::Relaxed);
                s.uploaded_chunks.fetch_add(1, Ordering::Relaxed);
                s.retry_count.fetch_add(retries, Ordering::Relaxed);
                events.emit(DomainEvent::TransferProgress {
                    progress: progress::make(&s),
                });
            });
            let result = scheduler::run(
                jobs,
                concurrency,
                state.clone(),
                metadata.auth_token,
                on_success,
            )
            .await;
            if state.cancelled.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(()) => {
                    *state.status.lock().expect("status lock poisoned") = TransferStatus::Completed;
                    manager.events.emit(DomainEvent::TransferCompleted {
                        progress: progress::make(&state),
                    });

                    let _ = manager
                        .transfer_events
                        .send(TransferEvent::Completed(state.transfer_id.clone()));
                }
                Err(e) => {
                    *state.status.lock().expect("status lock poisoned") = TransferStatus::Failed;
                    tracing::error!(transfer_id=%state.transfer_id,error=%e,"transfer failed");
                    manager.events.emit(DomainEvent::TransferFailed {
                        progress: progress::make(&state),
                    });
                    let _ = manager.transfer_events.send(TransferEvent::Failed(
                        state.transfer_id.clone(),
                        e.to_string(),
                    ));
                }
            }
        });
        Ok(response)
    }
    pub fn pause(&self, id: &str) -> Result<TransferStatusResponse> {
        let s = self.get(id)?;
        s.paused.store(true, Ordering::Relaxed);
        *s.status.lock().expect("status lock poisoned") = TransferStatus::Paused;
        self.events.emit(DomainEvent::TransferPaused {
            progress: progress::make(&s),
        });
        Ok(s.snapshot())
    }
    pub fn resume(&self, id: &str) -> Result<TransferStatusResponse> {
        let s = self.get(id)?;
        s.paused.store(false, Ordering::Relaxed);
        *s.status.lock().expect("status lock poisoned") = TransferStatus::Uploading;
        self.events.emit(DomainEvent::TransferResumed {
            progress: progress::make(&s),
        });
        Ok(s.snapshot())
    }
    pub async fn cancel(&self, id: &str) -> Result<TransferStatusResponse> {
        let s = self.get(id)?;
        s.cancelled.store(true, Ordering::Relaxed);
        *s.status.lock().expect("status lock poisoned") = TransferStatus::Cancelled;
        let _ = tokio::fs::remove_dir_all(self.upload_root.join("incoming").join(id)).await;
        self.events.emit(DomainEvent::TransferCancelled {
            progress: progress::make(&s),
        });
        self.transfers
            .lock()
            .expect("transfer lock poisoned")
            .remove(id);
        Ok(s.snapshot())
    }
    pub fn status(&self, id: &str) -> Result<TransferStatusResponse> {
        Ok(self.get(id)?.snapshot())
    }
    fn get(&self, id: &str) -> Result<Arc<UploadState>> {
        self.transfers
            .lock()
            .expect("transfer lock poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| TransferError::NotFound(id.into()))
    }
}
