//! A **v1 protocol fixture server**, built from the frozen schema.
//!
//! This is the pinned v1 receiver the phase brief asks for, in the form the
//! brief allows when a pinned *binary* cannot be produced:
//!
//! > If a pinned v1 build cannot be produced from this environment, build a v1
//! > protocol fixture server from the frozen schema and state this limitation in
//! > the report.
//!
//! # Why not the v1.0.4 binary
//!
//! The `v1.0.4` tag is in this repository, but its receiver is not a runnable
//! receiver. At that tag `src-tauri/src/transfer/writer.rs` takes a
//! `tauri::AppHandle` and uses it for three things on the request path — the
//! `settings.json` store for the download location (`resolve_download_root`),
//! `app.path().download_dir()`, and `KeyringService::get_device_private_key`
//! for the device's long-term X25519 key. There is no way to construct those
//! without a live Tauri application, so the receiver cannot be started headless
//! in CI. The tag also predates the Phase 2 workspace, so it builds as a
//! desktop app rather than as a crate a test can link.
//!
//! # Why this fixture is independent of the product's own crypto
//!
//! [`frozen_derive_transfer_key`] and [`frozen_decrypt_chunk`] do **not** call
//! `transfer::crypto`. They re-implement the v1 KDF and AEAD from the schema,
//! using the same primitive crates and the same parameters, written out here.
//!
//! That is the entire point. If the fixture called the product's functions,
//! both sides would move together and a change to the v1 wire would still
//! round-trip — the test would pass while the installed base broke. Because the
//! parameters are written down twice, changing `transfer::crypto` breaks
//! `compat_v1` and the change cannot land.
//!
//! The frozen facts this file encodes:
//!
//! * KDF: HKDF-SHA256, `salt = None` (32 zero bytes), `info =
//!   b"carsdv-transfer-key-v1"`, 32-byte output.
//! * ECDH: X25519, receiver long-term static secret against the sender's
//!   per-transfer ephemeral public key.
//! * AEAD: AES-256-GCM, 12-byte nonce, 16-byte tag appended, **no associated
//!   data**.
//! * Endpoints: `GET /transfer/public-key`, `POST /transfer/start`,
//!   `POST /transfer/chunk`, with the header set in [`CHUNK_HEADERS`].
//! * Authorization is checked for **presence only**. That is the High finding
//!   in `docs/SECURITY.md:5-18` and it is Phase 9's to fix; a fixture that
//!   hardened it would be testing a receiver that does not exist in the field.

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use x25519_dalek::{PublicKey, StaticSecret};

/// Every header `POST /transfer/chunk` carries in v1. The fixture requires
/// exactly this set, so a change to the sender's header contract fails the
/// round trip rather than being silently tolerated.
pub const CHUNK_HEADERS: [&str; 8] = [
    "Authorization",
    "Transfer-Id",
    "File-Id",
    "Chunk-Index",
    "Total-Chunks",
    "Relative-Path",
    "Chunk-Nonce",
    "Encryption",
];

/// The receiver's long-term device key, fixed so the fixture is reproducible
/// across runs. A real receiver reads this from the OS keychain.
const RECEIVER_SECRET: [u8; 32] = [0x77; 32];

/// The `file_size` in the frozen `StartTransferRequest` is only used by v1 for
/// its free-space check, which the fixture does not perform.
#[derive(Deserialize)]
struct FrozenStartRequest {
    transfer_id: String,
    sender_ephemeral_public_key: String,
    #[allow(dead_code)]
    file_size: u64,
}

#[derive(Serialize)]
struct FrozenPublicKeyResponse {
    public_key: String,
}

/// Removes the fixture's working directory when the test that owns it ends.
pub struct FixtureRoot(PathBuf);

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct FrozenReceiver {
    identity: StaticSecret,
    public_key_b64: String,
    root: PathBuf,
    /// `transfer_id` → transfer key, exactly the per-session map v1 keeps.
    keys: Mutex<HashMap<String, [u8; 32]>>,
}

