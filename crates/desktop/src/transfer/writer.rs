use crate::{
    models::{progress, transfer::TransferStatus},
    transfer::{
        cleanup, crypto,
        errors::{Result as TransferResult, TransferError},
        http_client::StartTransferRequest,
        manifest, merger,
        state::DownloadState,
    },
};
use vilsend_core::{DomainEvent, EventSink};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use tauri::Manager as _;
use tauri_plugin_store::StoreExt;

use crate::services::keyring_service::KeyringService;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc, Mutex,
    },
    time::{Duration, SystemTime},
};
use sysinfo::Disks;

use std::sync::atomic::Ordering;
use tokio::fs;

#[derive(Serialize, Deserialize)]
pub struct PublicKeyResponse {
    pub public_key: String,
}

/// What the receiver answers when a sender asks what it already holds.
///
/// Carries no chunk data. It identifies chunks — file, index, length, digest —
/// so the sender can decide which of its own chunks not to send; the ones still
/// missing travel through `/transfer/chunk` exactly as they always have.
#[derive(Debug, Serialize, Deserialize)]
pub struct TransferResumeResponse {
    /// The transfer this report is about.
    pub transfer_id: String,
    /// Completed chunks, grouped by file. A file with nothing completed is
    /// absent rather than present and empty, which reads as "send all of it".
    pub files: Vec<FileResumeState>,
}

/// One file's completed chunks.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileResumeState {
    /// The file these chunks belong to.
    pub file_id: String,
    /// Its completed chunks, in index order.
    pub chunks: Vec<ChunkResumeState>,
}

/// One completed chunk. The same four facts the manifest records, so a sender
/// can match a chunk it holds against the one the receiver has.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkResumeState {
    /// The chunk's index within its file.
    pub chunk_index: u64,
    /// How many bytes the receiver holds for it.
    pub chunk_length: u64,
    /// Hex SHA-256 of the plaintext the receiver holds.
    pub chunk_hash: String,
}

/// What the receiver needs from the machine it runs on.
///
/// Three things, and they are the only parts of the receiver that touch the
/// shell: where a finished file goes, how much room is left there, and this
/// device's long-term key. They are a port rather than the `AppHandle` itself so
/// that a receiver can be run — and driven end to end, over a real socket, by
/// the real sender — without a Tauri application behind it. See
/// `transfer::resumability_tests`.
///
/// The production implementation is [`TauriReceiverHost`]: the settings store
/// for the download directory, the disk for the space check, and the keychain
/// for the key.
pub trait ReceiverHost: Send + Sync + 'static {
    /// The directory a completed transfer's files are written into.
    fn downloads_root(&self) -> Result<PathBuf, String>;

    /// How many bytes are free on the filesystem holding `path`.
    ///
    /// Used to refuse a transfer the receiver cannot store before a byte of it
    /// crosses the wire.
    fn available_space(&self, path: &Path) -> Result<u64, String>;

    /// This device's long-term private key, as the keychain holds it.
    fn device_private_key(&self) -> Result<String, String>;

    /// This device's long-term public key, base64, as a sender fetches it.
    fn device_public_key(&self) -> Result<String, String>;
}

/// The receiver's host, backed by the running application.
pub struct TauriReceiverHost {
    app: tauri::AppHandle,
}

impl TauriReceiverHost {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl ReceiverHost for TauriReceiverHost {
    fn downloads_root(&self) -> Result<PathBuf, String> {
        resolve_download_root(&self.app)
    }

    fn available_space(&self, path: &Path) -> Result<u64, String> {
        check_receiver_storage(path)
    }

    fn device_private_key(&self) -> Result<String, String> {
        KeyringService::get_device_private_key(&self.app)
    }

    fn device_public_key(&self) -> Result<String, String> {
        KeyringService::get_device_public_key(&self.app)
    }
}

#[derive(Clone)]
pub struct ReceiverState {
    /// The machine the receiver is running on: where files go, whether there is
    /// room for them, and the key they are decrypted with.
    pub host: Arc<dyn ReceiverHost>,
    /// The shell's [`EventSink`]. Emitting progress goes here rather than
    /// through the application handle.
    pub events: Arc<dyn EventSink>,
    pub root: PathBuf,
    pub guard: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub keys: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, [u8; 32]>>>,

    pub downloads: Arc<tokio::sync::RwLock<HashMap<String, Arc<DownloadState>>>>,
    /// Which chunks each transfer has actually completed, keyed by transfer id.
    ///
    /// Backed by `manifest.json` in the transfer's directory. The map is a
    /// cache in front of that file — the file is what survives a restart, and
    /// is loaded the first time a transfer is touched after one. See
    /// [`crate::transfer::manifest`].
    pub manifests: Arc<tokio::sync::Mutex<HashMap<String, manifest::Manifest>>>,
    /// Transfers whose assembly has been refused since the session began.
    ///
    /// Set when an assembly attempt drops a chunk — the file is not the one the
    /// sender described, or a chunk no longer verifies — and cleared when the
    /// sender opens a session, which is the moment it has asked what is held and
    /// is about to send exactly that. While it is set, chunk requests for the
    /// transfer are refused; see the top of [`receive`].
    ///
    /// It only has to outlive the sender's retries, which it does. A receiver
    /// restart inside that window would lose it, and also lose the transfer key
    /// the sender's retries are being encrypted with — so the retry fails either
    /// way, and the resume that follows asks again.
    pub refused: Arc<tokio::sync::Mutex<HashSet<String>>>,
}

impl ReceiverState {
    pub fn new(host: Arc<dyn ReceiverHost>, events: Arc<dyn EventSink>, root: PathBuf) -> Self {
        Self {
            host,
            events,
            root,
            guard: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            keys: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            downloads: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            manifests: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            refused: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        }
    }

    /// The directory a transfer's parts and manifest live in.
    fn transfer_dir(&self, transfer_id: &str) -> PathBuf {
        self.root.join("incoming").join(transfer_id)
    }
}

/// Every route the receiver serves, in one place.
///
/// The shell binds this to the receiver port, and the end-to-end tests run the
/// same router over an ephemeral one — so a handler that works in a test is a
/// handler the application serves, and the two cannot drift apart.
pub fn router(state: ReceiverState) -> axum::Router {
    axum::Router::new()
        .route("/transfer/public-key", axum::routing::get(get_public_key))
        .route("/transfer/start", axum::routing::post(start_transfer))
        .route("/transfer/chunk", axum::routing::post(receive))
        .route("/transfer/status", axum::routing::get(transfer_status))
        .route("/transfer/cancel", axum::routing::post(cancel_transfer))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(state)
}

fn header(headers: &HeaderMap, name: &str) -> Result<String, (StatusCode, String)> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or((StatusCode::BAD_REQUEST, format!("missing {name}")))
}

