//! What a sender remembers about a transfer across a restart.
//!
//! A transfer's plan — which files, how they are split, where the receiver is,
//! which chunks have been acknowledged — used to live only in the
//! `UploadManager`'s hash map, so a restart lost everything and the only way to
//! send the file again was to send all of it. This module is where that
//! survives instead.
//!
//! # Secrets are kept elsewhere
//!
//! The transfer key and the receiver's bearer token are **not** in this file.
//! A state file is a plain JSON document in the application data directory; the
//! transfer key is what decrypts every byte of the transfer, so writing it there
//! would make the encryption of a completed transfer worth exactly as much as
//! the file permissions on a directory listing. The rest of this application
//! already keeps its secrets in the OS keychain — the device private key, the
//! tunnel token, the session tokens — and these go there too, behind
//! [`SecretStore`].
//!
//! So a transfer is *two* records: a clear one ([`PersistedTransfer`]) and a
//! secret one ([`TransferSecrets`]), stored in different places, written and
//! removed together by [`TransferStore`].
//!
//! # Nothing here decides a transfer is finished
//!
//! A persisted status is a record of what the sender believed, not evidence.
//! [`PersistedTransfer::resumable_status`] demotes a stored `Completed` that its
//! own chunk set does not account for, so a truncated or edited state file
//! cannot turn an unfinished transfer into a finished one.

use crate::models::file_info::FileInfo;
use crate::models::transfer::ConnectionType;
use crate::transfer::atomic_file;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::fs;
use vilsend_core::TransferStatus;

/// The on-disk shape this build writes and understands.
///
/// A state file carrying any other version is treated as absent rather than
/// guessed at, so a future format cannot be silently misread by this one.
pub const STATE_VERSION: u32 = 1;

/// A transfer's secrets, kept in the keychain rather than beside its plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferSecrets {
    /// The AES-256 key every chunk of the transfer is encrypted with.
    pub transfer_key: [u8; 32],
    /// The bearer token the receiver authenticates this transfer with.
    pub auth_token: String,
}

/// Where a transfer's secrets are kept between runs.
///
/// A port so that the manager does not have to hold a Tauri handle to persist a
/// transfer — the same reason [`vilsend_core::EventSink`] exists.
pub trait SecretStore: Send + Sync + 'static {
    fn save(&self, transfer_id: &str, secrets: &TransferSecrets) -> Result<(), String>;

    /// Resolves `Ok(None)` when nothing was ever stored for this transfer.
    ///
    /// Nothing in the upload path reads a secret back yet — that is what
    /// resuming an interrupted transfer needs, and resuming is deliberately a
    /// later step. It is part of the port now so that the store is written and
    /// tested as a whole rather than half of it.
    #[allow(dead_code)]
    fn load(&self, transfer_id: &str) -> Result<Option<TransferSecrets>, String>;

    /// Removes a transfer's secrets. Absent is not an error.
    fn delete(&self, transfer_id: &str) -> Result<(), String>;
}

/// A [`SecretStore`] that keeps secrets in memory.
///
/// For tests, and for a shell that has not wired a keychain: it is honest about
/// persisting nothing, which is better than a store that writes secrets
/// somewhere unclear.
///
/// Shipped rather than kept in a test module for the same reason
/// [`vilsend_core::RecordingEventSink`] is: it is a mutex and a map, and every
/// shell that has not wired a keychain needs it. Only the tests construct it
/// today, so the build does not see a caller.
#[allow(dead_code)]
#[derive(Default)]
pub struct InMemorySecretStore {
    secrets: std::sync::Mutex<std::collections::HashMap<String, TransferSecrets>>,
}

#[allow(dead_code)]
impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemorySecretStore {
    fn save(&self, transfer_id: &str, secrets: &TransferSecrets) -> Result<(), String> {
        self.secrets
            .lock()
            .map_err(|_| "secret store lock poisoned".to_string())?
            .insert(transfer_id.to_string(), secrets.clone());

        Ok(())
    }

    fn load(&self, transfer_id: &str) -> Result<Option<TransferSecrets>, String> {
        Ok(self
            .secrets
            .lock()
            .map_err(|_| "secret store lock poisoned".to_string())?
            .get(transfer_id)
            .cloned())
    }

    fn delete(&self, transfer_id: &str) -> Result<(), String> {
        self.secrets
            .lock()
            .map_err(|_| "secret store lock poisoned".to_string())?
            .remove(transfer_id);

        Ok(())
    }
}