impl FrozenReceiver {
    /// The directory the fixture writes into. The destination path is relative
    /// to this, which is how the `Relative-Path` header is honoured.
    pub fn root(&self) -> &PathBuf {
        &self.root
    }
}

/// A running fixture.
pub struct Fixture {
    /// A base URL with the form `http://127.0.0.1:<port>`, which is what
    /// `HttpClient`'s `normalize_endpoint` accepts unchanged.
    pub endpoint: String,
    pub receiver: Arc<FrozenReceiver>,
    /// Dropped with the fixture; deletes the working directory.
    _root: FixtureRoot,
}

/// A test-only temporary directory.
///
/// Hand-rolled rather than pulling in `tempfile`, because a new dependency edge
/// needs an ADR (rule 11) and this is eight lines. The name carries the process
/// id and a counter so two tests in one binary cannot collide.
fn unique_root() -> FixtureRoot {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);

    let directory =
        std::env::temp_dir().join(format!("vilsend-compat-{}-{}", std::process::id(), serial));

    let _ = std::fs::remove_dir_all(&directory);

    std::fs::create_dir_all(&directory).expect("could not create the fixture root");

    FixtureRoot(directory)
}

/// Start the fixture on an ephemeral loopback port.
///
/// The listener is bound **before** `axum::serve` is spawned, so a connection
/// that arrives while the server task is still starting is queued by the
/// kernel rather than refused. That is what lets the test proceed with no
/// `sleep` — rule 8 forbids sleeping as synchronisation, and a bound listener
/// removes the need for it.
pub async fn start() -> Fixture {
    let root = unique_root();

    let identity = StaticSecret::from(RECEIVER_SECRET);
    let public_key_b64 = STANDARD.encode(PublicKey::from(&identity).as_bytes());

    let receiver = Arc::new(FrozenReceiver {
        identity,
        public_key_b64,
        root: root.0.clone(),
        keys: Mutex::new(HashMap::new()),
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("could not bind the fixture listener");

    let address = listener.local_addr().expect("no local address");

    let router = Router::new()
        .route("/transfer/public-key", get(public_key))
        .route("/transfer/start", post(start_transfer))
        .route("/transfer/chunk", post(chunk))
        // The real receiver's limit, from `lib.rs`.
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(receiver.clone());

    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Fixture {
        endpoint: format!("http://{address}"),
        receiver,
        _root: root,
    }
}

/// `GET /transfer/public-key` — the frozen v1 response shape.
async fn public_key(
    State(receiver): State<Arc<FrozenReceiver>>,
    headers: HeaderMap,
) -> Result<Json<FrozenPublicKeyResponse>, (StatusCode, String)> {
    if headers.get("Authorization").is_none() {
        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    Ok(Json(FrozenPublicKeyResponse {
        public_key: receiver.public_key_b64.clone(),
    }))
}

/// `POST /transfer/start` — the frozen v1 request shape.
async fn start_transfer(
    State(receiver): State<Arc<FrozenReceiver>>,
    headers: HeaderMap,
    Json(request): Json<FrozenStartRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if headers.get("Authorization").is_none() {
        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
    }

    let sender = frozen_decode_public_key(&request.sender_ephemeral_public_key)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let shared = receiver.identity.diffie_hellman(&sender).to_bytes();

    let key = frozen_derive_transfer_key(&shared)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;

    receiver
        .keys
        .lock()
        .expect("fixture key map poisoned")
        .insert(request.transfer_id.clone(), key);

    Ok(StatusCode::OK)
}

/// `POST /transfer/chunk` — the frozen v1 header contract.
async fn chunk(
    State(receiver): State<Arc<FrozenReceiver>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    let transfer = frozen_header(&headers, "Transfer-Id")?;
    let file = frozen_header(&headers, "File-Id")?;
    let relative = frozen_header(&headers, "Relative-Path")?;

    // Present in v1 and never read. Required anyway, because "required and
    // ignored" is the contract and dropping it from the check would make the
    // fixture more permissive than the receiver it stands for.
    frozen_header(&headers, "Encryption")?;

    let index = frozen_u64(&headers, "Chunk-Index")?;
    let total = frozen_u64(&headers, "Total-Chunks")?;

    if headers.get("Authorization").is_none() {
        return Err((StatusCode::UNAUTHORIZED, "missing authorization".into()));
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

    if relative.starts_with('/') || relative.split('/').any(|segment| segment == "..") {
        return Err((StatusCode::BAD_REQUEST, "unsafe relative path".into()));
    }

    let key = {
        let keys = receiver.keys.lock().expect("fixture key map poisoned");

        *keys
            .get(&transfer)
            .ok_or((StatusCode::UNAUTHORIZED, "unknown transfer session".into()))?
    };

    let nonce = STANDARD
        .decode(frozen_header(&headers, "Chunk-Nonce")?)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid nonce encoding".into()))?;

    if nonce.len() != 12 {
        return Err((StatusCode::BAD_REQUEST, "nonce must be 12 bytes".into()));
    }

    let plaintext = frozen_decrypt_chunk(&key, &nonce, &body).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("chunk decrypt failed: {error}"),
        )
    })?;

    let parts = receiver.root.join("incoming").join(&transfer).join(&file);

    std::fs::create_dir_all(&parts).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("storage error: {error}"),
        )
    })?;

    let part = parts.join(format!("{index}.part"));

    // The v1 "already have it" gate. It is what makes a v1 receiver
    // accidentally resumable at file granularity, and it is why a retried chunk
    // is a no-op rather than a corruption.
    if !part.exists() {
        std::fs::write(&part, plaintext).map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write failed: {error}"),
            )
        })?;
    }

    let complete = (0..total).all(|i| parts.join(format!("{i}.part")).exists());

    if complete {
        let destination = receiver.root.join(&relative);

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        }

        let mut merged = Vec::new();

        for i in 0..total {
            let part = parts.join(format!("{i}.part"));

            let bytes = std::fs::read(&part)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

            merged.extend_from_slice(&bytes);
        }

        std::fs::write(&destination, &merged)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

        std::fs::remove_dir_all(&parts)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }

    Ok(StatusCode::OK)
}