fn resolve_download_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let store = app
        .store("settings.json")
        .map_err(|error| format!("failed to open settings store: {error}"))?;

    if let Some(value) = store.get("default_download_location") {
        if let Some(path) = value.as_str() {
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    app.path()
        .download_dir()
        .map_err(|error| format!("failed to resolve Downloads folder: {error}"))
}

fn check_receiver_storage(path: &Path) -> Result<u64, String> {
    let path = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };

    let disks = Disks::new_with_refreshed_list();

    for disk in disks.list() {
        if path.starts_with(disk.mount_point()) {
            return Ok(disk.available_space());
        }
    }

    Err("unable to determine receiver storage availability".into())
}

/// Report a storage failure while handling a chunk.
///
/// The download is marked failed and the frontend told, because a chunk that
/// could not be written or could not be discarded leaves the transfer in a
/// state the user needs to see rather than one that silently retries forever.
fn storage_failure(
    state: &ReceiverState,
    download: &std::sync::Arc<DownloadState>,
    transfer_id: &str,
    chunk: u64,
    message: &str,
    error: std::io::Error,
) -> (StatusCode, String) {
    *download.status.lock().unwrap() = TransferStatus::Failed;

    state.events.emit(DomainEvent::TransferFailed {
        progress: progress::make_download(download),
    });

    tracing::error!(
        transfer_id,
        chunk,
        error = %error,
        "{message}"
    );

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("{message}: {error}"),
    )
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
     * A session is the sender asking again. Whatever it was refused before, this
     * session starts from what the receiver actually holds — which is what the
     * sender is about to ask for and re-send.
     */
    state.refused.lock().await.remove(&request.transfer_id);

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

    if state
        .downloads
        .read()
        .await
        .contains_key(&request.transfer_id)
    {
        tracing::warn!(
            transfer_id = %request.transfer_id,
            "duplicate transfer request rejected"
        );

        let duplicate = Arc::new(DownloadState {
            transfer_id: request.transfer_id.clone(),
            total_bytes: request.file_size,
            received_bytes: AtomicU64::new(0),
            received_chunks: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            started_at: std::time::Instant::now(),
            status: Mutex::new(TransferStatus::Failed),
        });

        state.events.emit(DomainEvent::TransferFailed {
            progress: progress::make_download(&duplicate),
        });

        return Err((
            StatusCode::CONFLICT,
            format!("transfer {} is already active", request.transfer_id),
        ));
    }

    let destination_root = state.host.downloads_root().map_err(|e| {
        tracing::error!(
            transfer_id = %request.transfer_id,
            error = %e,
            "failed to resolve receiver download root"
        );

        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    let available_space = state.host.available_space(&destination_root).map_err(|e| {
        tracing::error!(
            transfer_id = %request.transfer_id,
            error = %e,
            "failed to determine receiver storage availability"
        );

        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    if available_space < request.file_size {
        tracing::warn!(
            transfer_id = %request.transfer_id,
            required_bytes = request.file_size,
            available_bytes = available_space,
            "receiver does not have enough free storage"
        );

        let insufficient = Arc::new(DownloadState {
            transfer_id: request.transfer_id.clone(),
            total_bytes: request.file_size,
            received_bytes: AtomicU64::new(0),
            received_chunks: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            started_at: std::time::Instant::now(),
            status: Mutex::new(TransferStatus::Failed),
        });

        state.events.emit(DomainEvent::TransferFailed {
            progress: progress::make_download(&insufficient),
        });

        return Err((
            StatusCode::INSUFFICIENT_STORAGE,
            format!(
                "insufficient storage for receiver: requires {} bytes, has {} bytes available",
                request.file_size, available_space
            ),
        ));
    }

    /*
     * Load receiver private key
     */
    let private_key_string = state.host.device_private_key().map_err(|e| {
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

    /*
     * Recover what this receiver already has of the transfer.
     *
     * A sender that re-offers a transfer — because the connection dropped, or
     * the app was restarted — arrives here again. The manifest on disk is what
     * remembers which chunks were completed before that, so loading it now
     * means the first chunk to arrive is measured against real progress rather
     * than an empty record. Nothing about the request or the response changes;
     * this is the receiver looking at its own storage.
     */
    /*
     * The digests the sender describes for its files.
     *
     * They are the only expectation the receiver did not write for itself, so
     * they are the only thing an assembled file can be checked against — see
     * [`crate::transfer::integrity`]. A description for a different transfer is
     * refused rather than kept: it would check this transfer's file against
     * another's bytes and fail it for the wrong reason.
     */
    let described = match request.expected {
        Some(expected) if expected.transfer_id == request.transfer_id => Some(expected),

        Some(expected) => {
            tracing::warn!(
                transfer_id = %request.transfer_id,
                described = %expected.transfer_id,
                "the sender described digests for a different transfer; ignoring them"
            );

            None
        }

        None => None,
    };

    let (recovered, expected_changed) = {
        let mut manifests = state.manifests.lock().await;

        let mut known = match manifests.get(&request.transfer_id).cloned() {
            Some(known) => known,

            None => {
                let transfer_dir = state.transfer_dir(&request.transfer_id);

                let recovered =
                    manifest::load_or_default(&transfer_dir, &request.transfer_id).await;

                tracing::info!(
                    transfer_id = %request.transfer_id,
                    completed_chunks = recovered.chunks.len(),
                    "recovered transfer manifest"
                );

                recovered
            }
        };

        /*
         * The sender is the authority on its own transfer, so what it describes
         * replaces what was remembered — including the digests of an earlier
         * attempt, whose bytes are no longer the ones being sent.
         */
        let changed = described.is_some() && known.expected != described;

        if changed {
            known.expected = described.clone();
        }

        manifests.insert(request.transfer_id.clone(), known.clone());

        (known, changed)
    };

    /*
     * Written down as well as remembered, so that the digests outlive this
     * process: a receiver that restarts mid-transfer has to be able to check the
     * file it goes on to assemble, and by then the sender may be gone.
     */
    if expected_changed {
        let transfer_dir = state.transfer_dir(&request.transfer_id);

        if let Err(error) = manifest::store(&transfer_dir, &recovered).await {
            tracing::warn!(
                transfer_id = %request.transfer_id,
                %error,
                "could not record the sender's digests; this transfer cannot be checked \
                 if this process ends"
            );
        }
    }

    /*
     * What the recovered manifest is worth as progress, checked against the
     * bytes on disk rather than believed: a receiver that restarted, or whose
     * files were disturbed while it was away, must not report progress it
     * cannot produce. This is what the first event below carries, so a resumed
     * transfer opens at the progress it already has instead of at zero.
     */
    let (received_bytes, received_chunks) =
        recovered_progress(&recovered, &state.transfer_dir(&request.transfer_id)).await;

    let download = Arc::new(DownloadState {
        transfer_id: request.transfer_id.clone(),

        total_bytes: request.file_size,
        received_bytes: AtomicU64::new(received_bytes),

        received_chunks: AtomicU64::new(received_chunks),

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
    state.events.emit(DomainEvent::TransferProgress {
        progress: progress::make_download(&download),
    });

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

    /*
     * A transfer whose assembly was refused does not take chunks until the
     * sender has been told and has come back for the ones that are missing.
     *
     * The sender counts a chunk as delivered when this handler answers `200`,
     * and it finishes the transfer when its queue drains. Once something has
     * been discarded, answering the *next* chunk — the retry of some other one,
     * which is exactly what the sender does after a failure — would let that
     * queue drain and the sender report a file as delivered that the receiver
     * refused to assemble. Refusing everything until a new session says so is
     * what keeps the two sides telling the same story.
     */
    if state.refused.lock().await.contains(&transfer) {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "{transfer} cannot be assembled as it stands; open a session to \
                 resume it and send the chunks that are missing"
            ),
        ));
    }

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

    let hash = Sha256::digest(transfer_key);

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

    let transfer_dir = state.transfer_dir(&transfer);

    let parts = transfer_dir.join(&file);

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

    let chunk_hash = manifest::sha256_hex(&decrypted);
    let chunk_length = decrypted.len() as u64;

    /*
     * The manifest is this receiver's record of which chunks it actually has.
     * It is read from disk the first time a transfer is touched after a
     * restart, so that knowledge survives the process — and it is the only
     * thing that makes a `.part` trustworthy; see `transfer::manifest`.
     */
    let mut manifests = state.manifests.lock().await;

    if !manifests.contains_key(&transfer) {
        let recovered = manifest::load_or_default(&transfer_dir, &transfer).await;

        tracing::info!(
            transfer_id=%transfer,
            completed_chunks=recovered.chunks.len(),
            "loaded transfer manifest"
        );

        manifests.insert(transfer.clone(), recovered);
    }

    let record = manifests
        .get_mut(&transfer)
        .expect("the manifest was inserted above");

    /*
     * A chunk may only be skipped when the manifest has an entry for it *and*
     * the bytes on disk still match that entry. A `.part` with no entry — left
     * by an older build, or by a crash before the manifest was committed — is
     * not evidence of anything and gets written again.
     */
    let already_complete = match record.entry(&file, index) {
        Some(entry) => {
            entry.chunk_length == chunk_length
                && entry.chunk_hash == chunk_hash
                && manifest::part_matches(&part, entry).await
        }

        None => false,
    };

    if already_complete {
        tracing::debug!(
            transfer_id=%transfer,
            chunk=index,
            "chunk already present and matches the manifest"
        );
    } else {
        /*
         * Whatever is on disk under this name is not this chunk: there was no
         * entry for it, or the entry disagreed with what arrived, or the bytes
         * did not hash to what the entry recorded. Remove it before writing the
         * replacement, so that a failure part-way through leaves nothing behind
         * that a later run could read as progress.
         */
        manifest::discard_part(&part).await.map_err(|e| {
            storage_failure(
                &state,
                &download,
                &transfer,
                index,
                "failed discarding an invalid chunk",
                e,
            )
        })?;

        /*
         * `write_part` writes a temporary and renames it into place once its
         * bytes are flushed and synced, so a crash mid-write cannot leave a
         * short `.part` under the real name — and when it returns, the bytes
         * are durably on disk.
         */
        manifest::write_part(&part, &decrypted).await.map_err(|e| {
            storage_failure(
                &state,
                &download,
                &transfer,
                index,
                "failed writing chunk",
                e,
            )
        })?;

        /*
         * Record the chunk only now, after its bytes are durably in place, so
         * there is no ordering in which a crash leaves a manifest entry for a
         * chunk the receiver cannot produce. `store` is itself atomic, so this
         * step too leaves the previous manifest or this one, never a torn file.
         */
        record.record(manifest::ChunkEntry {
            file_id: file.clone(),
            chunk_index: index,
            chunk_length,
            chunk_hash,
        });

        manifest::store(&transfer_dir, record).await.map_err(|e| {
            tracing::error!(
                transfer_id=%transfer,
                chunk=index,
                error=%e,
                "failed writing transfer manifest"
            );

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("manifest write failed: {e}"),
            )
        })?;

        // update progress only after the chunk is written and recorded
        download
            .received_bytes
            .fetch_add(chunk_length, Ordering::Relaxed);

        download.received_chunks.fetch_add(1, Ordering::Relaxed);

        *download.status.lock().unwrap() = TransferStatus::Downloading;

        state.events.emit(DomainEvent::TransferProgress {
            progress: progress::make_download(&download),
        });

        tracing::info!(
            transfer_id=%transfer,
            chunk=index,
            path=%part.display(),
            "chunk saved"
        );
    }

    /*
     * Completion is a question for the manifest, not for the filesystem, and it
     * is asked in two passes.
     *
     * The first is cheap — an entry for every index, and nothing read from disk
     * at all — and runs after each arriving chunk. Only when it passes is the
     * second run, which hashes every chunk against its record. Doing the full
     * check on every chunk would re-read the whole file each time and make a
     * transfer quadratic in its own size; doing it once, here, costs a single
     * pass over the file at the moment it matters, which is immediately before
     * the bytes are merged into a finished file.
     */
    let mut complete = manifest::all_chunks_recorded(record, &file, total);

    if complete {
        let verification = manifest::verify_and_prune(record, &parts, &file, total).await;

        complete = verification.complete;

        if verification.pruned {
            /*
             * Chunks that did not match their record have been removed from
             * disk and from the manifest. Persist that, or a restart would
             * reload the old entries and skip bytes we know are wrong.
             */
            manifest::store(&transfer_dir, record).await.map_err(|e| {
                tracing::error!(
                    transfer_id=%transfer,
                    error=%e,
                    "failed writing transfer manifest after pruning"
                );

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("manifest write failed: {e}"),
                )
            })?;

            let lost = (0..total)
                .filter(|index| record.entry(&file, *index).is_none())
                .collect::<Vec<_>>();

            state.refused.lock().await.insert(transfer.clone());

            tracing::warn!(
                transfer_id=%transfer,
                file_id=%file,
                ?lost,
                "chunks no longer match the manifest and were discarded; they have to be sent again"
            );

            /*
             * The sender has to hear about this, and this response is the only
             * thing that reaches it.
             *
             * A chunk the receiver was holding went bad and has been dropped.
             * Answering `200` would let the sender count it as delivered — it
             * has no other way to know — and it would finish a transfer whose
             * file can never be assembled, with nothing left to prompt a resend.
             * Refusing the request instead fails the sender's transfer, and its
             * next resume asks what the receiver holds and is told the truth:
             * the discarded chunks are missing, and it sends them again.
             */
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "chunks {lost:?} of {file} no longer match what was recorded for them \
                     and were discarded; they must be sent again"
                ),
            ));
        }
    }

    if complete {
        tracing::info!(
            transfer_id=%transfer,
            total_chunks=total,
            "all chunks received"
        );

        let downloads = state
            .host
            .downloads_root()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

        let destination = downloads.join(relative_path);

        match assemble_and_publish(record, &transfer_dir, &file, total, &destination).await {
            Ok(Assembly::Published) => {
                // Whatever was refused before, the file is delivered now.
                state.refused.lock().await.remove(&transfer);

                *download.status.lock().unwrap() = TransferStatus::Completed;

                state.events.emit(DomainEvent::TransferCompleted {
                    progress: progress::make_download(&download),
                });

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

                /*
                 * The parts are gone, so the manifest that described them is no
                 * longer worth caching. It stays on disk — a transfer that
                 * restarts under the same id will load it, find no backing
                 * `.part` files, and re-write the chunks, which is the safe
                 * answer.
                 */
                manifests.remove(&transfer);
            }

            /*
             * The chunks were all here and none of them could be shown to be
             * wrong on their own, and the file they assemble into is still not
             * the one the sender described. Nothing is marked complete and the
             * file is not delivered; the chunks behind the mismatch have already
             * been dropped and written down, so a resume asks for exactly those.
             */
            Ok(Assembly::Rejected { chunks }) => {
                tracing::error!(
                    transfer_id=%transfer,
                    file_id=%file,
                    ?chunks,
                    "the assembled file failed its integrity check; those chunks must be sent again"
                );

                let (accounted_bytes, accounted_chunks) = record.accounted();

                download
                    .received_bytes
                    .store(accounted_bytes, Ordering::Relaxed);
                download
                    .received_chunks
                    .store(accounted_chunks, Ordering::Relaxed);

                // Nothing else may be acknowledged for this transfer until the
                // sender has been told; see the top of [`receive`].
                state.refused.lock().await.insert(transfer.clone());

                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "integrity check failed for {file}: chunks {chunks:?} do not match \
                         the sender's digests and must be sent again"
                    ),
                ));
            }

            Err(e) => {
                tracing::error!(
                    transfer_id=%transfer,
                    error=%e,
                    "merge failed"
                );

                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("merge failed: {e}"),
                ));
            }
        }
    }

    tracing::info!(
        transfer_id=%transfer,
        chunk=index,
        "chunk processing complete"
    );

    Ok(StatusCode::OK)
}

