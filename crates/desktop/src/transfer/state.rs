use crate::models::file_info::FileInfo;
use crate::models::transfer::{ConnectionType, TransferStatus, TransferStatusResponse};
use crate::transfer::integrity::ExpectedTransfer;
use crate::transfer::persist::{PersistedChunk, PersistedFile, PersistedTransfer, STATE_VERSION};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Mutex,
    },
    time::Instant,
};

/// Identifies one chunk of one file. The unit the completed set is kept in.
pub type ChunkKey = (String, u64);

/// The two secrets a live transfer needs.
///
/// Behind a lock rather than held as plain fields because a resume can replace
/// both: the receiver derives a fresh chunk key when it re-establishes a
/// session after its own restart, and a transfer restored from disk has neither
/// until they are read back out of the keychain.
#[derive(Clone, Default)]
pub struct TransferCredentials {
    /// The AES-256 key every chunk is encrypted with.
    pub transfer_key: [u8; 32],
    /// The bearer token the receiver authenticates this transfer with.
    pub auth_token: String,
}

pub struct UploadState {
    pub transfer_id: String,
    /// The key this transfer currently encrypts with, and the token it
    /// authenticates with.
    pub credentials: std::sync::Mutex<TransferCredentials>,
    pub endpoint: String,
    pub network_type: ConnectionType,
    /// The files this transfer was planned from, so its jobs can be rebuilt
    /// after a restart rather than re-scanned from paths that may have moved.
    pub files: Vec<FileInfo>,
    /// How those files were split.
    pub chunk_size: usize,
    pub concurrency: usize,
    pub total_bytes: u64,
    pub uploaded_bytes: AtomicU64,
    pub total_chunks: u64,
    pub uploaded_chunks: AtomicU64,
    pub paused: AtomicBool,
    pub cancelled: AtomicBool,
    pub retry_count: AtomicU32,
    pub max_retries: u32,
    pub status: std::sync::Mutex<TransferStatus>,
    pub started_at: Instant,
    /// Which chunks the receiver has acknowledged.
    ///
    /// The counters above say how much has gone; this says *which*, which is
    /// what a resume needs — a count cannot tell you what is still missing.
    pub completed: std::sync::Mutex<HashSet<ChunkKey>>,
    /// Whether scheduler workers are alive for this transfer.
    ///
    /// Distinguishes a transfer that is merely paused — whose workers are parked
    /// and will pick the queue back up when the flag clears — from one that has
    /// no workers at all, which is what a restart leaves behind and what a
    /// resume has to start.
    pub running: AtomicBool,
    /// Serialises what a resume decides to do.
    ///
    /// Two resumes arriving together — a double-click, or a retry racing the
    /// user's own — must not both find that nothing is running and each start a
    /// scheduler: two worker sets on one queue would send every missing chunk
    /// twice. Whoever holds this gate decides; the other waits, and then finds
    /// the workers already running, or the transfer already finished.
    pub resume_lock: tokio::sync::Mutex<()>,
    /// What this transfer's files are supposed to hash to.
    ///
    /// Worked out from the source files, and handed to the receiver with the
    /// session so that it can check the file it assembles against something it
    /// did not write itself. Absent until it is needed, and absent for good on a
    /// transfer nobody has asked to describe — a receiver that is told nothing
    /// checks nothing, which is what an old one expects.
    pub expected: std::sync::Mutex<Option<ExpectedTransfer>>,
}
impl UploadState {
    /// How many bytes one chunk of this transfer holds, or `None` when the plan
    /// has no such chunk.
    ///
    /// This is the arithmetic [`crate::transfer::chunker::jobs`] uses, applied to
    /// the file the chunk belongs to, and it is what makes a chunk *valid*: an
    /// entry naming a file that is not in the transfer, or an index past the end
    /// of its file, has no length — so it can never be counted as progress,
    /// whatever recorded it.
    pub fn chunk_length(&self, file_id: &str, chunk_index: u64) -> Option<u64> {
        let file = self.files.iter().find(|file| file.id == file_id)?;
        let chunk_size = self.chunk_size as u64;

        if chunk_size == 0 {
            return None;
        }

        // `chunker::jobs` gives even an empty file one chunk, so a file has at
        // least one chunk however small it is.
        if chunk_index >= file.size.div_ceil(chunk_size).max(1) {
            return None;
        }

        Some((file.size - chunk_index * chunk_size).min(chunk_size))
    }