/// The v1 header extractor: present, non-empty, valid UTF-8.
fn frozen_header(headers: &HeaderMap, name: &str) -> Result<String, (StatusCode, String)> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or((StatusCode::BAD_REQUEST, format!("missing {name}")))
}

fn frozen_u64(headers: &HeaderMap, name: &str) -> Result<u64, (StatusCode, String)> {
    frozen_header(headers, name)?
        .parse()
        .map_err(|error| (StatusCode::BAD_REQUEST, format!("invalid {name}: {error}")))
}

/// The v1 KDF, written out from the schema rather than called.
///
/// HKDF-SHA256 with **no salt** and `info = b"carsdv-transfer-key-v1"`. The
/// `info` string is the historical one and is not a typo; see
/// `transfer::crypto`'s module docs.
fn frozen_derive_transfer_key(shared_secret: &[u8; 32]) -> Result<[u8; 32], String> {
    let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(None, shared_secret);

    let mut key = [0u8; 32];

    hkdf.expand(b"carsdv-transfer-key-v1", &mut key)
        .map_err(|error| format!("hkdf failed: {error}"))?;

    Ok(key)
}

/// The v1 AEAD, written out from the schema rather than called.
///
/// AES-256-GCM with **no associated data**. The absence is the replay-binding
/// gap task 4.1 closes for v2, and it is reproduced here so that a v2 chunk
/// presented to this receiver is genuinely rejected.
fn frozen_decrypt_chunk(
    key: &[u8; 32],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};

    if nonce.len() != 12 {
        return Err("nonce must be 12 bytes".into());
    }

    let cipher = aes_gcm::Aes256Gcm::new(key.into());

    cipher
        .decrypt(aes_gcm::Nonce::from_slice(nonce), ciphertext)
        .map_err(|error| error.to_string())
}

/// The v1 public-key decoder, written out from the schema.
fn frozen_decode_public_key(encoded: &str) -> Result<PublicKey, String> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|error| format!("invalid base64 public key: {error}"))?;

    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;

    Ok(PublicKey::from(bytes))
}