/// A [`SecretStore`] backed by the application's keychain.
pub struct KeyringSecretStore {
    app: tauri::AppHandle,
}

impl KeyringSecretStore {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl SecretStore for KeyringSecretStore {
    fn save(&self, transfer_id: &str, secrets: &TransferSecrets) -> Result<(), String> {
        let encoded = serde_json::to_string(secrets)
            .map_err(|error| format!("could not encode transfer secrets: {error}"))?;

        crate::services::keyring_service::KeyringService::save_transfer_secret(
            &self.app,
            transfer_id,
            &encoded,
        )
    }

    fn load(&self, transfer_id: &str) -> Result<Option<TransferSecrets>, String> {
        let Some(encoded) = crate::services::keyring_service::KeyringService::get_transfer_secret(
            &self.app,
            transfer_id,
        )?
        else {
            return Ok(None);
        };

        // A secret that cannot be decoded is reported as absent, not as an
        // error: the transfer is then treated as one that must be sent again
        // rather than one that fails to load at all.
        Ok(serde_json::from_str(&encoded).ok())
    }

    fn delete(&self, transfer_id: &str) -> Result<(), String> {
        crate::services::keyring_service::KeyringService::delete_transfer_secret(
            &self.app,
            transfer_id,
        )
    }
}

/// One source file of a transfer, as planned when it was started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedFile {
    /// The file's identity, as the scanner derives it.
    pub id: String,
    /// Where it is on this machine.
    pub path: String,
    /// Its path relative to the transfer root, as the receiver will store it.
    pub relative_path: String,
    /// Its size in bytes when the transfer was planned.
    pub size: u64,
}

/// One chunk the receiver has acknowledged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedChunk {
    pub file_id: String,
    pub chunk_index: u64,
}

/// Everything about a transfer that is safe to keep in the clear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedTransfer {
    /// Format version. See [`STATE_VERSION`].
    pub version: u32,
    pub transfer_id: String,
    /// Where the receiver is, and how to reach it.
    pub endpoint: String,
    pub network_type: ConnectionType,
    /// How the files were split, so the same jobs can be rebuilt.
    pub chunk_size: usize,
    pub concurrency: usize,
    pub max_retries: u32,
    /// The shape of the transfer, for rebuilding its job list.
    pub files: Vec<PersistedFile>,
    pub total_bytes: u64,
    pub total_chunks: u64,
    /// The chunks the receiver has acknowledged, in no particular order.
    pub completed_chunks: Vec<PersistedChunk>,
    /// What the sender last believed. Not evidence — see
    /// [`Self::resumable_status`].
    pub status: TransferStatus,
}

impl PersistedTransfer {
    /// The status to restore this transfer with.
    ///
    /// Two corrections are applied, and both exist so that a transfer is never
    /// called finished on the strength of local state alone:
    ///
    /// * A `Completed` record whose chunk set does not account for every chunk
    ///   is demoted. Completion is the receiver's word; a state file claiming it
    ///   without the acknowledged chunks to back it up is a state file that was
    ///   truncated, edited, or written by a version that meant something else.
    /// * A transfer that was mid-flight is restored as `Paused`. Nothing is
    ///   running after a restart and nothing here starts it — that is the
    ///   scheduler's job, and it is not this step's.
    pub fn resumable_status(&self) -> TransferStatus {
        match &self.status {
            TransferStatus::Completed => {
                if self.accounts_for_every_chunk() {
                    TransferStatus::Completed
                } else {
                    tracing::warn!(
                        transfer_id = %self.transfer_id,
                        completed_chunks = self.completed_chunks.len(),
                        total_chunks = self.total_chunks,
                        "a state file claims completion its chunk set does not account for; \
                         restoring as paused"
                    );

                    TransferStatus::Paused
                }
            }

            TransferStatus::Queued | TransferStatus::Uploading | TransferStatus::Paused => {
                TransferStatus::Paused
            }

            // Failed and cancelled are terminal and were reached deliberately.
            other => other.clone(),
        }
    }

    /// Whether the acknowledged chunk set covers every chunk of the transfer.
    ///
    /// The count is compared rather than the identities, because the identities
    /// need each file's chunk count recomputed from its size, and a state file
    /// whose two halves disagree is exactly what this is here to catch.
    pub fn accounts_for_every_chunk(&self) -> bool {
        self.total_chunks > 0 && self.completed_chunks.len() as u64 == self.total_chunks
    }