    /// The transfer's size, as the files it was planned from add up to.
    ///
    /// Derived rather than stored so that the total progress is measured against
    /// cannot disagree with the file list it is measured over.
    pub fn planned_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// How many chunks the plan has.
    pub fn planned_chunks(&self) -> u64 {
        let chunk_size = self.chunk_size as u64;

        if chunk_size == 0 {
            return 0;
        }

        self.files
            .iter()
            .map(|file| file.size.div_ceil(chunk_size).max(1))
            .sum()
    }

    /// Record a chunk as acknowledged by the receiver.
    ///
    /// Returns whether this chunk was newly recorded. A chunk the plan does not
    /// have is refused rather than recorded — nothing could have been sent for
    /// it, so counting it would inflate progress — and one already recorded is
    /// not counted twice. The byte and chunk counters are maintained here, each
    /// at the plan's own length for its chunk, so they cannot drift from the set
    /// they describe.
    pub fn mark_completed(&self, file_id: &str, chunk_index: u64) -> bool {
        let Some(length) = self.chunk_length(file_id, chunk_index) else {
            tracing::warn!(
                file_id,
                chunk_index,
                "ignoring an acknowledged chunk that is not part of the transfer"
            );

            return false;
        };

        let inserted = self
            .completed
            .lock()
            .expect("completed lock poisoned")
            .insert((file_id.to_string(), chunk_index));

        if inserted {
            self.uploaded_bytes.fetch_add(length, Ordering::Relaxed);
            self.uploaded_chunks.fetch_add(1, Ordering::Relaxed);
        }

        inserted
    }

    /// Replace the acknowledged set wholesale, and recompute progress from it.
    ///
    /// Used by a resume: the receiver is the authority on what it holds, so its
    /// report replaces what the sender believed rather than adding to it. Only
    /// the chunks the plan can account for are kept, so a resumed transfer
    /// starts from validated bytes — the ones it is about to skip — and never
    /// from a claim it cannot match to a real chunk.
    ///
    /// Returns how many bytes and how many chunks were kept.
    pub fn set_completed(&self, chunks: HashSet<ChunkKey>) -> (u64, u64) {
        let mut validated = HashSet::with_capacity(chunks.len());
        let mut bytes = 0u64;
        let mut count = 0u64;

        for (file_id, chunk_index) in chunks {
            let Some(length) = self.chunk_length(&file_id, chunk_index) else {
                tracing::warn!(
                    file_id,
                    chunk_index,
                    "discarding a completed chunk that is not part of the transfer"
                );

                continue;
            };

            if validated.insert((file_id, chunk_index)) {
                bytes += length;
                count += 1;
            }
        }

        *self.completed.lock().expect("completed lock poisoned") = validated;

        self.uploaded_bytes.store(bytes, Ordering::Relaxed);
        self.uploaded_chunks.store(count, Ordering::Relaxed);

        (bytes, count)
    }

    /// Whether every chunk the plan expects has been acknowledged.
    ///
    /// A drained queue is not completion by itself: the queue is built from the
    /// chunks that were still missing, so a plan the files disagree with would
    /// otherwise let a transfer call itself finished with chunks never sent.
    pub fn is_complete(&self) -> bool {
        self.uploaded_chunks.load(Ordering::Relaxed) == self.planned_chunks()
            && self.uploaded_bytes.load(Ordering::Relaxed) == self.planned_bytes()
    }