pub async fn get_public_key(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
) -> Result<Json<PublicKeyResponse>, (StatusCode, String)> {
    if headers.get("Authorization").is_none() {
        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    let public_key = state.host.device_public_key().map_err(|e| {
        tracing::error!(
            error = %e,
            "failed to load device public key"
        );

        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    tracing::info!("device public key requested");

    Ok(Json(PublicKeyResponse { public_key }))
}

/// The bytes and chunks a recovered manifest is worth as progress.
///
/// An entry counts only when the bytes on disk still match it — the same
/// four-part test [`manifest::verified_completion`] applies — so a manifest that
/// outlived the `.part` files it describes, or whose bytes were changed
/// underneath it, contributes nothing and the transfer opens at the progress it
/// can actually produce. That check reads every chunk back, which is the same
/// cost the resume report pays; it happens once per session, not per chunk.
///
/// Split out from the handler so it can be tested without a running app.
pub async fn recovered_progress(record: &manifest::Manifest, transfer_dir: &Path) -> (u64, u64) {
    let completion = manifest::verified_completion(record, transfer_dir).await;

    let mut bytes = 0u64;
    let mut chunks = 0u64;

    for file in &completion {
        for chunk in &file.chunks {
            bytes += chunk.chunk_length;
            chunks += 1;
        }
    }

    (bytes, chunks)
}

/// What assembling one file came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assembly {
    /// The file was assembled, checked against the sender's digests when it
    /// described any, and moved into place.
    Published,

    /// The assembled file is not the one the sender described. The chunks that
    /// are wrong or missing have been dropped from the manifest and from disk,
    /// and that verdict has been written down, so the sender's next resume sends
    /// them again. Nothing was published.
    Rejected { chunks: Vec<u64> },
}

/// Assemble one file, check it against what the sender said it would be, and
/// publish it only if it is.
///
/// Split out from the chunk handler so the decision can be exercised without a
/// running app: it needs only the manifest, the transfer's directory, and where
/// the file is to end up.
///
/// A rejection is a *durable* verdict. The chunks behind it are forgotten and
/// the manifest is written before the caller is told, so that whatever the
/// sender does next — resume, retry this chunk, or ask what is held — it is told
/// the truth about a file that could not be assembled. A rejected file is never
/// renamed into place: a transfer that fails verification must not leave its
/// output looking finished.
pub async fn assemble_and_publish(
    record: &mut manifest::Manifest,
    transfer_dir: &Path,
    file_id: &str,
    total: u64,
    destination: &Path,
) -> TransferResult<Assembly> {
    let parts = transfer_dir.join(file_id);

    let expected = record
        .expected
        .as_ref()
        .and_then(|expected| expected.file(file_id))
        .cloned();

    let digest = expected.as_ref().map(|file| file.sha256.as_str());

    match merger::merge(&parts, destination, total, digest).await {
        Ok(()) => Ok(Assembly::Published),

        Err(TransferError::IntegrityMismatch {
            expected: wanted,
            actual,
            ..
        }) => {
            tracing::error!(
                file_id,
                expected = %wanted,
                actual = %actual,
                "the assembled file is not the one the sender described"
            );

            let chunks =
                manifest::discard_untrusted(record, &parts, file_id, total, expected.as_ref())
                    .await;

            manifest::store(transfer_dir, record)
                .await
                .map_err(TransferError::Io)?;

            Ok(Assembly::Rejected { chunks })
        }

        Err(error) => Err(error),
    }
}

/// Build the resume report for a transfer, or the error that says there is
/// nothing to report on.
///
/// Split out from the handler so it can be tested without a running app: it
/// needs only the transfer's identity, whether a session is live, and the
/// directory its manifest would live in.
///
/// A transfer is known when a session is live **or** a manifest survived a
/// restart. The second case is the reason this exists — after a restart nothing
/// is in memory, and the manifest on disk is the only thing that remembers what
/// was received.
async fn resume_state(
    transfer_id: &str,
    is_live: bool,
    transfer_dir: &Path,
) -> Result<TransferResumeResponse, (StatusCode, String)> {
    let manifest_exists = fs::metadata(manifest::manifest_path(transfer_dir))
        .await
        .is_ok();

    if !is_live && !manifest_exists {
        return Err((
            StatusCode::NOT_FOUND,
            format!("unknown transfer {transfer_id}"),
        ));
    }

    /*
     * An unreadable manifest reports nothing rather than failing the request.
     * The transfer demonstrably existed, and "I hold nothing" is both true and
     * the safe answer: the sender offers every chunk again.
     */
    let record = manifest::load_or_default(transfer_dir, transfer_id).await;

    let files = manifest::verified_completion(&record, transfer_dir)
        .await
        .into_iter()
        .map(|file| FileResumeState {
            file_id: file.file_id,
            chunks: file
                .chunks
                .into_iter()
                .map(|chunk| ChunkResumeState {
                    chunk_index: chunk.chunk_index,
                    chunk_length: chunk.chunk_length,
                    chunk_hash: chunk.chunk_hash,
                })
                .collect(),
        })
        .collect();

    Ok(TransferResumeResponse {
        transfer_id: transfer_id.to_string(),
        files,
    })
}

/// Report which chunks of a transfer the receiver already holds.
///
/// The sender calls this before re-offering a transfer it has partly sent, so
/// that resuming costs the chunks still missing rather than the whole file. The
/// answer is derived from the persisted manifest, checked against the bytes on
/// disk, and contains no chunk data.
pub async fn transfer_status(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
) -> Result<Json<TransferResumeResponse>, (StatusCode, String)> {
    if headers.get("Authorization").is_none() {
        tracing::warn!("transfer status requested without authorization");

        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    let transfer = header(&headers, "Transfer-Id")?;

    let is_live = state.downloads.read().await.contains_key(&transfer);

    let report = resume_state(&transfer, is_live, &state.transfer_dir(&transfer)).await;

    match &report {
        Ok(response) => tracing::info!(
            transfer_id = %transfer,
            files = response.files.len(),
            "reported transfer resume state"
        ),

        Err((status, _)) => tracing::warn!(
            transfer_id = %transfer,
            %status,
            "transfer resume state not available"
        ),
    }

    report.map(Json)
}

/// `POST /transfer/cancel` — stop a transfer and remove everything it holds.
///
/// The receiver's half of a cancellation. The sender has already stopped, and
/// this is it saying so, because the receiver is the party holding the bytes:
/// the `.part` files, the manifest, and the in-memory session, key and manifest
/// entry all go. Removing the directory is what makes the cancellation survive a
/// restart — the receiver's knowledge of a transfer it was in the middle of
/// comes from that directory, so a transfer whose directory is gone is not one
/// it can be talked into resuming.
///
/// A new path rather than a change to the three `/transfer/*` endpoints the
/// protocol freezes (ADR-0010 §1). A receiver that predates it answers `404`,
/// which the sender reads as "could not tell it" and moves on — see
/// [`crate::transfer::http_client::HttpClient::cancel_transfer`].
pub async fn cancel_transfer(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    if headers.get("Authorization").is_none() {
        tracing::warn!("transfer cancellation requested without authorization");

        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    let transfer = header(&headers, "Transfer-Id")?;

    let was_known = {
        let manifests = state.manifests.lock().await;

        manifests.contains_key(&transfer) || state.downloads.read().await.contains_key(&transfer)
    };

    /*
     * The abort comes before the removal. A chunk being written holds its own
     * handle to the download rather than looking it up, so taking the entry out
     * of the map does not stop it — the flag does, at its next check. Removing
     * the directory underneath a writer that is already committed to a write
     * would only half-cancel the transfer; this way the write is refused and the
     * transfer is gone.
     */
    if let Some(download) = state.downloads.write().await.remove(&transfer) {
        download.cancelled.store(true, Ordering::Relaxed);
        *download.status.lock().unwrap() = TransferStatus::Cancelled;

        state.events.emit(DomainEvent::TransferCancelled {
            progress: progress::make_download(&download),
        });
    }

    state.keys.write().await.remove(&transfer);
    state.manifests.lock().await.remove(&transfer);

    match cleanup::remove_transfer(&state.root, &transfer).await {
        Ok(removed) => {
            if !was_known && !removed {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!("no transfer {transfer} to cancel"),
                ));
            }

            tracing::info!(
                transfer_id = %transfer,
                removed_from_disk = removed,
                "cancelled a transfer"
            );

            Ok(StatusCode::OK)
        }

        // Logged and reported, never fatal: a directory that cannot be removed
        // is left to the sweep rather than taking the receiver down.
        Err(error) => {
            tracing::error!(
                transfer_id = %transfer,
                %error,
                "could not remove a cancelled transfer's data"
            );

            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cancel cleanup failed: {error}"),
            ))
        }
    }
}