    /// Rebuild the file list the chunker needs.
    pub fn file_infos(&self) -> Vec<FileInfo> {
        self.files
            .iter()
            .map(|file| FileInfo {
                id: file.id.clone(),
                path: PathBuf::from(&file.path),
                relative_path: file.relative_path.clone(),
                size: file.size,
            })
            .collect()
    }
}

/// A transfer's clear record, where a transfer is known by an id that is safe to
/// use as a file name.
fn state_path(state_dir: &Path, transfer_id: &str) -> io::Result<PathBuf> {
    if transfer_id.is_empty()
        || transfer_id.contains('/')
        || transfer_id.contains('\\')
        || transfer_id.contains("..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("transfer id cannot name a state file: {transfer_id:?}"),
        ));
    }

    Ok(state_dir.join(format!("{transfer_id}.json")))
}

/// The sender's persisted transfers: clear records on disk, secrets in a
/// [`SecretStore`].
pub struct TransferStore {
    state_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
}

impl TransferStore {
    pub fn new(state_dir: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        Self { state_dir, secrets }
    }

    /// A store that keeps its records in `state_dir` and its secrets nowhere.
    ///
    /// For tests: it persists the plan durable-y and the secrets not at all,
    /// which is enough to exercise everything except a real keychain.
    #[cfg(test)]
    pub fn in_memory(state_dir: PathBuf) -> Self {
        Self::new(state_dir, Arc::new(InMemorySecretStore::new()))
    }

    /// Write a transfer's clear record, replacing any earlier one.
    ///
    /// Does not touch the secrets: they are written once when the transfer
    /// starts, and rewriting them on every checkpoint would mean a keychain
    /// round trip per checkpoint for a value that never changes.
    pub async fn save(&self, record: &PersistedTransfer) -> io::Result<()> {
        fs::create_dir_all(&self.state_dir).await?;

        let bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;

        atomic_file::write_atomic(&state_path(&self.state_dir, &record.transfer_id)?, &bytes).await
    }

    /// Store a transfer's secrets.
    pub fn save_secrets(&self, transfer_id: &str, secrets: &TransferSecrets) -> Result<(), String> {
        self.secrets.save(transfer_id, secrets)
    }

    /// Read a transfer's secrets, if they are still stored.
    ///
    /// See [`SecretStore::load`] for why this has no caller in the upload path
    /// yet.
    #[allow(dead_code)]
    pub fn load_secrets(&self, transfer_id: &str) -> Result<Option<TransferSecrets>, String> {
        self.secrets.load(transfer_id)
    }

    /// Read one transfer's record.
    ///
    /// Resolves `None` for a transfer with no record, and also for one whose
    /// record cannot be read — a truncated or edited file is moved aside
    /// (`.json.corrupt`) and reported as absent. Forgetting a transfer costs a
    /// resend; trusting a broken file costs whatever it says.
    pub async fn load(&self, transfer_id: &str) -> Option<PersistedTransfer> {
        let path = match state_path(&self.state_dir, transfer_id) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(%error, "refusing to read a transfer state file");
                return None;
            }
        };

        let bytes = match fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(
                    transfer_id,
                    path = %path.display(),
                    %error,
                    "could not read a transfer state file"
                );

                return None;
            }
        };

        match decode(&bytes) {
            Ok(record) => Some(record),

            Err(error) => {
                tracing::warn!(
                    transfer_id,
                    path = %path.display(),
                    %error,
                    "ignoring an unreadable transfer state file; the transfer will be sent again"
                );

                quarantine(&path).await;

                None
            }
        }
    }

    /// Read every transfer record in the state directory.
    ///
    /// Records that cannot be read are skipped, not fatal: one broken file must
    /// not stop the rest of a restart.
    pub async fn load_all(&self) -> Vec<PersistedTransfer> {
        let mut entries = match fs::read_dir(&self.state_dir).await {
            Ok(entries) => entries,
            // No directory yet is the ordinary state of a machine that has
            // never sent anything.
            Err(_) => return Vec::new(),
        };

        let mut records = Vec::new();

        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(%error, "could not read a transfer state directory entry");
                    break;
                }
            };

            let path = entry.path();

            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let Some(transfer_id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };

            if let Some(record) = self.load(transfer_id).await {
                records.push(record);
            }
        }

        records
    }

    /// Forget a transfer: its record and its secrets.
    ///
    /// Both are removed even if one of them fails, so that a failure to clear
    /// one half cannot leave the other behind forever.
    pub async fn remove(&self, transfer_id: &str) -> io::Result<()> {
        let secrets = self.secrets.delete(transfer_id);

        let path = state_path(&self.state_dir, transfer_id)?;

        let record = match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };

        if let Err(error) = secrets {
            tracing::warn!(transfer_id, %error, "could not remove a transfer's stored secrets");
        }

        record
    }
}