    /// The key this transfer is currently encrypting with.
    pub fn transfer_key(&self) -> [u8; 32] {
        self.credentials
            .lock()
            .expect("credentials lock poisoned")
            .transfer_key
    }

    /// The token this transfer is currently authenticating with.
    pub fn auth_token(&self) -> String {
        self.credentials
            .lock()
            .expect("credentials lock poisoned")
            .auth_token
            .clone()
    }

    /// Replace both secrets, as a re-established session does.
    pub fn set_credentials(&self, credentials: TransferCredentials) {
        *self.credentials.lock().expect("credentials lock poisoned") = credentials;
    }

    /// What this transfer's files are supposed to hash to, if it has been worked
    /// out yet.
    pub fn expected(&self) -> Option<ExpectedTransfer> {
        self.expected
            .lock()
            .expect("expected lock poisoned")
            .clone()
    }

    /// Record what this transfer's files are supposed to hash to.
    pub fn set_expected(&self, expected: ExpectedTransfer) {
        *self.expected.lock().expect("expected lock poisoned") = Some(expected);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Relaxed);
    }

    /// The acknowledged chunks, as the state file keeps them.
    pub fn completed_chunks(&self) -> Vec<PersistedChunk> {
        let mut chunks: Vec<PersistedChunk> = self
            .completed
            .lock()
            .expect("completed lock poisoned")
            .iter()
            .map(|(file_id, chunk_index)| PersistedChunk {
                file_id: file_id.clone(),
                chunk_index: *chunk_index,
            })
            .collect();

        // Sorted so that two snapshots of the same state are byte-identical,
        // which makes a state file diffable and a test assertion stable.
        chunks.sort_by(|left, right| {
            left.file_id
                .cmp(&right.file_id)
                .then(left.chunk_index.cmp(&right.chunk_index))
        });

        chunks
    }

    /// The durable record of this transfer, without its secrets.
    ///
    /// The transfer key and auth token are deliberately absent — they belong in
    /// the keychain, not in a state file. See `transfer::persist`.
    pub fn to_persisted(&self) -> PersistedTransfer {
        PersistedTransfer {
            version: STATE_VERSION,
            transfer_id: self.transfer_id.clone(),
            endpoint: self.endpoint.clone(),
            network_type: self.network_type.clone(),
            chunk_size: self.chunk_size,
            concurrency: self.concurrency,
            max_retries: self.max_retries,
            files: self
                .files
                .iter()
                .map(|file| PersistedFile {
                    id: file.id.clone(),
                    path: file.path.to_string_lossy().into_owned(),
                    relative_path: file.relative_path.clone(),
                    size: file.size,
                })
                .collect(),
            total_bytes: self.total_bytes,
            total_chunks: self.total_chunks,
            completed_chunks: self.completed_chunks(),
            status: self.status.lock().expect("status lock poisoned").clone(),
        }
    }

    pub fn snapshot(&self) -> TransferStatusResponse {
        TransferStatusResponse {
            transfer_id: self.transfer_id.clone(),
            receiver_endpoint: self.endpoint.clone(),
            network_type: self.network_type.clone(),
            total_bytes: self.total_bytes,
            uploaded_bytes: self.uploaded_bytes.load(Ordering::Relaxed),
            total_chunks: self.total_chunks,
            uploaded_chunks: self.uploaded_chunks.load(Ordering::Relaxed),
            retry_count: self.retry_count.load(Ordering::Relaxed),
            status: self.status.lock().expect("status lock poisoned").clone(),
        }
    }
}

pub struct DownloadState {
    pub transfer_id: String,

    pub total_bytes: u64,
    pub received_bytes: AtomicU64,

    pub received_chunks: AtomicU64,

    pub paused: AtomicBool,
    pub cancelled: AtomicBool,

    pub started_at: Instant,

    pub status: Mutex<TransferStatus>,
}