/// Remove the transfers nobody has come back for, and forget them.
///
/// Takes the receiver's maps rather than the whole [`ReceiverState`] so that
/// what it does to them can be exercised without a running app — see the tests.
///
/// Forgetting is not tidiness. The manifest cache is what the receiver answers a
/// sender's resume query from, so a cached manifest for a directory that has
/// been swept would have it claim chunks it no longer holds, and the sender
/// would send nothing and wait for a file that can never be assembled.
async fn sweep_abandoned(
    root: &Path,
    active: &HashSet<String>,
    ttl: Duration,
    now: SystemTime,
    manifests: &tokio::sync::Mutex<HashMap<String, manifest::Manifest>>,
    keys: &tokio::sync::RwLock<HashMap<String, [u8; 32]>>,
) -> Vec<String> {
    let removed = cleanup::sweep(root, ttl, active, now).await;

    if !removed.is_empty() {
        let mut manifests = manifests.lock().await;
        let mut keys = keys.write().await;

        for id in &removed {
            manifests.remove(id);
            keys.remove(id);
        }
    }

    removed
}

/// Sweep for abandoned transfers, for as long as the receiver runs.
///
/// Runs once at startup and then on [`cleanup::SWEEP_INTERVAL`]. Nothing here is
/// fatal: a sweep that fails logs and waits for the next one, because the
/// alternative is a receiver that stops accepting transfers over a directory it
/// could not delete.
pub async fn run_cleanup(state: ReceiverState, ttl: Duration, interval: Duration) {
    loop {
        let active: HashSet<String> = state.downloads.read().await.keys().cloned().collect();

        let removed = sweep_abandoned(
            &state.root,
            &active,
            ttl,
            SystemTime::now(),
            &state.manifests,
            &state.keys,
        )
        .await;

        if !removed.is_empty() {
            tracing::info!(transfers = ?removed, "removed transfers nobody came back for");
        }

        tokio::time::sleep(interval).await;
    }
}