/// Decode a state file, rejecting anything this build should not interpret.
fn decode(bytes: &[u8]) -> Result<PersistedTransfer, String> {
    let record: PersistedTransfer =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;

    if record.version != STATE_VERSION {
        return Err(format!("unsupported state version {}", record.version));
    }

    Ok(record)
}

/// Move an unreadable state file aside, so it is not trusted and does not warn
/// on every subsequent start.
async fn quarantine(path: &Path) {
    let mut aside = path.as_os_str().to_os_string();
    aside.push(".corrupt");

    let aside = PathBuf::from(aside);

    if let Err(error) = fs::rename(path, &aside).await {
        tracing::warn!(path = %path.display(), %error, "could not set a broken state file aside");
    }
}

/// What a sender remembers, and where.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::manifest::test_support::TempDir;

    fn record(transfer_id: &str) -> PersistedTransfer {
        PersistedTransfer {
            version: STATE_VERSION,
            transfer_id: transfer_id.to_string(),
            endpoint: "https://receiver.example".into(),
            network_type: ConnectionType::Tunnel,
            chunk_size: 1024,
            concurrency: 4,
            max_retries: 3,
            files: vec![PersistedFile {
                id: "f-1".into(),
                path: "/tmp/a.bin".into(),
                relative_path: "a.bin".into(),
                size: 2048,
            }],
            total_bytes: 2048,
            total_chunks: 2,
            completed_chunks: Vec::new(),
            status: TransferStatus::Uploading,
        }
    }

    fn store(scratch: &TempDir) -> TransferStore {
        TransferStore::in_memory(scratch.path().join("state"))
    }

    /*
     * ----------------------------------------------------------------------
     * A record survives the process that wrote it
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_record_survives_a_restart() {
        let scratch = TempDir::new();
        let dir = scratch.path().join("state");

        {
            let store = store(&scratch);
            store.save(&record("t-1")).await.expect("saving");
        }

        // A new store over the same directory: nothing in memory, only dist.
        let reopened = TransferStore::in_memory(dir);

        let loaded = reopened.load("t-1").await.expect("the record is there");

        assert_eq!(loaded.transfer_id, "t-1");
        assert_eq!(loaded.endpoint, "https://receiver.example");
        assert_eq!(loaded.chunk_size, 1024);
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.files[0].relative_path, "a.bin");
        assert_eq!(loaded.files[0].size, 2048);
        assert_eq!(loaded.total_chunks, 2);
    }

    #[tokio::test]
    async fn a_partially_completed_transfer_reloads_with_its_chunks() {
        let scratch = TempDir::new();

        let mut record = record("t-1");
        record.completed_chunks = vec![PersistedChunk {
            file_id: "f-1".into(),
            chunk_index: 0,
        }];
        record.status = TransferStatus::Uploading;

        let store = store(&scratch);
        store.save(&record).await.expect("saving");

        let loaded = store.load("t-1").await.expect("the record is there");

        assert_eq!(loaded.completed_chunks.len(), 1);
        assert_eq!(loaded.completed_chunks[0].chunk_index, 0);
        assert_eq!(
            loaded.resumable_status(),
            TransferStatus::Paused,
            "a transfer interrupted mid-flight comes back paused, not running"
        );
    }

    #[tokio::test]
    async fn every_transfer_is_listed_after_a_restart() {
        let scratch = TempDir::new();
        let store = store(&scratch);

        store.save(&record("t-1")).await.expect("saving t-1");
        store.save(&record("t-2")).await.expect("saving t-2");

        let mut ids: Vec<String> = TransferStore::in_memory(scratch.path().join("state"))
            .load_all()
            .await
            .into_iter()
            .map(|record| record.transfer_id)
            .collect();

        ids.sort();
        assert_eq!(ids, vec!["t-1".to_string(), "t-2".to_string()]);
    }

    #[tokio::test]
    async fn listing_a_directory_that_does_not_exist_is_empty_not_an_error() {
        let scratch = TempDir::new();
        let store = TransferStore::in_memory(scratch.path().join("never-created"));

        assert!(store.load_all().await.is_empty());
    }

    #[tokio::test]
    async fn forgetting_a_transfer_removes_its_record_and_its_secrets() {
        let scratch = TempDir::new();
        let store = store(&scratch);

        store.save(&record("t-1")).await.expect("saving");
        store
            .save_secrets("t-1", &secrets())
            .expect("saving secrets");

        store.remove("t-1").await.expect("removing");

        assert!(store.load("t-1").await.is_none());
        assert!(store.load_secrets("t-1").expect("load").is_none());

        // Removing twice is not a failure: the caller wants it gone, and it is.
        store.remove("t-1").await.expect("removing again");
    }

    /*
     * ----------------------------------------------------------------------
     * Secrets do not go in the state file
     * ----------------------------------------------------------------------
     */

    fn secrets() -> TransferSecrets {
        TransferSecrets {
            transfer_key: [0xAB; 32],
            auth_token: "a-bearer-token".into(),
        }
    }

    #[tokio::test]
    async fn secrets_are_stored_apart_from_the_clear_record() {
        let scratch = TempDir::new();
        let store = store(&scratch);

        store.save(&record("t-1")).await.expect("saving");
        store
            .save_secrets("t-1", &secrets())
            .expect("saving secrets");

        // The clear record round trips.
        assert!(store.load_secrets("t-1").expect("load").is_some());

        // And the file on disk holds none of the secret material. This is the
        // assertion that matters: a transfer key is what decrypts every byte of
        // the transfer, and it must not be readable by anyone who can read a
        // JSON file in the application data directory.
        let on_disk = fs::read_to_string(scratch.path().join("state").join("t-1.json"))
            .await
            .expect("reading the state file");

        assert!(
            !on_disk.contains("a-bearer-token"),
            "the auth token must not be in the clear state file"
        );
        assert!(
            !on_disk.contains("abababab"),
            "the transfer key must not be in the clear state file"
        );
        assert!(
            !on_disk.contains("transfer_key") && !on_disk.contains("auth_token"),
            "and neither field should be named there"
        );
    }

    #[tokio::test]
    async fn a_secret_that_was_never_stored_reads_as_absent() {
        let scratch = TempDir::new();

        assert!(store(&scratch)
            .load_secrets("t-nobody")
            .expect("load")
            .is_none());
    }

    /*
     * ----------------------------------------------------------------------
     * A state file we cannot read is not trusted
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_corrupt_state_file_is_ignored_and_set_aside() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        fs::create_dir_all(&state_dir).await.expect("mkdir");

        let path = state_dir.join("t-1.json");
        fs::write(&path, b"{ not json at all")
            .await
            .expect("writing");

        let store = TransferStore::in_memory(state_dir.clone());

        assert!(
            store.load("t-1").await.is_none(),
            "an unreadable record must read as absent, never be guessed at"
        );
        assert!(!path.exists(), "and it must be moved aside");
        assert!(
            state_dir.join("t-1.json.corrupt").exists(),
            "so that it does not warn on every start"
        );
    }

    #[tokio::test]
    async fn a_truncated_state_file_is_ignored() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        fs::create_dir_all(&state_dir).await.expect("mkdir");

        let mut record = record("t-1");
        record.completed_chunks = vec![PersistedChunk {
            file_id: "f-1".into(),
            chunk_index: 0,
        }];

        let complete = serde_json::to_vec_pretty(&record).expect("serialise");

        fs::write(state_dir.join("t-1.json"), &complete[..complete.len() / 2])
            .await
            .expect("writing a truncated file");

        let store = TransferStore::in_memory(state_dir);

        assert!(store.load("t-1").await.is_none());
    }

    #[tokio::test]
    async fn a_state_file_from_a_future_version_is_ignored() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        fs::create_dir_all(&state_dir).await.expect("mkdir");

        fs::write(
            state_dir.join("t-1.json"),
            br#"{"version":999,"transfer_id":"t-1","endpoint":"e","network_type":"TUNNEL",
                 "chunk_size":1,"concurrency":1,"max_retries":1,"files":[],
                 "total_bytes":0,"total_chunks":0,"completed_chunks":[],"status":"QUEUED"}"#,
        )
        .await
        .expect("writing");

        assert!(TransferStore::in_memory(state_dir)
            .load("t-1")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn one_broken_file_does_not_stop_the_others_loading() {
        let scratch = TempDir::new();
        let state_dir = scratch.path().join("state");
        fs::create_dir_all(&state_dir).await.expect("mkdir");

        let store = TransferStore::in_memory(state_dir.clone());

        store.save(&record("t-good")).await.expect("saving");
        fs::write(state_dir.join("t-bad.json"), b"broken")
            .await
            .expect("writing");

        let loaded = store.load_all().await;

        assert_eq!(loaded.len(), 1, "the readable record must still load");
        assert_eq!(loaded[0].transfer_id, "t-good");
    }

    #[tokio::test]
    async fn a_records_temporary_never_survives_a_write() {
        let scratch = TempDir::new();
        let store = store(&scratch);

        store.save(&record("t-1")).await.expect("saving");
        store.save(&record("t-1")).await.expect("saving again");

        let mut entries = fs::read_dir(scratch.path().join("state"))
            .await
            .expect("listing");
        let mut names = Vec::new();

        while let Some(entry) = entries.next_entry().await.expect("entry") {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }

        assert_eq!(names, vec!["t-1.json".to_string()]);
    }

    #[tokio::test]
    async fn an_id_that_cannot_name_a_file_is_refused() {
        let scratch = TempDir::new();
        let store = store(&scratch);

        for bad in ["", "../escape", "a/b", "a\\b"] {
            assert!(
                store.save(&record(bad)).await.is_err(),
                "a transfer id of {bad:?} must not be turned into a path"
            );
        }
    }

    /*
     * ----------------------------------------------------------------------
     * Local state does not get to call a transfer finished
     * ----------------------------------------------------------------------
     */

    #[test]
    fn a_record_claiming_completion_it_cannot_account_for_is_demoted() {
        let mut record = record("t-1");
        record.status = TransferStatus::Completed;
        record.total_chunks = 4;

        // One acknowledged chunk out of four.
        record.completed_chunks = vec![PersistedChunk {
            file_id: "f-1".into(),
            chunk_index: 0,
        }];

        assert_eq!(
            record.resumable_status(),
            TransferStatus::Paused,
            "a truncated or edited state file must not make a transfer finished"
        );
    }

    #[test]
    fn a_record_whose_chunks_all_arrived_keeps_its_completion() {
        let mut record = record("t-1");
        record.status = TransferStatus::Completed;
        record.total_chunks = 2;
        record.completed_chunks = vec![
            PersistedChunk {
                file_id: "f-1".into(),
                chunk_index: 0,
            },
            PersistedChunk {
                file_id: "f-1".into(),
                chunk_index: 1,
            },
        ];

        assert_eq!(record.resumable_status(), TransferStatus::Completed);
    }

    #[test]
    fn a_transfer_with_no_chunks_at_all_is_not_complete() {
        let mut record = record("t-1");
        record.status = TransferStatus::Completed;
        record.total_chunks = 0;
        record.completed_chunks = Vec::new();

        assert_eq!(record.resumable_status(), TransferStatus::Paused);
    }

    #[test]
    fn an_interrupted_transfer_comes_back_paused() {
        for status in [
            TransferStatus::Queued,
            TransferStatus::Uploading,
            TransferStatus::Paused,
        ] {
            let mut record = record("t-1");
            record.status = status;

            assert_eq!(record.resumable_status(), TransferStatus::Paused);
        }
    }

    #[test]
    fn terminal_states_are_kept() {
        for status in [TransferStatus::Failed, TransferStatus::Cancelled] {
            let mut record = record("t-1");
            record.status = status.clone();

            assert_eq!(record.resumable_status(), status);
        }
    }

    /*
     * ----------------------------------------------------------------------
     * The plan rebuilds into the jobs the chunker needs
     * ----------------------------------------------------------------------
     */

    #[test]
    fn the_file_list_rebuilds_into_what_the_chunker_takes() {
        let record = record("t-1");

        let infos = record.file_infos();

        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "f-1");
        assert_eq!(infos[0].path, PathBuf::from("/tmp/a.bin"));
        assert_eq!(infos[0].relative_path, "a.bin");
        assert_eq!(infos[0].size, 2048);
    }
}
