use crate::{
    models::{progress, transfer::TransferStatus},
    transfer::{crypto, events, http_client::StartTransferRequest, merger, state::DownloadState},
};

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use tauri::Manager as _;

use crate::services::keyring_service::KeyringService;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64},
        Arc, Mutex,
    },
};

use std::sync::atomic::Ordering;
use tokio::fs;

#[derive(Serialize, Deserialize)]
pub struct PublicKeyResponse {
    pub public_key: String,
}

#[derive(Clone)]
pub struct ReceiverState {
    pub app: tauri::AppHandle,
    pub root: PathBuf,
    pub guard: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub keys: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, [u8; 32]>>>,

    pub downloads: Arc<tokio::sync::RwLock<HashMap<String, Arc<DownloadState>>>>,
}

impl ReceiverState {
    pub fn new(app: tauri::AppHandle, root: PathBuf) -> Self {
        Self {
            app,
            root,
            guard: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            keys: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            downloads: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }
}

fn header(headers: &HeaderMap, name: &str) -> Result<String, (StatusCode, String)> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or((StatusCode::BAD_REQUEST, format!("missing {name}")))
}

pub async fn start_transfer(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
    Json(request): Json<StartTransferRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    tracing::info!(
        transfer_id = %request.transfer_id,
        sender_public_key_len = request.sender_ephemeral_public_key.len(),
        "received start transfer request"
    );

    /*
     * Validate Authorization header
     */
    if headers.get("Authorization").is_none() {
        tracing::warn!(
            transfer_id = %request.transfer_id,
            "missing Authorization header"
        );

        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    tracing::debug!(
        transfer_id = %request.transfer_id,
        "authorization header present"
    );

    /*
     * Decode sender ephemeral public key
     */
    let sender_public =
        crypto::decode_public_key(&request.sender_ephemeral_public_key).map_err(|e| {
            tracing::warn!(
                transfer_id = %request.transfer_id,
                error = %e,
                "failed to decode sender public key"
            );

            (StatusCode::BAD_REQUEST, e)
        })?;

    tracing::debug!(
        transfer_id = %request.transfer_id,
        "sender public key decoded"
    );

    /*
     * Load receiver private key
     */
    let private_key_string = KeyringService::get_device_private_key(&state.app).map_err(|e| {
        tracing::error!(
            transfer_id = %request.transfer_id,
            error = %e,
            "failed to load receiver private key"
        );

        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    tracing::debug!(
        transfer_id = %request.transfer_id,
        private_key_len = private_key_string.len(),
        "receiver private key loaded"
    );

    let receiver_private = crypto::decode_private_key(&private_key_string).map_err(|e| {
        tracing::error!(
            transfer_id = %request.transfer_id,
            error = %e,
            "failed to decode receiver private key"
        );

        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    tracing::debug!(
        transfer_id = %request.transfer_id,
        "receiver private key decoded"
    );

    /*
     * Derive shared secret
     */
    let shared_secret = crypto::derive_shared_secret(&receiver_private, &sender_public);

    tracing::debug!(
        transfer_id = %request.transfer_id,
        "shared secret derived"
    );

    /*
     * Derive AES-256 transfer key
     */
    let transfer_key = crypto::derive_transfer_key(&shared_secret).map_err(|e| {
        tracing::error!(
            transfer_id = %request.transfer_id,
            error = %e,
            "failed to derive transfer key"
        );

        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    tracing::debug!(
        transfer_id = %request.transfer_id,
        "transfer key derived"
    );

    /*
     * Store transfer key
     */
    {
        let mut keys = state.keys.write().await;

        keys.insert(request.transfer_id.clone(), transfer_key);

        tracing::debug!(
            transfer_id = %request.transfer_id,
            active_transfers = keys.len(),
            "transfer key stored"
        );
    }

    let download = Arc::new(DownloadState {
        transfer_id: request.transfer_id.clone(),

        total_bytes: 0,
        received_bytes: AtomicU64::new(0),

        total_chunks: 0,
        received_chunks: AtomicU64::new(0),

        paused: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),

        started_at: std::time::Instant::now(),

        status: Mutex::new(TransferStatus::Pending),
    });

    /*
     * Register active download
     */
    state
        .downloads
        .write()
        .await
        .insert(request.transfer_id.clone(), download.clone());

    /*
     * Notify frontend so it can immediately show the transfer.
     */
    events::emit_progress(
        &state.app,
        "transfer-progress",
        progress::make_download(&download),
    );

    tracing::info!(
        transfer_id = %request.transfer_id,
        "transfer session established"
    );

    tracing::info!(
        transfer_id = %request.transfer_id,
        "transfer session established"
    );

    Ok(StatusCode::OK)
}

pub async fn receive(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    tracing::info!(body_size = body.len(), "chunk upload request received");

    let transfer = header(&headers, "Transfer-Id").map_err(|e| {
        tracing::warn!(error=?e, "missing Transfer-Id");
        e
    })?;

    let file = header(&headers, "File-Id").map_err(|e| {
        tracing::warn!(
            transfer_id=%transfer,
            error=?e,
            "missing File-Id"
        );
        e
    })?;

    let relative = header(&headers, "Relative-Path").map_err(|e| {
        tracing::warn!(
            transfer_id=%transfer,
            error=?e,
            "missing Relative-Path"
        );
        e
    })?;

    let index = header(&headers, "Chunk-Index")
        .map_err(|e| {
            tracing::warn!(
                transfer_id=%transfer,
                error=?e,
                "missing Chunk-Index"
            );
            e
        })?
        .parse::<u64>()
        .map_err(|e| {
            tracing::warn!(
                transfer_id=%transfer,
                error=%e,
                "invalid Chunk-Index"
            );

            (StatusCode::BAD_REQUEST, format!("invalid Chunk-Index: {e}"))
        })?;

    let total = header(&headers, "Total-Chunks")
        .map_err(|e| {
            tracing::warn!(
                transfer_id=%transfer,
                error=?e,
                "missing Total-Chunks"
            );
            e
        })?
        .parse::<u64>()
        .map_err(|e| {
            tracing::warn!(
                transfer_id=%transfer,
                error=%e,
                "invalid Total-Chunks"
            );

            (
                StatusCode::BAD_REQUEST,
                format!("invalid Total-Chunks: {e}"),
            )
        })?;

    tracing::info!(
        transfer_id=%transfer,
        file_id=%file,
        chunk=index,
        total_chunks=total,
        relative_path=%relative,
        body_size=body.len(),
        "chunk metadata parsed"
    );

    if total == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "total chunks cannot be zero".into(),
        ));
    }

    if index >= total {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("chunk index {index} exceeds total chunks {total}"),
        ));
    }

    if headers.get("Authorization").is_none() {
        tracing::warn!(
            transfer_id=%transfer,
            "missing authorization header"
        );

        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    let relative_path = Path::new(&relative);

    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|c| c == Component::ParentDir)
    {
        tracing::warn!(
            transfer_id=%transfer,
            path=%relative,
            "unsafe path rejected"
        );

        return Err((StatusCode::BAD_REQUEST, "unsafe relative path".into()));
    }

    let transfer_key = {
        let keys = state.keys.read().await;

        match keys.get(&transfer) {
            Some(key) => {
                tracing::debug!(
                    transfer_id=%transfer,
                    "transfer key found"
                );

                *key
            }

            None => {
                tracing::error!(
                    transfer_id=%transfer,
                    "transfer key missing"
                );

                return Err((StatusCode::UNAUTHORIZED, "unknown transfer session".into()));
            }
        }
    };

    let download = {
        let downloads = state.downloads.read().await;

        downloads.get(&transfer).cloned().ok_or((
            StatusCode::NOT_FOUND,
            "download state not found".to_string(),
        ))?
    };

    let hash = Sha256::digest(&transfer_key);

    tracing::info!(
        transfer_key_hash=%hex::encode(hash),
        "sender transfer key fingerprint"
    );

    let nonce_b64 = header(&headers, "Chunk-Nonce").map_err(|e| {
        tracing::warn!(
            transfer_id=%transfer,
            chunk=index,
            error=?e,
            "missing Chunk-Nonce"
        );

        e
    })?;

    let nonce = STANDARD.decode(&nonce_b64).map_err(|e| {
        tracing::warn!(
            transfer_id=%transfer,
            chunk=index,
            error=%e,
            "nonce base64 decode failed"
        );

        (StatusCode::BAD_REQUEST, "invalid nonce encoding".into())
    })?;

    if nonce.len() != 12 {
        tracing::warn!(
            transfer_id=%transfer,
            chunk=index,
            nonce_length=nonce.len(),
            "invalid nonce length"
        );

        return Err((StatusCode::BAD_REQUEST, "nonce must be 12 bytes".into()));
    }

    tracing::debug!(
        transfer_id=%transfer,
        chunk=index,
        nonce=%nonce_b64,
        ciphertext_size=body.len(),
        "attempting chunk decrypt"
    );

    let decrypted = match crypto::decrypt_chunk(&transfer_key, &nonce, &body) {
        Ok(data) => data,

        Err(e) => {
            tracing::error!(
                transfer_id=%transfer,
                chunk=index,
                error=%e,
                nonce=%nonce_b64,
                ciphertext_size=body.len(),
                "chunk decryption failed"
            );

            return Err((
                StatusCode::BAD_REQUEST,
                format!("chunk decrypt failed: {e}"),
            ));
        }
    };

    tracing::info!(
        transfer_id=%transfer,
        chunk=index,
        decrypted_size=decrypted.len(),
        "chunk decrypted"
    );

    let _guard = state.guard.lock().await;

    let parts = state.root.join("incoming").join(&transfer).join(&file);

    fs::create_dir_all(&parts).await.map_err(|e| {
        tracing::error!(
            transfer_id=%transfer,
            error=%e,
            "failed creating chunk directory"
        );

        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("storage error: {e}"),
        )
    })?;

    let part = parts.join(format!("{index}.part"));

    while download.paused.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    if download.cancelled.load(Ordering::Relaxed) {
        return Err((StatusCode::CONFLICT, "transfer cancelled".into()));
    }

    // if fs::metadata(&part).await.is_err() {
    //     fs::write(&part, decrypted).await.map_err(|e| {
    //         download
    //             .received_bytes
    //             .fetch_add(decrypted.len() as u64, Ordering::Relaxed);

    //         download.received_chunks.fetch_add(1, Ordering::Relaxed);

    //         *download.status.lock().unwrap() = TransferStatus::Downloading;

    //         events::emit_progress(
    //             &state.app,
    //             "transfer-progress",
    //             progress::make_download(&download),
    //         );
    //         tracing::error!(
    //             transfer_id=%transfer,
    //             chunk=index,
    //             error=%e,
    //             "failed writing chunk"
    //         );

    //         (
    //             StatusCode::INTERNAL_SERVER_ERROR,
    //             format!("write failed: {e}"),
    //         )
    //     })?;

    //     tracing::info!(
    //         transfer_id=%transfer,
    //         chunk=index,
    //         path=%part.display(),
    //         "chunk saved"
    //     );
    // } else {
    //     tracing::debug!(
    //         transfer_id=%transfer,
    //         chunk=index,
    //         "chunk already exists"
    //     );
    // }

    if fs::metadata(&part).await.is_err() {
        let decrypted_size = decrypted.len() as u64;

        fs::write(&part, decrypted).await.map_err(|e| {
            *download.status.lock().unwrap() = TransferStatus::Failed;

            events::emit_progress(
                &state.app,
                "transfer-failed",
                progress::make_download(&download),
            );

            tracing::error!(
                transfer_id=%transfer,
                chunk=index,
                error=%e,
                "failed writing chunk"
            );

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write failed: {e}"),
            )
        })?;

        // update progress only after successful write
        download
            .received_bytes
            .fetch_add(decrypted_size, Ordering::Relaxed);

        download.received_chunks.fetch_add(1, Ordering::Relaxed);

        *download.status.lock().unwrap() = TransferStatus::Downloading;

        events::emit_progress(
            &state.app,
            "transfer-progress",
            progress::make_download(&download),
        );

        tracing::info!(
            transfer_id=%transfer,
            chunk=index,
            path=%part.display(),
            "chunk saved"
        );
    } else {
        tracing::debug!(
            transfer_id=%transfer,
            chunk=index,
            "chunk already exists"
        );
    }

    let complete = (0..total).all(|i| std::fs::metadata(parts.join(format!("{i}.part"))).is_ok());

    if complete {
        tracing::info!(
            transfer_id=%transfer,
            total_chunks=total,
            "all chunks received"
        );

        let downloads = state.app.path().download_dir().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to locate Downloads folder: {e}"),
            )
        })?;

        let destination = downloads.join(relative_path);
        merger::merge(&parts, &destination, total)
            .await
            .map_err(|e| {
                tracing::error!(
                    transfer_id=%transfer,
                    error=%e,
                    "merge failed"
                );

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("merge failed: {e}"),
                )
            })?;

        *download.status.lock().unwrap() = TransferStatus::Completed;

        events::emit_progress(
            &state.app,
            "transfer-completed",
            progress::make_download(&download),
        );

        state.downloads.write().await.remove(&transfer);

        tracing::info!(
            transfer_id=%transfer,
            destination=%destination.display(),
            "merge complete"
        );

        fs::remove_dir_all(&parts).await.map_err(|e| {
            tracing::warn!(
                transfer_id=%transfer,
                error=%e,
                "failed removing temp chunks"
            );

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cleanup failed: {e}"),
            )
        })?;
    }

    tracing::info!(
        transfer_id=%transfer,
        chunk=index,
        "chunk processing complete"
    );

    Ok(StatusCode::OK)
}

fn io_error(e: std::io::Error) -> (StatusCode, String) {
    tracing::warn!(
        error=%e,
        "receiver write failed"
    );

    (StatusCode::INTERNAL_SERVER_ERROR, "storage failure".into())
}

pub async fn get_public_key(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
) -> Result<Json<PublicKeyResponse>, (StatusCode, String)> {
    if headers.get("Authorization").is_none() {
        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    let public_key = KeyringService::get_device_public_key(&state.app).map_err(|e| {
        tracing::error!(
            error = %e,
            "failed to load device public key"
        );

        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    tracing::info!("device public key requested");

    Ok(Json(PublicKeyResponse { public_key }))
}