/// The resume handshake's decisions, checked without a running app.
///
/// `resume_state` is deliberately separate from the handler so that the parts
/// that matter — what counts as a known transfer, and what is reported once it
/// is — can be exercised directly. The handler above it only reads the request
/// headers and the live-session map.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::{
        integrity::ExpectedTransfer,
        manifest::test_support::{expected_file, TempDir},
    };

    /// Write a manifest and matching `.part` files, as a receiver that had
    /// received `chunks` would have left them.
    async fn receive(transfer_dir: &Path, file_id: &str, chunks: &[(u64, &[u8])]) {
        let parts = transfer_dir.join(file_id);

        fs::create_dir_all(&parts).await.expect("mkdir");

        let mut record = manifest::load_or_default(transfer_dir, "t-manifest").await;

        for (index, bytes) in chunks {
            manifest::write_part(&manifest::part_path(&parts, *index), bytes)
                .await
                .expect("writing the part");

            record.record(manifest::ChunkEntry {
                file_id: file_id.to_string(),
                chunk_index: *index,
                chunk_length: bytes.len() as u64,
                chunk_hash: manifest::sha256_hex(bytes),
            });
        }

        manifest::store(transfer_dir, &record)
            .await
            .expect("storing");
    }

    /*
     * ----------------------------------------------------------------------
     * What a recovered manifest is worth as progress
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_recovered_transfer_counts_the_chunks_it_already_has() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(&dir, "f-1", &[(0, b"abcd"), (1, b"efgh")]).await;

        let record = manifest::load_or_default(&dir, "t-manifest").await;

        assert_eq!(
            recovered_progress(&record, &dir).await,
            (8, 2),
            "a session that resumes opens with the bytes it already holds, not at zero"
        );
    }

    #[tokio::test]
    async fn a_manifest_entry_the_bytes_no_longer_back_is_not_progress() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(&dir, "f-1", &[(0, b"abcd"), (1, b"efgh")]).await;

        let record = manifest::load_or_default(&dir, "t-manifest").await;

        // The file the entry describes is gone — a restart into a directory
        // that was cleaned under it, say.
        fs::remove_file(manifest::part_path(&dir.join("f-1"), 0))
            .await
            .expect("removing the part");

        assert_eq!(
            recovered_progress(&record, &dir).await,
            (4, 1),
            "an entry with no bytes behind it contributes nothing"
        );
    }

    #[tokio::test]
    async fn a_manifest_entry_whose_bytes_changed_is_not_progress() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        // Two chunks, so that dropping the bad one still leaves the good one to
        // account for.
        receive(&dir, "f-1", &[(0, b"abcd"), (1, b"efgh")]).await;

        let record = manifest::load_or_default(&dir, "t-manifest").await;

        // Same length, different bytes: only the digest can tell the two apart.
        manifest::write_part(&manifest::part_path(&dir.join("f-1"), 0), b"wxyz")
            .await
            .expect("rewriting the part");

        assert_eq!(
            recovered_progress(&record, &dir).await,
            (4, 1),
            "the chunk whose bytes changed is not progress; the other one still is"
        );
    }

    #[tokio::test]
    async fn a_transfer_with_nothing_recovered_opens_at_zero() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let record = manifest::load_or_default(&dir, "t-manifest").await;

        assert_eq!(recovered_progress(&record, &dir).await, (0, 0));
    }

    /*
     * ----------------------------------------------------------------------
     * The file that is published is the file the sender described
     * ----------------------------------------------------------------------
     */

    /// A three-chunk file written as parts, with a manifest recording what
    /// arrived and the sender's description of what it should have been.
    async fn received(scratch: &TempDir, file_id: &str) -> (manifest::Manifest, PathBuf) {
        let chunks: Vec<(u64, &'static [u8])> = vec![(0, b"abcd"), (1, b"efgh"), (2, b"ij")];

        let transfer_dir = scratch.transfer_dir();
        let parts = transfer_dir.join(file_id);

        fs::create_dir_all(&parts).await.expect("mkdir");

        let mut record = manifest::Manifest::new("t-manifest");

        for (index, bytes) in &chunks {
            manifest::write_part(&manifest::part_path(&parts, *index), bytes)
                .await
                .expect("writing a part");

            record.record(manifest::ChunkEntry {
                file_id: file_id.to_string(),
                chunk_index: *index,
                chunk_length: bytes.len() as u64,
                chunk_hash: manifest::sha256_hex(bytes),
            });
        }

        record.expected = Some(ExpectedTransfer {
            transfer_id: "t-manifest".into(),
            files: vec![expected_file(file_id, &chunks)],
        });

        (record, transfer_dir)
    }

    /// Rewrite one chunk on disk and record it, so the receiver's own per-chunk
    /// check has nothing to object to and only the sender's digests can tell.
    async fn silently_replace(record: &mut manifest::Manifest, parts: &Path, index: u64) {
        let bytes: &[u8] = b"WXYZ";

        manifest::write_part(&manifest::part_path(parts, index), bytes)
            .await
            .expect("rewriting the part");

        record.record(manifest::ChunkEntry {
            file_id: "f-1".into(),
            chunk_index: index,
            chunk_length: bytes.len() as u64,
            chunk_hash: manifest::sha256_hex(bytes),
        });
    }

    #[tokio::test]
    async fn an_assembled_file_that_matches_the_sender_is_published() {
        let scratch = TempDir::new();
        let (mut record, transfer_dir) = received(&scratch, "f-1").await;
        let destination = scratch.path().join("out").join("a.bin");

        let outcome = assemble_and_publish(&mut record, &transfer_dir, "f-1", 3, &destination)
            .await
            .expect("assembling");

        assert_eq!(outcome, Assembly::Published);
        assert_eq!(
            fs::read(&destination).await.expect("reading the result"),
            b"abcdefghij"
        );
        assert_eq!(record.chunks.len(), 3, "nothing had to be dropped");
    }

    #[tokio::test]
    async fn a_chunk_that_changed_after_it_was_recorded_fails_the_final_check() {
        let scratch = TempDir::new();
        let (mut record, transfer_dir) = received(&scratch, "f-1").await;
        let destination = scratch.path().join("out").join("a.bin");

        silently_replace(&mut record, &transfer_dir.join("f-1"), 1).await;

        let outcome = assemble_and_publish(&mut record, &transfer_dir, "f-1", 3, &destination)
            .await
            .expect("assembling");

        assert_eq!(
            outcome,
            Assembly::Rejected { chunks: vec![1] },
            "the sender's digests are the only thing that can name this chunk"
        );

        assert!(
            !destination.exists(),
            "a file that failed its check must not be left where it looks delivered"
        );

        assert_eq!(record.chunks.len(), 2, "the chunk behind it is dropped");

        let persisted = manifest::load(&transfer_dir)
            .await
            .expect("reading")
            .expect("the manifest is there");

        assert_eq!(
            persisted.chunks.len(),
            2,
            "and the verdict is written down, so a resume asks for it again"
        );
    }

    #[tokio::test]
    async fn a_retransmitted_chunk_can_complete_the_file() {
        let scratch = TempDir::new();
        let (mut record, transfer_dir) = received(&scratch, "f-1").await;
        let destination = scratch.path().join("out").join("a.bin");
        let parts = transfer_dir.join("f-1");

        silently_replace(&mut record, &parts, 1).await;

        assert!(
            matches!(
                assemble_and_publish(&mut record, &transfer_dir, "f-1", 3, &destination)
                    .await
                    .expect("assembling"),
                Assembly::Rejected { .. }
            ),
            "the first attempt has to fail"
        );

        // The sender sends that one chunk again, and this time it is the right
        // one: the chunk the discard removed is exactly the chunk re-written.
        manifest::write_part(&manifest::part_path(&parts, 1), b"efgh")
            .await
            .expect("writing the part again");

        record.record(manifest::ChunkEntry {
            file_id: "f-1".into(),
            chunk_index: 1,
            chunk_length: 4,
            chunk_hash: manifest::sha256_hex(b"efgh"),
        });

        let outcome = assemble_and_publish(&mut record, &transfer_dir, "f-1", 3, &destination)
            .await
            .expect("assembling");

        assert_eq!(outcome, Assembly::Published);
        assert_eq!(
            fs::read(&destination).await.expect("reading the result"),
            b"abcdefghij"
        );
    }

    #[tokio::test]
    async fn the_senders_digests_still_apply_after_a_restart() {
        let scratch = TempDir::new();
        let (record, transfer_dir) = received(&scratch, "f-1").await;
        let destination = scratch.path().join("out").join("a.bin");

        // What a restart leaves behind: the manifest on disk, nothing in memory.
        manifest::store(&transfer_dir, &record)
            .await
            .expect("storing");

        let mut reloaded = manifest::load(&transfer_dir)
            .await
            .expect("reading")
            .expect("the manifest is there");

        silently_replace(&mut reloaded, &transfer_dir.join("f-1"), 2).await;

        let outcome = assemble_and_publish(&mut reloaded, &transfer_dir, "f-1", 3, &destination)
            .await
            .expect("assembling");

        assert_eq!(
            outcome,
            Assembly::Rejected { chunks: vec![2] },
            "the digests came back with the manifest, so the check still holds"
        );
        assert!(!destination.exists());
    }

    /*
     * ----------------------------------------------------------------------
     * A cancellation outlives the process that made it
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_cancelled_transfer_is_not_resurrected_by_a_restart() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(&dir, "f-1", &[(0, b"abcd"), (1, b"efgh")]).await;

        // A receiver that had forgotten everything would still know this one
        // from the manifest on disk — which is exactly what has to go.
        assert!(
            resume_state("t-manifest", false, &dir).await.is_ok(),
            "the transfer is known before it is cancelled"
        );

        cleanup::remove_transfer(scratch.path(), "t-manifest")
            .await
            .expect("cancelling");

        assert!(!dir.exists(), "the manifest and the parts go together");

        let (status, _) = resume_state("t-manifest", false, &dir)
            .await
            .expect_err("a cancelled transfer is not one to resume");

        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "nothing in memory and nothing on disk is nothing to resume from"
        );
    }

    #[tokio::test]
    async fn sweeping_forgets_what_it_removes() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(&dir, "f-1", &[(0, b"abcd")]).await;

        let manifests = tokio::sync::Mutex::new(HashMap::from([(
            "t-manifest".to_string(),
            manifest::load_or_default(&dir, "t-manifest").await,
        )]));
        let keys = tokio::sync::RwLock::new(HashMap::from([("t-manifest".to_string(), [7u8; 32])]));

        let ttl = Duration::from_secs(60);

        let removed = sweep_abandoned(
            scratch.path(),
            &HashSet::new(),
            ttl,
            SystemTime::now() + ttl + Duration::from_secs(1),
            &manifests,
            &keys,
        )
        .await;

        assert_eq!(removed, vec!["t-manifest"]);
        assert!(!dir.exists());

        assert!(
            manifests.lock().await.is_empty(),
            "a cached manifest would have the receiver claiming chunks it no longer holds"
        );
        assert!(
            keys.read().await.is_empty(),
            "and a key for a transfer that is gone is worth nothing"
        );
    }

    #[tokio::test]
    async fn sweeping_leaves_a_transfer_that_is_still_running() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(&dir, "f-1", &[(0, b"abcd")]).await;

        let manifests = tokio::sync::Mutex::new(HashMap::from([(
            "t-manifest".to_string(),
            manifest::load_or_default(&dir, "t-manifest").await,
        )]));
        let keys = tokio::sync::RwLock::new(HashMap::from([("t-manifest".to_string(), [7u8; 32])]));

        let ttl = Duration::from_secs(60);
        let active: HashSet<String> = ["t-manifest".to_string()].into_iter().collect();

        let removed = sweep_abandoned(
            scratch.path(),
            &active,
            ttl,
            SystemTime::now() + ttl + Duration::from_secs(1),
            &manifests,
            &keys,
        )
        .await;

        assert!(removed.is_empty());
        assert!(
            dir.exists(),
            "a live transfer is not swept, however old it looks"
        );
        assert_eq!(manifests.lock().await.len(), 1);
        assert_eq!(keys.read().await.len(), 1);
    }

    #[tokio::test]
    async fn a_chunk_whose_bytes_were_lost_is_dropped_rather_than_waited_for() {
        let scratch = TempDir::new();
        let (mut record, transfer_dir) = received(&scratch, "f-1").await;
        let destination = scratch.path().join("out").join("a.bin");
        let parts = transfer_dir.join("f-1");

        fs::remove_file(manifest::part_path(&parts, 1))
            .await
            .expect("removing the part");

        /*
         * The chunk was recorded, so the gate is satisfied and the full check
         * runs. The gate asks whether every chunk arrived, not whether it is
         * still on disk: a chunk that arrived and then went missing is one the
         * sender believes it has delivered, and waiting for it to be sent again
         * would be waiting forever.
         */
        assert!(manifest::all_chunks_recorded(&record, "f-1", 3));

        let verification = manifest::verify_and_prune(&mut record, &parts, "f-1", 3).await;

        assert!(!verification.complete, "the file is not whole");
        assert!(
            verification.pruned,
            "and the entry goes, which is what makes a resume ask for it again"
        );
        assert!(
            !manifest::all_chunks_recorded(&record, "f-1", 3),
            "so the file cannot be called complete"
        );

        assert!(
            assemble_and_publish(&mut record, &transfer_dir, "f-1", 3, &destination)
                .await
                .is_err(),
            "and nothing is published from a file that is missing a chunk"
        );

        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn an_unknown_transfer_is_not_found() {
        let scratch = TempDir::new();

        // No live session and nothing on disk: the receiver has never heard of
        // it. `NOT_FOUND` is what the chunk path already answers for a transfer
        // it holds no state for.
        let result = resume_state("t-nobody", false, &scratch.transfer_dir()).await;

        let (status, message) = result.expect_err("an unknown transfer must not report");

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            message.contains("t-nobody"),
            "the error must name the transfer: {message}"
        );
    }

    #[tokio::test]
    async fn a_live_transfer_with_no_chunks_reports_empty_completion() {
        let scratch = TempDir::new();

        // A session exists but nothing has arrived yet.
        let report = resume_state("t-manifest", true, &scratch.transfer_dir())
            .await
            .expect("a live transfer is known");

        assert_eq!(report.transfer_id, "t-manifest");
        assert!(
            report.files.is_empty(),
            "nothing received must be an empty report, not an error"
        );
    }

    #[tokio::test]
    async fn a_transfer_restarted_with_a_manifest_is_known() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(&dir, "f-1", &[(0, b"chunk zero".as_slice())]).await;

        // Not live — this is the state after the receiver restarted, with only
        // the manifest on disk to go on.
        let report = resume_state("t-manifest", false, &dir)
            .await
            .expect("a manifest on disk makes the transfer known");

        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].file_id, "f-1");
    }

    #[tokio::test]
    async fn completed_chunks_are_reported_with_everything_a_sender_needs() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(
            &dir,
            "f-1",
            &[(0, b"chunk zero".as_slice()), (1, b"chunk one".as_slice())],
        )
        .await;
        receive(&dir, "f-2", &[(0, b"other file".as_slice())]).await;

        let report = resume_state("t-manifest", true, &dir).await.expect("known");

        assert_eq!(report.files.len(), 2);

        let first = report
            .files
            .iter()
            .find(|file| file.file_id == "f-1")
            .expect("f-1 reported");

        assert_eq!(first.chunks.len(), 2);
        assert_eq!(first.chunks[0].chunk_index, 0);
        assert_eq!(first.chunks[0].chunk_length, 10);
        assert_eq!(
            first.chunks[0].chunk_hash,
            manifest::sha256_hex(b"chunk zero")
        );
        assert_eq!(first.chunks[1].chunk_index, 1);
    }

    #[tokio::test]
    async fn stale_chunks_are_not_reported() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(
            &dir,
            "f-1",
            &[(0, b"chunk zero".as_slice()), (1, b"chunk one!".as_slice())],
        )
        .await;

        // The bytes behind chunk 1 change after the manifest recorded them.
        fs::write(manifest::part_path(&dir.join("f-1"), 1), b"CHUNK ONE!")
            .await
            .expect("altering the part");

        let report = resume_state("t-manifest", true, &dir).await.expect("known");

        assert_eq!(
            report.files[0].chunks.len(),
            1,
            "a chunk that no longer verifies must not be offered as held"
        );
        assert_eq!(report.files[0].chunks[0].chunk_index, 0);
    }

    #[tokio::test]
    async fn the_report_carries_no_chunk_data() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(&dir, "f-1", &[(0, b"chunk zero".as_slice())]).await;

        let report = resume_state("t-manifest", true, &dir).await.expect("known");

        // Serialising is the contract: the response must describe chunks
        // without carrying a single byte of them.
        let json = serde_json::to_string(&report).expect("serialise");

        assert!(!json.contains("chunk zero"), "no chunk bytes in the report");

        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");

        assert_eq!(value["transfer_id"], "t-manifest");
        assert_eq!(value["files"][0]["file_id"], "f-1");
        assert_eq!(value["files"][0]["chunks"][0]["chunk_index"], 0);
        assert_eq!(value["files"][0]["chunks"][0]["chunk_length"], 10);
        assert_eq!(
            value["files"][0]["chunks"][0]["chunk_hash"],
            manifest::sha256_hex(b"chunk zero")
        );
    }

    #[tokio::test]
    async fn a_transfer_restart_reports_the_same_chunks() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        receive(
            &dir,
            "f-1",
            &[(0, b"chunk zero".as_slice()), (1, b"chunk one".as_slice())],
        )
        .await;

        // Before the restart: the session is live.
        let before = resume_state("t-manifest", true, &dir).await.expect("live");

        // After: nothing in memory, and the same answer must come off disk.
        let after = resume_state("t-manifest", false, &dir)
            .await
            .expect("known");

        assert_eq!(
            serde_json::to_value(&after).expect("serialise"),
            serde_json::to_value(&before).expect("serialise"),
            "the resume report must be identical across a restart"
        );
    }

    #[tokio::test]
    async fn an_unreadable_manifest_reports_nothing_rather_than_failing() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        fs::create_dir_all(&dir).await.expect("mkdir");
        fs::write(manifest::manifest_path(&dir), b"not json")
            .await
            .expect("writing a corrupt manifest");

        // The transfer existed — there is a manifest — so it is not unknown.
        // The honest answer is that nothing is verified.
        let report = resume_state("t-manifest", false, &dir)
            .await
            .expect("a transfer with a manifest is known");

        assert!(report.files.is_empty());
    }
}
