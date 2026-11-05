//! The receiver's per-transfer record of which chunks it has actually got.
//!
//! Before this existed the receiver inferred completion from the presence of a
//! `.part` file: `fs::metadata(&part).is_err()` decided whether to write, and
//! `(0..total).all(metadata(...).is_ok())` decided whether the file was done.
//! Existence is not evidence — `fs::write` truncates before it writes, so a
//! crash mid-write leaves a short `.part` that the old check accepted and the
//! merger spliced into the finished file. Nothing recorded the size or the
//! content, so nothing could tell.
//!
//! So the receiver now keeps a manifest beside the parts, under
//! `{transfer_dir}/manifest.json`, and a chunk counts as received only when the
//! manifest says so *and* the file on disk still matches what the manifest
//! recorded. A `.part` with no entry is not complete, however plausible it
//! looks — see [`chunk_complete`].
//!
//! An accepted chunk does not rewrite the manifest. It appends one line to
//! `{transfer_dir}/chunks.journal`, which is replayed over the manifest when the
//! transfer is loaded and folded back into it once, when the transfer ends.
//! Rewriting the whole document per chunk made a transfer quadratic in its own
//! chunk count; an append is O(1) whatever the transfer's size, and the one
//! full write happens off every chunk's path.
//!
//! Everything here is storage, not wire: nothing in this module changes what
//! the sender sends or what the receiver accepts. The manifest only decides
//! what the receiver may skip re-writing.
//!
//! Four properties are load-bearing and are what the tests below pin:
//!
//! * **A chunk is complete only if it can be proved so.** [`chunk_complete`]
//!   asks for a manifest entry, the `.part` file it names, that file's length
//!   and its SHA-256. Presence alone was never enough and is not accepted.
//! * **The write is atomic.** [`write_part`] and [`store`] each write a
//!   temporary file in the target's own directory, flush and `sync_all` it, and
//!   only then rename it into place. A crash at any point leaves either the
//!   previous file or the new one; a reader can never observe a half-written
//!   file, and a half-written `.part` is never visible under its real name.
//! * **A completion entry follows its bytes.** The caller records a chunk only
//!   after [`write_part`] has returned, so there is no ordering in which a crash
//!   produces a manifest entry for a chunk that is not durably on disk.
//! * **A manifest we cannot parse is not trusted.** [`load_or_default`] returns
//!   an empty record for a corrupt, truncated or future-versioned file. That
//!   costs the completed-chunk knowledge and re-writes chunks as they arrive,
//!   which is extra work; trusting a manifest we cannot read would be
//!   corruption.

use crate::models::file_info::modified_nanos;
use crate::transfer::{atomic_file, report::Telemetry};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex, OnceLock, Weak,
    },
    time::Duration,
};
use tokio::{
    fs,
    sync::{oneshot, Mutex, Notify},
};

/// The manifest's file name inside a transfer's directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// The chunk journal's file name inside a transfer's directory.
///
/// One JSON [`ChunkEntry`] per line, appended and synced once its `.part` is
/// durable. It is replayed over the manifest by [`load_or_default`], and folded
/// back into the manifest — and emptied — by [`store`].
pub const CHUNK_JOURNAL_FILE: &str = "chunks.journal";

/// How large the journal may grow before it is folded into the manifest.
///
/// A megabyte is a few thousand entries: replaying one at startup costs one
/// read, and a transfer that reaches it rewrites its manifest once per megabyte
/// of journal instead of once per chunk.

/// How many chunk entries one journal flush may cover.
///
/// Each chunk still writes its `.part` with its own fsync; this only decides
/// how many journal appends share one fsync. 32 turns one append+fsync per
/// chunk into one per 32, which is the cost the receiver report now measures.
pub const JOURNAL_BATCH_ENTRIES: usize = 32;

/// How long a journaled chunk may wait for batch-mates before its own flush.
///
/// The batch is a latency/throughput trade: a chunk is not acknowledged until
/// its entry is fsynced, so this bounds the added wait. A transfer with fewer
/// than [`JOURNAL_BATCH_ENTRIES`] chunks in flight pays at most this once per
/// flush.
pub const JOURNAL_FLUSH_INTERVAL: Duration = Duration::from_millis(10);

/// The on-disk shape this build writes and understands.
///
/// A manifest carrying any other version is treated as absent rather than
/// guessed at, so that a future format cannot be silently misread by this one.
pub const MANIFEST_VERSION: u32 = 1;

/// One chunk the receiver has written and verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkEntry {
    /// Which file of the transfer this chunk belongs to.
    pub file_id: String,
    /// The chunk's index within that file.
    pub chunk_index: u64,
    /// How many bytes the plaintext chunk held, so a truncated `.part` can be
    /// told from a whole one without reading it.
    pub chunk_length: u64,
    /// Hex SHA-256 of the plaintext chunk as written.
    pub chunk_hash: String,
    /// Where these bytes start in the file's assembly file.
    ///
    /// Present for a file received at its offsets, absent for the `.part`
    /// layout. It is also what tells a restarted receiver which layout a file
    /// was written in, without any extra state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    /// When the `.part` file was written, in nanoseconds since the epoch.
    ///
    /// Recorded so that a chunk offered again can be recognised as already held
    /// without reading it back: if the length and this modification time still
    /// match, nothing has touched the file since this receiver wrote it.
    /// Absent on entries written by older builds, which are hashed as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_nanos: Option<u64>,
}

/// Where a chunk's bytes are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// At this offset in the file's assembly file.
    Direct(u64),
    /// In its own `.part` file.
    Parts,
    /// The sender's description does not fit the layout: the chunk is refused
    /// rather than written where it would overwrite a neighbour.
    Invalid,
}

/// One chunk's identity in the in-memory manifest.
///
/// The wire names a file by string, but this receiver only ever mints that
/// string as `format!("{:x}", u64)`, so the key holds the parsed value. Anything
/// that does not parse keeps a 64-bit hash of the string; the entry itself
/// still carries the original, so the JSON is unaffected.
///
/// `(u64, u64)` is the same 16 bytes as `(u64, u32)` once alignment padding is
/// counted, and it cannot truncate a chunk index.
pub type ChunkKey = (u64, u64);

/// The key half for a file id. Parsed once at load/record time, not per lookup.
fn file_key(file_id: &str) -> u64 {
    u64::from_str_radix(file_id, 16).unwrap_or_else(|_| {
        let mut hasher = DefaultHasher::new();
        file_id.hash(&mut hasher);
        hasher.finish()
    })
}

fn chunk_key(file_id: &str, chunk_index: u64) -> ChunkKey {
    (file_key(file_id), chunk_index)
}

/// Keeps `manifest.json`'s `chunks` a JSON array, in a deterministic order, even
/// though the in-memory form is a map.
mod chunks_serde {
    use super::{chunk_key, ChunkEntry, ChunkKey};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::HashMap;

    pub fn serialize<S: Serializer>(
        chunks: &HashMap<ChunkKey, ChunkEntry>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut entries: Vec<&ChunkEntry> = chunks.values().collect();

        entries.sort_by(|left, right| {
            left.file_id
                .cmp(&right.file_id)
                .then(left.chunk_index.cmp(&right.chunk_index))
        });

        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HashMap<ChunkKey, ChunkEntry>, D::Error> {
        let entries = Vec::<ChunkEntry>::deserialize(deserializer)?;

        let mut chunks = HashMap::with_capacity(entries.len());

        for entry in entries {
            chunks.insert(chunk_key(&entry.file_id, entry.chunk_index), entry);
        }

        Ok(chunks)
    }
}

/// A transfer's completed chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version. See [`MANIFEST_VERSION`].
    pub version: u32,
    /// The transfer this manifest belongs to, so a manifest found under the
    /// wrong directory can be noticed.
    pub transfer_id: String,
    /// One entry per completed chunk, keyed by (file, index) so a lookup or a
    /// retry insert does not scan the transfer. `#[serde(with)]` keeps the
    /// on-disk form the same JSON array old builds read.
    #[serde(default, with = "chunks_serde")]
    pub chunks: HashMap<ChunkKey, ChunkEntry>,
    /// What the sender says this transfer's files hash to.
    ///
    /// The one part of the manifest the receiver did not write: it arrives with
    /// the transfer's metadata and is kept beside the chunks so that it survives
    /// a restart along with them. Absent for a sender that describes no digests,
    /// in which case there is nothing to check an assembled file against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<crate::transfer::integrity::ExpectedTransfer>,
    /// The sender's chunk size, learned from the first non-final chunk.
    ///
    /// A transfer is chunked at one size, so any non-final chunk is exactly
    /// `chunk_size` bytes and the last chunk of a file is its tail. Not
    /// persisted: after a restart it is recovered from the offsets recorded in
    /// the entries themselves.
    #[serde(skip)]
    pub chunk_size: Option<u64>,
    /// Which layout each file of this transfer is being received in.
    ///
    /// Kept in memory so two chunks of the same file arriving at once cannot
    /// disagree about where their bytes go. A restarted receiver rebuilds it
    /// from the entries, each of which records its own offset.
    #[serde(skip)]
    layouts: HashMap<u64, bool>,
}

impl Manifest {
    /// The bytes and chunks this manifest accounts for.
    ///
    /// What a receiver should be shown once chunks have been dropped: the count
    /// a frontend was given when the chunks arrived includes the ones that are
    /// no longer held, and progress that counts discarded chunks is progress
    /// towards a file that cannot be assembled.
    pub fn accounted(&self) -> (u64, u64) {
        let bytes = self
            .chunks
            .values()
            .map(|chunk| chunk.chunk_length)
            .sum::<u64>();

        (bytes, self.chunks.len() as u64)
    }

    /// An empty manifest for `transfer_id`.
    pub fn new(transfer_id: impl Into<String>) -> Self {
        Self {
            version: MANIFEST_VERSION,
            transfer_id: transfer_id.into(),
            chunks: HashMap::new(),
            expected: None,
            chunk_size: None,
            layouts: HashMap::new(),
        }
    }

    /// Room for `total` chunks, so the map does not grow one reallocation at a
    /// time through a large transfer. A no-op once the capacity is there.
    pub fn reserve(&mut self, total: u64) {
        let total = usize::try_from(total).unwrap_or(usize::MAX);

        if total > self.chunks.len() && total > self.chunks.capacity() {
            self.chunks.reserve(total - self.chunks.len());
        }
    }

    /// How many chunks the transfer is holding.
    pub fn chunk_count(&self) -> u32 {
        u32::try_from(self.chunks.len()).unwrap_or(u32::MAX)
    }

    /// A rough resident size for the map. Deliberately approximate: the map's
    /// bucket array plus the two heap strings each entry owns.
    ///
    /// key + entry: `size_of::<(ChunkKey, ChunkEntry)>()`; file id: ≤16 bytes of
    /// heap; chunk hash: 64 bytes of hex.
    pub fn memory_estimate(&self) -> u64 {
        let slots = std::mem::size_of::<(ChunkKey, ChunkEntry)>() as u64;
        let per_entry_strings = 16u64 + 64u64;

        self.chunks.capacity() as u64 * slots
            + self.chunks.len() as u64 * per_entry_strings
    }

    /// The record for one chunk, if it has one. O(1).
    pub fn entry(&self, file_id: &str, chunk_index: u64) -> Option<&ChunkEntry> {
        self.chunks.get(&chunk_key(file_id, chunk_index))
    }

    /// Record a chunk as completed, replacing any earlier record for the same
    /// chunk.
    ///
    /// `insert` has exactly the old `iter_mut().find(...)` semantics: an entry
    /// for the same (file, index) is overwritten whole, including its length,
    /// hash and mtime.
    pub fn record(&mut self, entry: ChunkEntry) {
        self.chunks
            .insert(chunk_key(&entry.file_id, entry.chunk_index), entry);
    }

    /// Drop any record of one chunk, so it reads as never received.
    ///
    /// Used when a chunk's bytes are found not to match its entry: the claim
    /// has to go, or the next run would skip a chunk known to be wrong.
    pub fn forget(&mut self, file_id: &str, chunk_index: u64) {
        self.chunks.remove(&chunk_key(file_id, chunk_index));
    }

    /// Drop every chunk of one file, after it has been published.
    pub fn remove_file(&mut self, file_id: &str) {
        let file = file_key(file_id);

        self.chunks.retain(|key, _| key.0 != file);
    }

    /// Where a chunk's bytes go, decided once per file.
    ///
    /// The sender chunks every file at one size per transfer and places chunk
    /// `i` at `i * chunk_size` ([`crate::transfer::chunker::jobs`]), so the
    /// receiver can place a chunk without being told its offset:
    ///
    /// * a non-final chunk is exactly one chunk size long, so its offset is
    ///   `index * length`, and it tells the receiver the transfer's chunk size;
    /// * the last chunk of a file is its tail, placed at
    ///   `(total - 1) * chunk_size` once some other chunk has said what that is.
    ///
    /// A file whose *first* chunk is its last one has no chunk size to learn
    /// from yet, and keeps the `.part` layout — it will be concatenated at
    /// completion like every file used to be. That costs one merge for that file
    /// and changes nothing about its bytes.
    ///
    /// The decision is written into `self` so that two chunks of one file
    /// arriving together cannot choose differently.
    pub fn place(
        &mut self,
        file_id: &str,
        chunk_index: u64,
        total: u64,
        chunk_length: u64,
    ) -> Placement {
        let key = file_key(file_id);

        match self.layouts.get(&key) {
            Some(true) => return self.direct_placement(file_id, chunk_index, total, chunk_length),
            Some(false) => return Placement::Parts,
            None => {}
        }

        /*
         * Nothing decided yet in this process. Entries written by a previous
         * process already say which layout the file was received in: an entry
         * with an offset is a range of the assembly file.
         */
        let direct = self
            .chunks
            .values()
            .any(|entry| file_key(&entry.file_id) == key && entry.offset.is_some());

        let recorded = direct
            || self
                .chunks
                .values()
                .any(|entry| file_key(&entry.file_id) == key);

        if recorded {
            self.layouts.insert(key, direct);

            return if direct {
                self.direct_placement(file_id, chunk_index, total, chunk_length)
            } else {
                Placement::Parts
            };
        }

        match self.inferred_offset(file_id, chunk_index, total, chunk_length) {
            Some(offset) => {
                self.learn_chunk_size(chunk_index, total, chunk_length);
                self.layouts.insert(key, true);

                Placement::Direct(offset)
            }

            None => {
                self.layouts.insert(key, false);

                Placement::Parts
            }
        }
    }

    /// Whether this file's chunks are kept in one assembly file.
    ///
    /// True when this process decided so, or when a recorded chunk says where
    /// its bytes start — which is how a restarted receiver knows.
    pub fn file_uses_assembly(&self, file_id: &str) -> bool {
        let key = file_key(file_id);

        self.layouts.get(&key).copied().unwrap_or(false)
            || self
                .chunks
                .values()
                .any(|entry| file_key(&entry.file_id) == key && entry.offset.is_some())
    }

    fn direct_placement(
        &mut self,
        file_id: &str,
        chunk_index: u64,
        total: u64,
        chunk_length: u64,
    ) -> Placement {
        if let Some(offset) = self
            .entry(file_id, chunk_index)
            .and_then(|entry| entry.offset)
        {
            return Placement::Direct(offset);
        }

        if total == 0 || chunk_index >= total {
            return Placement::Invalid;
        }

        if chunk_index < total - 1 {
            match self.known_chunk_size(file_id) {
                Some(size) if size != chunk_length => return Placement::Invalid,
                _ => {}
            }

            self.learn_chunk_size(chunk_index, total, chunk_length);

            return chunk_index
                .checked_mul(chunk_length)
                .map_or(Placement::Invalid, Placement::Direct);
        }

        let Some(size) = self.known_chunk_size(file_id) else {
            return Placement::Invalid;
        };

        // The tail cannot be longer than one chunk, or it would overlap the
        // chunk before it.
        if chunk_length > size {
            return Placement::Invalid;
        }

        (total - 1)
            .checked_mul(size)
            .map_or(Placement::Invalid, Placement::Direct)
    }

    /// The offset a chunk would have, if the sender's own layout says.
    fn inferred_offset(
        &self,
        file_id: &str,
        chunk_index: u64,
        total: u64,
        chunk_length: u64,
    ) -> Option<u64> {
        if total == 0 || chunk_index >= total {
            return None;
        }

        // A single-chunk file starts at zero whatever its length.
        if total == 1 {
            return Some(0);
        }

        if chunk_index < total - 1 {
            return chunk_index.checked_mul(chunk_length);
        }

        (total - 1).checked_mul(self.known_chunk_size(file_id)?)
    }

    /// Record the transfer's chunk size from a non-final chunk.
    fn learn_chunk_size(&mut self, chunk_index: u64, total: u64, chunk_length: u64) {
        if chunk_index < total.saturating_sub(1) && self.chunk_size.is_none() {
            self.chunk_size = Some(chunk_length);
        }
    }

    /// The sender's chunk size, from memory or from a recorded offset.
    fn known_chunk_size(&self, file_id: &str) -> Option<u64> {
        if let Some(size) = self.chunk_size {
            return Some(size);
        }

        let key = file_key(file_id);
        let mut first = None;

        for entry in self
            .chunks
            .values()
            .filter(|entry| file_key(&entry.file_id) == key)
        {
            match entry.offset {
                Some(offset) if entry.chunk_index > 0 => {
                    return offset.checked_div(entry.chunk_index);
                }

                // Chunk zero is non-final whenever the file has more than one.
                Some(_) => first = Some(entry.chunk_length),
                None => {}
            }
        }

        first
    }

    /// Every recorded entry of `file_id`, by index, for a verification pass that
    /// runs without the manifest lock.
    pub fn file_entries(
        &self,
        file_id: &str,
        total: u64,
    ) -> Vec<(u64, ChunkEntry)> {
        let file = file_key(file_id);

        (0..total)
            .filter_map(|index| {
                self.chunks
                    .get(&(file, index))
                    .map(|entry| (index, entry.clone()))
            })
            .collect()
    }
}

/// Where a transfer's manifest lives, given the transfer's own directory.
pub fn manifest_path(transfer_dir: &Path) -> PathBuf {
    transfer_dir.join(MANIFEST_FILE)
}

/// Hex SHA-256 of a plaintext chunk — the digest [`ChunkEntry::chunk_hash`]
/// holds.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Read a transfer's manifest.
///
/// `Ok(None)` means there is no manifest, which is the ordinary state of a
/// transfer that has just started. `Err` means there is one and it is unusable
/// — unparseable, truncated, or written by a format version this build does not
/// know. Callers that cannot act on the distinction want [`load_or_default`].
pub async fn load(transfer_dir: &Path) -> io::Result<Option<Manifest>> {
    let path = manifest_path(transfer_dir);

    let bytes = match fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        )
    })?;

    if manifest.version != MANIFEST_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}: unsupported manifest version {}",
                path.display(),
                manifest.version
            ),
        ));
    }

    Ok(Some(manifest))
}

/// Read a transfer's manifest, or start a fresh one.
///
/// An unreadable manifest is logged and discarded rather than propagated: the
/// transfer keeps its `.part` bytes but forgets which of them it trusts, and
/// re-writes chunks as the sender re-offers them. That is strictly safe — the
/// worst case is work already done — whereas failing the chunk or trusting the
/// broken record is not.
pub async fn load_or_default(transfer_dir: &Path, transfer_id: &str) -> Manifest {
    let (mut manifest, replay) = match load(transfer_dir).await {
        Ok(Some(manifest)) => (manifest, true),
        Ok(None) => (Manifest::new(transfer_id), true),
        Err(error) => {
            tracing::warn!(
                transfer_id,
                path = %manifest_path(transfer_dir).display(),
                %error,
                "ignoring unreadable transfer manifest; completed chunks will be re-written"
            );

            /*
             * The journal is not replayed over a record this build could not
             * read: a manifest that is unparseable may be from a format that
             * means something else, and the journal was written to match it.
             */
            (Manifest::new(transfer_id), false)
        }
    };

    if replay {
        replay_journal(transfer_dir, &mut manifest).await;
    }

    manifest
}

/// Fold the chunks journaled since the last compaction into `manifest`.
///
/// A line that does not parse is a crash mid-append — the bytes are a partial
/// chunk's record and never a whole one, because a line is only durable once
/// written in full. It is skipped rather than failing the load: the chunk it
/// described is simply not counted, and is sent again.
async fn replay_journal(transfer_dir: &Path, manifest: &mut Manifest) {
    let bytes = match fs::read(journal_path(transfer_dir)).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::warn!(
                transfer_id = %manifest.transfer_id,
                %error,
                "could not read a transfer's chunk journal; its last chunks will be sent again"
            );

            return;
        }
    };

    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }

        match serde_json::from_slice::<ChunkEntry>(line) {
            Ok(entry) => manifest.record(entry),

            Err(error) => tracing::warn!(
                transfer_id = %manifest.transfer_id,
                %error,
                "skipping a partial line in the chunk journal"
            ),
        }
    }
}

/// Append one accepted chunk to the journal, and make it durable.
///
/// Returns the journal's length afterwards, so the caller can decide whether it
/// is time to compact. The append is not atomic on its own — a crash can leave
/// half a line — but `replay_journal` skips exactly that, and a chunk that is
/// not journaled is simply not counted.
#[cfg(test)]
pub async fn journal(transfer_dir: &Path, entry: &ChunkEntry) -> io::Result<u64> {
    append_batch(transfer_dir, std::slice::from_ref(entry)).await
}

/// Append several entries and make them durable with **one** `sync_all`.
///
/// The batching counterpart of [`journal`]: the bytes of every entry are
/// written in one buffer and covered by a single fsync. A caller may only
/// acknowledge a chunk after the future returns `Ok`, so every entry in the
/// batch is on disk before any of their senders is told it is.
pub async fn append_batch(transfer_dir: &Path, entries: &[ChunkEntry]) -> io::Result<u64> {
    use tokio::io::AsyncWriteExt;

    let gate = journal_gate(transfer_dir);
    let _serialised = gate.lock().await;

    fs::create_dir_all(transfer_dir).await?;

    let path = journal_path(transfer_dir);

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;

    let mut buffer = Vec::new();

    for entry in entries {
        let mut line = serde_json::to_vec(entry).map_err(io::Error::other)?;

        line.push(b'\n');

        buffer.extend_from_slice(&line);
    }

    file.write_all(&buffer).await?;
    file.sync_all().await?;

    Ok(file.metadata().await?.len())
}

/// One lock per journal file, shared by the batcher and every [`store`].
///
/// `store` removes the journal; an append racing it could write to an unlinked
/// file and then be acknowledged as durable while nothing on disk records it.
/// The batcher and `store` are in different structs and call sites, so the gate
/// is keyed by the journal's path. Weak, so a finished transfer's gate is
/// collected with the rest of its state.
fn journal_gate(transfer_dir: &Path) -> Arc<Mutex<()>> {
    static GATES: OnceLock<StdMutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

    let mut gates = GATES
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .expect("journal gate map poisoned");

    let key = journal_path(transfer_dir);

    if let Some(gate) = gates.get(&key).and_then(Weak::upgrade) {
        return gate;
    }

    let gate = Arc::new(Mutex::new(()));

    gates.insert(key, Arc::downgrade(&gate));

    gate
}

/// One transfer's journal batcher.
///
/// `accept` appends an entry and waits for the flush that covers it; the
/// batcher thread writes everything pending, fsyncs once, and only then wakes
/// the chunks whose entries were in that flush. So:
///
/// * an acknowledged chunk's entry is always on disk;
/// * entries buffered but not yet flushed belong to chunks that have not been
///   acknowledged, and are safely retried after a crash;
/// * one fsync covers up to [`JOURNAL_BATCH_ENTRIES`] chunks.
pub struct JournalWriter {
    dir: PathBuf,
    report: Arc<Telemetry>,
    pending: Mutex<PendingJournal>,
    notify: Notify,
    urgent: AtomicBool,
    stop: AtomicBool,
}

#[derive(Default)]
struct PendingJournal {
    entries: Vec<ChunkEntry>,
    /// Resolved once the flush covering `entries` has been fsynced.
    waiters: Vec<oneshot::Sender<Result<(), String>>>,
}

impl JournalWriter {
    /// Start the batcher for one transfer's directory.
    pub fn start(dir: PathBuf, report: Arc<Telemetry>) -> Arc<Self> {
        let writer = Arc::new(Self {
            dir,
            report,
            pending: Mutex::new(PendingJournal::default()),
            notify: Notify::new(),
            urgent: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });

        tokio::spawn(Arc::clone(&writer).run());

        writer
    }

    /// Journal one entry and wait until it is durable.
    ///
    /// `Ok` means a flush that included this entry has completed its fsync —
    /// not that the entry is buffered.
    pub async fn append(&self, entry: &ChunkEntry) -> io::Result<()> {
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending.lock().await;

            pending.entries.push(entry.clone());
            pending.waiters.push(tx);

            if pending.entries.len() >= JOURNAL_BATCH_ENTRIES {
                self.urgent.store(true, Ordering::Relaxed);
            }
        }

        self.notify.notify_one();

        match rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(io::Error::other(message)),
            Err(_) => Err(io::Error::other(
                "the journal flusher stopped before this chunk was durable",
            )),
        }
    }

    /// Entries buffered and not yet flushed.
    pub async fn pending_len(&self) -> u32 {
        u32::try_from(self.pending.lock().await.entries.len()).unwrap_or(u32::MAX)
    }

    /// Flush what is pending and stop the batcher.
    pub async fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);

        self.notify.notify_one();
    }

    async fn run(self: Arc<Self>) {
        loop {
            let notified = self.notify.notified();

            if self.is_empty().await && !self.stop.load(Ordering::Relaxed) {
                notified.await;
            }

            if self.is_empty().await && self.stop.load(Ordering::Relaxed) {
                return;
            }

            // A full batch flushes at once; anything smaller gets one debounce
            // window so it can collect batch-mates.
            if !self.urgent.swap(false, Ordering::Relaxed) && !self.stop.load(Ordering::Relaxed) {
                tokio::time::sleep(JOURNAL_FLUSH_INTERVAL).await;
            }

            self.flush().await;

            if self.stop.load(Ordering::Relaxed) && self.is_empty().await {
                return;
            }
        }
    }

    async fn is_empty(&self) -> bool {
        self.pending.lock().await.entries.is_empty()
    }

    async fn flush(&self) {
        let (entries, waiters) = {
            let mut pending = self.pending.lock().await;

            if pending.entries.is_empty() {
                return;
            }

            (
                std::mem::take(&mut pending.entries),
                std::mem::take(&mut pending.waiters),
            )
        };

        let result = append_batch(&self.dir, &entries).await;

        match &result {
            Ok(_) => {
                self.report.record_journal_flush(entries.len() as u64);
            }

            Err(error) => tracing::warn!(
                error = %error,
                entries = entries.len(),
                "could not flush the chunk journal; those chunks will not be acknowledged"
            ),
        }

        let outcome = result.map(|_| ()).map_err(|error| error.to_string());

        for waiter in waiters {
            let _ = waiter.send(outcome.clone());
        }
    }
}

/// Where a transfer's chunk journal lives.
pub fn journal_path(transfer_dir: &Path) -> PathBuf {
    transfer_dir.join(CHUNK_JOURNAL_FILE)
}

/// Empty the journal, if there is one.
async fn clear_journal(transfer_dir: &Path) -> io::Result<()> {
    match fs::remove_file(journal_path(transfer_dir)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Write a transfer's manifest atomically.
///
/// The bytes go to a temporary file in the manifest's own directory, are
/// flushed and `sync_all`ed with the file still open, and are only then renamed
/// over the manifest. `rename` within one directory replaces the target
/// atomically (on Windows `std::fs::rename` maps to `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING`), so a reader sees the whole old manifest or the
/// whole new one — never the temporary, and never a partial write.
///
/// The temporary is created next to the target rather than in the system temp
/// directory precisely so that the rename cannot cross a filesystem boundary,
/// which would make it a copy and lose the atomicity this function exists for.
///
/// The journal is emptied **before** the manifest is written. A manifest is
/// written to state the whole truth — including a discarded chunk — so a crash
/// between the two must leave the older, shorter truth and a journal that
/// cannot resurrect what this call dropped. Losing recent journal lines costs a
/// resend; resurrecting a chunk that was pruned would hand the sender a
/// completion that can never assemble.
///
/// Callers must hold the transfer's manifest lock: emptying the journal is what
/// makes this call safe against a concurrent [`journal`] append.
pub async fn store(transfer_dir: &Path, manifest: &Manifest) -> io::Result<()> {
    /*
     * Held against the batcher: clearing a journal while a flush is appending
     * to it would let the append land in an unlinked file and be acknowledged
     * for a chunk nothing on disk records.
     */
    let gate = journal_gate(transfer_dir);
    let _serialised = gate.lock().await;

    fs::create_dir_all(transfer_dir).await?;

    clear_journal(transfer_dir).await?;

    let bytes = serde_json::to_vec(manifest).map_err(io::Error::other)?;

    atomic_file::write_atomic(&manifest_path(transfer_dir), &bytes).await
}

/// Write a chunk's bytes into place, atomically and durably.
///
/// The bytes go to `{index}.part.tmp` in the parts directory, are flushed and
/// `sync_all`ed with the file still open, and are only then renamed over
/// `{index}.part`. Two consequences the caller depends on:
///
/// * A crash part-way through writing leaves a temporary file, never a short
///   `.part` under the real name — so a chunk can never be half-present at the
///   path everything else reads.
/// * When this returns `Ok`, the bytes are on disk and not merely in the OS
///   cache, so a manifest entry written afterwards describes data that will
///   survive a power loss as well as a process crash.
///
/// Callers must record the manifest entry **after** this returns. See the
/// module docs.
pub async fn write_part(part: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_file::write_atomic(part, bytes).await
}

/// Remove a `.part` file that is not wanted.
///
/// Used before re-writing a chunk that failed verification, so that a failure
/// part-way through the replacement leaves nothing that a later run could read
/// as progress. A file that is already absent is not an error: the caller's
/// intent is "this chunk should not exist", and it does not.
pub async fn discard_part(part: &Path) -> io::Result<()> {
    match fs::remove_file(part).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Whether the `.part` file behind a manifest entry still holds what the
/// manifest says it holds.
///
/// Length is compared first because it costs only a metadata read and catches
/// the truncation a crash mid-`fs::write` leaves behind; the hash is only
/// computed when the length is already right, where it is what catches a
/// same-length difference — a `.part` left over from a different transfer, or
/// a silent bit rot.
pub async fn part_matches(part: &Path, entry: &ChunkEntry) -> bool {
    let metadata = match fs::metadata(part).await {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };

    if metadata.len() != entry.chunk_length {
        return false;
    }

    /*
     * A `.part` whose length and modification time are exactly what this
     * receiver recorded when it wrote it has not been touched since, so it
     * still holds the bytes that were recorded and does not have to be read
     * back. An entry with no recorded time — written by an older build — or a
     * file whose stamp has moved is hashed as before.
     */
    if entry.written_nanos.is_some() && modified_nanos(&metadata) == entry.written_nanos {
        return true;
    }

    match fs::read(part).await {
        Ok(bytes) => sha256_off_thread(bytes)
            .await
            .map(|digest| digest == entry.chunk_hash)
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// SHA-256 of `bytes`, on the blocking pool.
///
/// Hashing a chunk is a few milliseconds of pure CPU at the default chunk size.
/// Doing it inline would stall every other task on the runtime for that long.
pub async fn sha256_off_thread(bytes: Vec<u8>) -> io::Result<String> {
    tokio::task::spawn_blocking(move || sha256_hex(&bytes))
        .await
        .map_err(io::Error::other)
}

/// The path of one chunk's `.part` file inside a file's parts directory.
pub fn part_path(parts_dir: &Path, chunk_index: u64) -> PathBuf {
    parts_dir.join(format!("{chunk_index}.part"))
}

/// The single file a described file's chunks are written into, at their offsets.
///
/// Used only when the sender described every chunk (see [`direct_layout`]). Its
/// presence is what tells `finish` and the resume report which layout a file was
/// received in, so no extra state has to be recorded.
pub const ASSEMBLY_FILE: &str = "assembly";

/// Where a file's assembly file lives, given its parts directory.
pub fn assembly_path(parts_dir: &Path) -> PathBuf {
    parts_dir.join(ASSEMBLY_FILE)
}

/// Write one chunk into the assembly file at `offset`, and make it durable.
///
/// Unlike [`write_part`] there is no temporary and no rename: the chunk is not
/// atomic, but it does not need to be. Nothing counts a chunk until its journal
/// entry is written, and that entry is only written after this returns — so a
/// crash mid-write costs a resend, never a chunk claimed and not held. A later
/// write to the same range overwrites whatever a failed attempt left behind.
pub async fn write_chunk_at(path: &Path, offset: u64, bytes: Vec<u8>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        use std::io::{Seek, SeekFrom, Write as _};

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&path)?;

        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&bytes)?;
        file.sync_all()
    })
    .await
    .map_err(io::Error::other)?
}

/// Read `length` bytes of the assembly file starting at `offset`.
async fn read_range(path: &Path, offset: u64, length: u64) -> Option<Vec<u8>> {
    use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

    let mut file = fs::File::open(path).await.ok()?;

    file.seek(std::io::SeekFrom::Start(offset)).await.ok()?;

    let mut bytes = Vec::new();

    file.take(length).read_to_end(&mut bytes).await.ok()?;

    Some(bytes)
}

/// Whether the assembly file still holds what a chunk's entry says, at `offset`.
pub async fn chunk_matches(path: &Path, offset: u64, entry: &ChunkEntry) -> bool {
    match read_range(path, offset, entry.chunk_length).await {
        Some(bytes) if bytes.len() as u64 == entry.chunk_length => sha256_off_thread(bytes)
            .await
            .map(|digest| digest == entry.chunk_hash)
            .unwrap_or(false),

        _ => false,
    }
}

/// The direct-layout counterpart of [`discard_untrusted`].
///
/// Ranges share one file, so nothing is removed from disk: forgetting the entry
/// is what makes the chunk read as missing, and the sender's resend overwrites
/// the range. A stale range whose chunk is never sent again cannot be published,
/// because [`all_chunks_recorded`] stays false.
pub async fn discard_untrusted_direct(
    record: &mut Manifest,
    assembly: &Path,
    file_id: &str,
    total: u64,
    expected: Option<&crate::transfer::integrity::ExpectedFile>,
) -> Vec<u64> {
    let mut discarded = Vec::new();

    for index in 0..total {
        let entry = record.entry(file_id, index).cloned();

        // The recorded digest says what the range held when it was written; the
        // sender's, when it gives one, says what it should have held.
        let matches_record = match entry.as_ref().and_then(|entry| entry.offset) {
            Some(offset) => match entry.as_ref() {
                Some(entry) => chunk_matches(assembly, offset, entry).await,
                None => false,
            },

            None => false,
        };

        let matches_sender = match (expected.and_then(|file| file.chunk(index)), entry.as_ref()) {
            (Some(chunk), Some(entry)) => {
                chunk.length == entry.chunk_length && chunk.sha256 == entry.chunk_hash
            }

            (Some(_), None) => false,

            // Nothing to judge this chunk by, but the file is known to be
            // wrong, so everything is asked for again.
            (None, _) if expected.is_some() => false,

            (None, _) => true,
        };

        if matches_record && matches_sender {
            continue;
        }

        record.forget(file_id, index);

        discarded.push(index);
    }

    if !discarded.is_empty() {
        tracing::warn!(
            file_id,
            discarded = discarded.len(),
            total,
            "dropped chunks that do not match the sender's digests"
        );
    }

    discarded
}

/// The length and hex SHA-256 of a `.part` file, or `None` if it is not there.
///
/// [`part_matches`] judges a chunk against what the receiver recorded; this
/// hands back what is actually on disk, so a chunk can be judged against what
/// the *sender* said it would be. They differ exactly when the receiver's record
/// and the sender's are not describing the same bytes.
pub async fn part_digest(part: &Path) -> Option<(u64, String)> {
    let bytes = fs::read(part).await.ok()?;
    let length = bytes.len() as u64;

    let digest = sha256_off_thread(bytes).await.ok()?;

    Some((length, digest))
}

/// Drop every chunk of `file_id` that the sender's digests do not vouch for.
///
/// Returns the indices that have to be sent again, in order.
///
/// A chunk is kept only when the bytes on disk hash to what the sender said they
/// would. Everything else — a chunk whose bytes disagree, one that is not on
/// disk at all, and, when the sender described no per-chunk digests, every chunk
/// of the file — is removed from the manifest and from the parts directory.
///
/// Removing the entry is the point, not a tidy-up. A chunk left in the manifest
/// would be reported to the sender as held, so a resume would send nothing and a
/// file that cannot be assembled would sit there for good. Dropped, it reads as
/// missing and the next resume fetches it.
pub async fn discard_untrusted(
    record: &mut Manifest,
    parts_dir: &Path,
    file_id: &str,
    total: u64,
    expected: Option<&crate::transfer::integrity::ExpectedFile>,
) -> Vec<u64> {
    let mut discarded = Vec::new();

    for index in 0..total {
        let trusted = expected.and_then(|file| file.chunk(index));

        let keep = match trusted {
            Some(chunk) => match part_digest(&part_path(parts_dir, index)).await {
                Some((length, digest)) => length == chunk.length && digest == chunk.sha256,
                None => false,
            },

            // Nothing to judge this chunk by. The whole file is known to be
            // wrong, and the sender is the only party that can say what each
            // chunk should have been, so the file is asked for again.
            None => false,
        };

        if keep {
            continue;
        }

        let _ = discard_part(&part_path(parts_dir, index)).await;

        record.forget(file_id, index);

        discarded.push(index);
    }

    if !discarded.is_empty() {
        tracing::warn!(
            file_id,
            discarded = discarded.len(),
            total,
            "dropped chunks that do not match the sender's digests"
        );
    }

    discarded
}

/// Whether one chunk may be treated as already received.
///
/// **A bare `.part` is not enough, and neither is a plausible-looking one.**
/// All four of these must hold:
///
/// 1. the manifest has an entry for `(file_id, chunk_index)`,
/// 2. the `.part` file that entry names exists,
/// 3. its length equals the entry's `chunk_length`, and
/// 4. its bytes hash to the entry's `chunk_hash`.
///
/// A `.part` with no entry — written by an older build, by a crash before the
/// manifest was committed, or by a different transfer that reused the directory
/// — fails at the first step, however good it looks. A `.part` that was
/// truncated or altered after being recorded fails at the third or fourth.
/// Every one of those is re-written rather than merged.
#[cfg(test)]
pub async fn chunk_complete(
    record: &Manifest,
    parts_dir: &Path,
    file_id: &str,
    chunk_index: u64,
) -> bool {
    let Some(entry) = record.entry(file_id, chunk_index) else {
        return false;
    };

    part_matches(&part_path(parts_dir, chunk_index), entry).await
}

/// Whether every chunk of one file, `0..total`, has been recorded.
///
/// The completion gate, and the half of it that does not touch the disk: `true`
/// means every chunk has arrived and been written durably at least once, so the
/// full check is worth running. It says nothing about whether the bytes behind
/// those entries are still the ones that were recorded — that is
/// [`verify_and_prune`]'s question, and it is asked exactly when this passes.
///
/// **Recorded, not "still there".** A `.part` that was truncated, deleted or
/// altered after it was recorded is a chunk the receiver *had*; asking whether
/// it is still intact would leave the file looking like one still waiting for a
/// chunk that nobody is going to send again, because by then the sender has been
/// told the chunk arrived. Asking instead whether every chunk was recorded
/// leaves those files to the full check, which drops what no longer verifies and
/// hands the sender a reason to send it again.
///
/// A gap — an index with no entry at all — is still a gap: a chunk that never
/// arrived is one the sender still owes, and there is nothing to check.
pub fn all_chunks_recorded(record: &Manifest, file_id: &str, total: u64) -> bool {
    (0..total).all(|index| record.entry(file_id, index).is_some())
}

/// The result of hashing every chunk of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// Every chunk was present and hashed to its recorded digest.
    pub complete: bool,
    /// At least one chunk failed and was removed from disk.
    /// The caller must persist the manifest again.
    pub pruned: bool,
    /// Entries to remove under the manifest lock, in index order. Verified from
    /// a snapshot, so the removal step re-checks the entry before dropping it.
    pub prune: Vec<(u64, ChunkEntry)>,
}

/// Hash every chunk of a file against the manifest, removing any that no longer
/// match, and report whether the file is whole.
///
/// This is the full half of the completion gate, and the only place the
/// receiver decides a file is finished. It reads back every byte it is about to
/// merge, so it runs once — when [`all_chunks_recorded`] has already said the
/// file looks assembled — rather than after each arriving chunk, which would
/// make a transfer quadratic in its own size.
///
/// Removal is the point of the `pruned` flag, not a tidy-up. A chunk whose
/// bytes disagree with its record is not recoverable by keeping it: leaving it
/// on disk and leaving its entry in the manifest would mean the next run
/// skipped a chunk that is known to be wrong. Both are dropped, so the chunk
/// reads as missing and is fetched again.
///
/// A chunk that is merely *absent* is not damaged and is not pruned — there is
/// nothing to discard and nothing to forget. The two cases both leave `complete`
/// false but differ in `pruned`, which is what tells the caller whether the
/// manifest on disk needs rewriting.
#[cfg(test)]
pub async fn verify_and_prune(
    record: &mut Manifest,
    parts_dir: &Path,
    file_id: &str,
    total: u64,
) -> Verification {
    let entries = record.file_entries(file_id, total);
    let verification = verify_entries(parts_dir, file_id, total, &entries).await;

    apply_prune(record, file_id, &verification.prune);

    verification
}

/// The disk half of [`verify_and_prune`], with no manifest access at all.
///
/// Takes a snapshot of the file's entries so the caller can drop the manifest
/// lock before any of the stat/hash work. Parts are discarded here; the manifest
/// entries are only collected, for [`apply_prune`].
pub async fn verify_entries(
    parts_dir: &Path,
    file_id: &str,
    total: u64,
    entries: &[(u64, ChunkEntry)],
) -> Verification {
    let mut complete = true;
    let mut pruned = false;
    let mut prune = Vec::new();

    let by_index: HashMap<u64, &ChunkEntry> =
        entries.iter().map(|(index, entry)| (*index, entry)).collect();

    for index in 0..total {
        let entry = by_index.get(&index);
        let part = part_path(parts_dir, index);

        if let Some(entry) = entry {
            if part_matches(&part, entry).await {
                continue;
            }
        }

        complete = false;

        let has_bytes = fs::metadata(&part).await.is_ok();

        if entry.is_none() && !has_bytes {
            // Simply not here yet. There is no claim to withdraw and no bytes
            // to remove, so this is a gap rather than damage.
            continue;
        }

        /*
         * There is a claim, or bytes, or both, and they do not agree. Drop both
         * halves so nothing downstream can act on either.
         */
        let _ = discard_part(&part).await;

        if let Some(entry) = entry {
            prune.push((index, (*entry).clone()));
        }

        pruned = true;

        tracing::warn!(
            file_id,
            chunk = index,
            had_entry = entry.is_some(),
            had_bytes = has_bytes,
            "chunk failed verification; discarded it and its manifest entry"
        );
    }

    Verification {
        complete,
        pruned,
        prune,
    }
}

/// The direct-layout counterpart of [`verify_entries`].
///
/// The ranges share one file, so a failed chunk is only forgotten by the
/// caller's [`apply_prune`]; its bytes stay until the resend overwrites them.
pub async fn verify_entries_direct(
    assembly: &Path,
    total: u64,
    entries: &[(u64, ChunkEntry)],
) -> Verification {
    let mut complete = true;
    let mut pruned = false;
    let mut prune = Vec::new();

    let by_index: HashMap<u64, &ChunkEntry> = entries
        .iter()
        .map(|(index, entry)| (*index, entry))
        .collect();

    let length = fs::metadata(assembly)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    for index in 0..total {
        let entry = by_index.get(&index);
        let offset = entry.and_then(|entry| entry.offset);

        if let (Some(entry), Some(offset)) = (entry, offset) {
            if chunk_matches(assembly, offset, entry).await {
                continue;
            }
        }

        complete = false;

        /*
         * No claim to withdraw. The ranges share one file, so "there are bytes
         * at this offset" says nothing about this index — a gap, not damage.
         */
        let Some(entry) = entry else {
            continue;
        };

        let had_bytes = offset
            .map(|offset| length >= offset.saturating_add(entry.chunk_length))
            .unwrap_or(false);

        prune.push((index, (*entry).clone()));

        pruned = true;

        tracing::warn!(
            chunk = index,
            had_bytes,
            "chunk failed verification; discarded its manifest entry"
        );
    }

    Verification {
        complete,
        pruned,
        prune,
    }
}

/// Whether the assembly file is at least as long as the sender said the file is.
///
/// The metadata-only half of the completion gate, the counterpart of
/// `all_parts_present`: every recorded chunk wrote its range, so a file that is
/// still exactly the described size has nothing obviously missing, and the merge
/// itself will digest it anyway.
pub async fn assembly_covers(assembly: &Path, expected_size: u64) -> bool {
    fs::metadata(assembly)
        .await
        .map(|metadata| metadata.len() >= expected_size)
        .unwrap_or(false)
}

/// Remove the entries [`verify_entries`] found bad, under the manifest lock.
///
/// Re-checks each entry against the snapshot it was verified from: a chunk that
/// was re-recorded between the verification and this call (a retry that landed
/// with the same index) keeps its fresh entry.
pub fn apply_prune(record: &mut Manifest, file_id: &str, prune: &[(u64, ChunkEntry)]) {
    for (index, verified) in prune {
        let unchanged = record.entry(file_id, *index).map_or(true, |current| {
            current.chunk_hash == verified.chunk_hash
                && current.chunk_length == verified.chunk_length
        });

        if unchanged {
            record.forget(file_id, *index);
        }
    }
}

/// One chunk that is provably on disk, as a resume report describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedChunk {
    /// The chunk's index within its file.
    pub chunk_index: u64,
    /// How many bytes it holds.
    pub chunk_length: u64,
    /// Hex SHA-256 of its plaintext, as written.
    pub chunk_hash: String,
}

/// One file's chunks that are provably on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCompletion {
    /// The file these chunks belong to.
    pub file_id: String,
    /// Its completed chunks, in index order.
    pub chunks: Vec<CompletedChunk>,
}

/// Every chunk of `record` that still verifies against the bytes on disk,
/// grouped by file and ordered by chunk index.
///
/// This is the receiver's answer to "what do you already have?", and it is a
/// claim about *disk*, not about the manifest. Each entry is checked with the
/// same four-part test [`chunk_complete`] applies — entry, file, length, hash —
/// so a manifest that outlived the `.part` files it describes, or one whose
/// bytes were changed underneath it, reports only what is actually there. A
/// stale entry is left out silently: the sender will offer that chunk again and
/// the receiver will write it, which is exactly the safe outcome.
///
/// Files with nothing completed are absent from the result rather than present
/// and empty, so a caller can read "no entry for this file" as "send all of it".
pub async fn verified_completion(record: &Manifest, transfer_dir: &Path) -> Vec<FileCompletion> {
    let mut files: Vec<FileCompletion> = Vec::new();

    for entry in record.chunks.values() {
        /*
         * `chunk_complete` would look the entry up again by (file_id, index);
         * we already hold it, so this applies the same test — file present, at
         * the recorded length, hashing to the recorded digest — directly. That
         * keeps the scan linear in the number of chunks instead of quadratic.
         */
        let parts_dir = transfer_dir.join(&entry.file_id);

        /*
         * An entry with an offset is a range of the file's assembly file;
         * anything else is a `.part`. The entry records where it was written,
         * so no sender description is needed to find it again.
         */
        let still_there = match entry.offset {
            Some(offset) => chunk_matches(&assembly_path(&parts_dir), offset, entry).await,

            None => part_matches(&part_path(&parts_dir, entry.chunk_index), entry).await,
        };

        if !still_there {
            tracing::debug!(
                file_id = %entry.file_id,
                chunk = entry.chunk_index,
                "not reporting a recorded chunk whose bytes did not verify"
            );

            continue;
        }

        let completed = CompletedChunk {
            chunk_index: entry.chunk_index,
            chunk_length: entry.chunk_length,
            chunk_hash: entry.chunk_hash.clone(),
        };

        match files.iter_mut().find(|file| file.file_id == entry.file_id) {
            Some(file) => file.chunks.push(completed),

            None => files.push(FileCompletion {
                file_id: entry.file_id.clone(),
                chunks: vec![completed],
            }),
        }
    }

    for file in &mut files {
        file.chunks.sort_by_key(|chunk| chunk.chunk_index);
    }

    files
}

/// Scratch storage for tests, shared by this module and its siblings.
///
/// `tempfile` is not a dependency of this crate and the tests need one
/// directory that removes itself, so the guard lives here rather than in
/// `Cargo.toml`.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    /// A unique directory that deletes itself when it goes out of scope.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vilsend-manifest-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));

            std::fs::create_dir_all(&path).expect("creating the scratch directory");

            Self(path)
        }

        /// A transfer directory inside the scratch directory, laid out the way
        /// the receiver lays it out.
        pub(crate) fn transfer_dir(&self) -> PathBuf {
            self.0.join("incoming").join("t-manifest")
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The sender's description of a file whose chunks are `chunks`.
    ///
    /// The digests are real ones, computed from the bytes given, so a test can
    /// hand them to the receiver and have it agree with a file that is actually
    /// assembled from those chunks.
    pub(crate) fn expected_file(
        file_id: &str,
        chunks: &[(u64, &[u8])],
    ) -> crate::transfer::integrity::ExpectedFile {
        use super::sha256_hex;
        use crate::transfer::integrity::{ExpectedChunk, ExpectedFile};

        let mut whole = Vec::new();
        let mut described = Vec::new();

        for (index, bytes) in chunks {
            whole.extend_from_slice(bytes);

            described.push(ExpectedChunk {
                index: *index,
                length: bytes.len() as u64,
                sha256: sha256_hex(bytes),
            });
        }

        ExpectedFile {
            file_id: file_id.to_string(),
            relative_path: "a.bin".to_string(),
            size: whole.len() as u64,
            sha256: sha256_hex(&whole),
            chunks: described,
        }
    }
}

/// What the manifest guarantees, checked against real files.
#[cfg(test)]
mod tests {
    use super::test_support::TempDir;
    use super::*;

    fn entry(file_id: &str, index: u64, bytes: &[u8]) -> ChunkEntry {
        ChunkEntry {
            file_id: file_id.to_string(),
            chunk_index: index,
            chunk_length: bytes.len() as u64,
            chunk_hash: sha256_hex(bytes),
            offset: None,
            written_nanos: None,
        }
    }

    /*
     * ----------------------------------------------------------------------
     * A successful chunk leaves a manifest
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn recording_a_chunk_creates_a_manifest_holding_all_four_fields() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let payload = b"the quick brown fox".as_slice();

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 3, payload));

        store(&dir, &manifest).await.expect("storing the manifest");

        assert!(
            manifest_path(&dir).is_file(),
            "the manifest file must exist after a chunk is recorded"
        );

        // Read it back from disk with no in-memory state carried over.
        let reloaded = load(&dir).await.expect("reading it").expect("it is there");

        assert_eq!(
            reloaded.entry("f-1", 3),
            Some(&ChunkEntry {
                file_id: "f-1".into(),
                chunk_index: 3,
                chunk_length: payload.len() as u64,
                chunk_hash: sha256_hex(payload),
                offset: None,
                written_nanos: None,
            })
        );
        assert_eq!(reloaded.version, MANIFEST_VERSION);
        assert_eq!(reloaded.transfer_id, "t-manifest");
    }

    #[tokio::test]
    async fn a_direct_chunk_is_written_at_its_offset_and_verified_there() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let assembly = assembly_path(&dir.join("f-1"));

        write_chunk_at(&assembly, 4, b"efgh".to_vec())
            .await
            .expect("writing at an offset");

        let entry = entry("f-1", 1, b"efgh");

        assert!(chunk_matches(&assembly, 4, &entry).await);
        assert!(
            !chunk_matches(&assembly, 0, &entry).await,
            "the same bytes are not this chunk at another offset"
        );
    }

    #[test]
    fn chunk_offsets_follow_the_senders_layout() {
        let mut manifest = Manifest::new("t-manifest");

        // Three chunks of four, four and two bytes: `index * chunk_size`, with
        // the tail placed from the size the first chunk taught us.
        assert_eq!(manifest.place("f-1", 0, 3, 4), Placement::Direct(0));
        assert_eq!(manifest.place("f-1", 1, 3, 4), Placement::Direct(4));
        assert_eq!(manifest.place("f-1", 2, 3, 2), Placement::Direct(8));
        assert!(manifest.file_uses_assembly("f-1"));

        // A chunk whose length disagrees with the transfer's chunk size would
        // overwrite its neighbour, so it is refused rather than placed.
        assert_eq!(manifest.place("f-1", 1, 3, 5), Placement::Invalid);
    }

    #[test]
    fn a_tail_that_arrives_before_any_chunk_size_is_known_keeps_the_parts_layout() {
        let mut manifest = Manifest::new("t-manifest");

        // The last chunk of a file, with nothing yet to say how big a chunk is.
        assert_eq!(manifest.place("f-1", 2, 3, 2), Placement::Parts);

        // The decision is remembered, so its earlier chunks go to parts too.
        assert_eq!(manifest.place("f-1", 0, 3, 4), Placement::Parts);
        assert!(!manifest.file_uses_assembly("f-1"));

        // A single-chunk file always starts at zero.
        assert_eq!(manifest.place("f-2", 0, 1, 7), Placement::Direct(0));
    }

    #[tokio::test]
    async fn a_manifest_with_no_chunks_yet_is_an_empty_one_not_an_error() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        // Nothing written at all.
        assert_eq!(
            load(&dir).await.expect("no manifest is not a failure"),
            None
        );

        let fresh = load_or_default(&dir, "t-manifest").await;

        assert!(fresh.chunks.is_empty());
        assert_eq!(fresh.transfer_id, "t-manifest");
    }

    /*
     * ----------------------------------------------------------------------
     * The manifest outlives the process that wrote it
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_manifest_survives_a_restart() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        // First "process": receive two chunks, then everything in memory is
        // dropped, exactly as it would be on exit.
        {
            let mut manifest = Manifest::new("t-manifest");
            manifest.record(entry("f-1", 0, b"first chunk"));
            manifest.record(entry("f-1", 1, b"second chunk"));

            store(&dir, &manifest).await.expect("storing");
        }

        // Second "process": the receiver restarts with no state at all and
        // recovers what it had completed from the manifest alone.
        let recovered = load_or_default(&dir, "t-manifest").await;

        assert_eq!(recovered.chunks.len(), 2, "both chunks survive the restart");
        assert!(recovered.entry("f-1", 0).is_some());
        assert!(recovered.entry("f-1", 1).is_some());
        assert_eq!(
            recovered.entry("f-1", 1).expect("present").chunk_hash,
            sha256_hex(b"second chunk")
        );
    }

    #[tokio::test]
    async fn recoding_a_chunk_replaces_its_entry_and_does_not_duplicate_it() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        manifest.record(entry("f-1", 0, b"first attempt"));
        manifest.record(entry("f-1", 0, b"second attempt that is longer"));

        assert_eq!(
            manifest.chunks.len(),
            1,
            "a re-sent chunk must supersede its entry, not add a second one"
        );
        assert_eq!(
            manifest.entry("f-1", 0).expect("present").chunk_hash,
            sha256_hex(b"second attempt that is longer")
        );

        store(&dir, &manifest).await.expect("storing");
        let reloaded = load(&dir).await.expect("reading").expect("present");

        assert_eq!(reloaded.chunks.len(), 1);
    }

    /*
     * ----------------------------------------------------------------------
     * A manifest we cannot parse is discarded, not trusted
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_corrupt_manifest_is_reported_by_load_and_ignored_by_load_or_default() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        tokio::fs::create_dir_all(&dir).await.expect("mkdir");
        tokio::fs::write(manifest_path(&dir), b"{ this is not json")
            .await
            .expect("writing garbage");

        assert!(
            load(&dir).await.is_err(),
            "load must report an unusable manifest rather than guess"
        );

        let recovered = load_or_default(&dir, "t-manifest").await;

        assert!(
            recovered.chunks.is_empty(),
            "a corrupt manifest must leave the transfer with no trusted chunks"
        );
        assert_eq!(recovered.transfer_id, "t-manifest");
    }

    #[tokio::test]
    async fn a_truncated_manifest_is_ignored() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        tokio::fs::create_dir_all(&dir).await.expect("mkdir");

        // A valid manifest cut off mid-file, which is what a crash during a
        // non-atomic write would leave.
        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));
        let complete = serde_json::to_vec_pretty(&manifest).expect("serialise");

        tokio::fs::write(manifest_path(&dir), &complete[..complete.len() / 2])
            .await
            .expect("writing the truncated file");

        let recovered = load_or_default(&dir, "t-manifest").await;

        assert!(recovered.chunks.is_empty());
    }

    #[tokio::test]
    async fn a_manifest_from_a_future_version_is_ignored() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        tokio::fs::create_dir_all(&dir).await.expect("mkdir");

        // Perfectly valid JSON, but a shape this build must not interpret.
        tokio::fs::write(
            manifest_path(&dir),
            br#"{"version":999,"transfer_id":"t-manifest","chunks":[]}"#,
        )
        .await
        .expect("writing");

        assert!(load(&dir).await.is_err());
        assert!(load_or_default(&dir, "t-manifest").await.chunks.is_empty());
    }

    #[tokio::test]
    async fn a_corrupt_manifest_is_healed_by_the_next_completed_chunk() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        tokio::fs::create_dir_all(&dir).await.expect("mkdir");
        tokio::fs::write(manifest_path(&dir), b"not json at all")
            .await
            .expect("writing garbage");

        // What `receive` does: recover (an empty record), record, store.
        let mut manifest = load_or_default(&dir, "t-manifest").await;
        manifest.record(entry("f-1", 0, b"chunk zero"));
        store(&dir, &manifest).await.expect("storing");

        let reloaded = load(&dir).await.expect("reading").expect("present");

        assert_eq!(reloaded.chunks.len(), 1);
        assert!(reloaded.entry("f-1", 0).is_some());
    }

    /*
     * ----------------------------------------------------------------------
     * The five ways a chunk fails to be complete
     *
     * `chunk_complete` is the receiver's definition of "already have it", and
     * each of these removes exactly one of the four things it requires. Every
     * one of them must read as *not complete*, so that the chunk is written
     * again rather than merged.
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_valid_chunk_is_treated_as_already_received() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");
        tokio::fs::write(parts.join("0.part"), b"chunk zero")
            .await
            .expect("writing the part");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));

        assert!(chunk_complete(&manifest, &parts, "f-1", 0).await);
        assert!(all_chunks_recorded(&manifest, "f-1", 1));

        let verification = verify_and_prune(&mut manifest, &parts, "f-1", 1).await;

        assert!(verification.complete);
        assert!(!verification.pruned, "a good chunk must not be removed");
        assert!(
            parts.join("0.part").exists(),
            "and its bytes must still be there"
        );
    }

    #[tokio::test]
    async fn a_part_file_without_a_manifest_entry_is_not_complete() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        // Exactly what an older build, or a crash before the manifest commit,
        // would leave behind: a perfectly good-looking `.part` and no record.
        tokio::fs::write(parts.join("0.part"), b"chunk zero")
            .await
            .expect("writing the part");

        let empty = Manifest::new("t-manifest");

        assert!(
            !chunk_complete(&empty, &parts, "f-1", 0).await,
            "a bare `.part` must not be treated as a received chunk"
        );
        assert!(
            !all_chunks_recorded(&empty, "f-1", 1),
            "and it must not complete the file either"
        );
    }

    #[tokio::test]
    async fn a_missing_part_file_is_not_complete() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));

        // Recorded, then the file went away.
        assert!(!chunk_complete(&manifest, &parts, "f-1", 0).await);
    }

    #[tokio::test]
    async fn a_truncated_part_is_not_complete() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));

        // The manifest says ten bytes. The crash left four.
        tokio::fs::write(parts.join("0.part"), b"chun")
            .await
            .expect("writing the short part");

        assert!(
            !chunk_complete(&manifest, &parts, "f-1", 0).await,
            "a short `.part` contradicts the manifest"
        );
        assert!(
            !part_matches(
                &parts.join("0.part"),
                manifest.entry("f-1", 0).expect("entry")
            )
            .await
        );
    }

    #[tokio::test]
    async fn a_modified_part_is_not_complete() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"AAAAAAAAAA"));

        // Same length, different content. The length check cannot see this;
        // the hash is the whole reason the manifest records one.
        tokio::fs::write(parts.join("0.part"), b"BBBBBBBBBB")
            .await
            .expect("writing the altered part");

        assert!(
            all_chunks_recorded(&manifest, "f-1", 1),
            "the gate reads no bytes at all, so it is blind to this by design —              the full check is what has to catch it"
        );
        assert!(
            !chunk_complete(&manifest, &parts, "f-1", 0).await,
            "the full check must see the altered bytes"
        );
    }

    #[tokio::test]
    async fn a_chunk_recorded_for_another_file_does_not_complete_this_one() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");
        tokio::fs::write(parts.join("0.part"), b"chunk zero")
            .await
            .expect("writing");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-2", 0, b"chunk zero"));

        assert!(
            !chunk_complete(&manifest, &parts, "f-1", 0).await,
            "entries are keyed by file as well as index"
        );
    }

    #[tokio::test]
    async fn one_missing_chunk_keeps_a_file_incomplete() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));
        manifest.record(entry("f-1", 2, b"chunk two"));
        tokio::fs::write(parts.join("0.part"), b"chunk zero")
            .await
            .expect("writing 0");
        tokio::fs::write(parts.join("2.part"), b"chunk two")
            .await
            .expect("writing 2");

        // Chunk 1 never arrived: the file is not complete even though every
        // part that exists has a matching entry.
        assert!(!all_chunks_recorded(&manifest, "f-1", 3));

        let verification = verify_and_prune(&mut manifest, &parts, "f-1", 3).await;

        assert!(!verification.complete, "a gap is not a file");
        assert!(
            !verification.pruned,
            "nothing was wrong, so nothing should have been removed"
        );
        assert!(
            parts.join("0.part").exists() && parts.join("2.part").exists(),
            "the good chunks must survive"
        );
        assert_eq!(manifest.chunks.len(), 2, "and keep their entries");
    }

    /*
     * ----------------------------------------------------------------------
     * A chunk that fails verification is removed, not kept
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_chunk_that_fails_verification_is_deleted_and_forgotten() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));
        manifest.record(entry("f-1", 1, b"chunk one"));

        // Chunk 0 is fine; chunk 1 was altered after being recorded.
        tokio::fs::write(parts.join("0.part"), b"chunk zero")
            .await
            .expect("writing 0");
        tokio::fs::write(parts.join("1.part"), b"CHUNK ONE")
            .await
            .expect("writing 1");

        let verification = verify_and_prune(&mut manifest, &parts, "f-1", 2).await;

        assert!(!verification.complete);
        assert!(verification.pruned);

        assert!(
            !parts.join("1.part").exists(),
            "the bad chunk must be removed, or the next run would skip it forever"
        );
        assert!(
            manifest.entry("f-1", 1).is_none(),
            "and its entry must be forgotten"
        );

        assert!(parts.join("0.part").exists(), "the good chunk is untouched");
        assert!(manifest.entry("f-1", 0).is_some());

        // The pruned state has to survive being persisted.
        store(&dir, &manifest).await.expect("storing");
        let reloaded = load(&dir).await.expect("reading").expect("present");

        assert!(reloaded.entry("f-1", 1).is_none());
        assert!(reloaded.entry("f-1", 0).is_some());
    }

    #[tokio::test]
    async fn pruning_what_survives_a_restart_reads_as_incomplete() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));

        tokio::fs::write(parts.join("0.part"), b"NOT THE CHUNK")
            .await
            .expect("writing");

        verify_and_prune(&mut manifest, &parts, "f-1", 1).await;
        store(&dir, &manifest).await.expect("storing");

        // The next process sees a transfer with nothing completed, so every
        // chunk is fetched again rather than merged from bad bytes.
        let recovered = load_or_default(&dir, "t-manifest").await;

        assert!(recovered.chunks.is_empty());
        assert!(!chunk_complete(&recovered, &parts, "f-1", 0).await);
    }

    /*
     * ----------------------------------------------------------------------
     * A crash cannot leave a completion entry for a chunk that is not there
     *
     * The ordering the receiver relies on is: bytes durably in place, *then*
     * the entry that claims them. These tests pin both halves of that.
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_crash_between_writing_a_part_and_recording_it_leaves_no_entry() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        // The chunk is written...
        write_part(&parts.join("0.part"), b"chunk zero")
            .await
            .expect("writing the part");

        // ...and the process dies here, before `record` and before `store`.
        assert!(
            load(&dir).await.expect("reading").is_none(),
            "no manifest may exist, so nothing claims the chunk is complete"
        );
        assert!(
            !chunk_complete(&Manifest::new("t-manifest"), &parts, "f-1", 0).await,
            "and the part must not read as complete without an entry"
        );
    }

    #[tokio::test]
    async fn a_failed_part_write_creates_no_completion_entry_and_no_temporary() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        // Make the destination impossible to write over, so the rename at the
        // end of `write_part` fails after the bytes were staged.
        tokio::fs::create_dir(parts.join("0.part"))
            .await
            .expect("mkdir over the part path");

        let failed = write_part(&parts.join("0.part"), b"chunk zero").await;

        assert!(failed.is_err(), "writing over a directory must fail");

        assert!(
            !parts.join("0.part.tmp").exists(),
            "a failed write must not leave its temporary behind"
        );

        let manifest = load_or_default(&dir, "t-manifest").await;

        assert!(
            manifest.entry("f-1", 0).is_none(),
            "nothing may be recorded for a chunk that never landed"
        );
    }

    #[tokio::test]
    async fn a_crash_mid_write_is_never_visible_as_a_part() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        // What a crash *during* `write_part` leaves: the temporary, half
        // written, and no `.part` under the real name at all.
        tokio::fs::write(parts.join("0.part.tmp"), b"chun")
            .await
            .expect("writing the abandoned temporary");

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));

        assert!(
            !chunk_complete(&manifest, &parts, "f-1", 0).await,
            "an abandoned temporary is not the chunk"
        );
        assert!(
            tokio::fs::read_dir(&parts)
                .await
                .expect("listing")
                .next_entry()
                .await
                .expect("entry")
                .is_some(),
            "and it is still on disk, unread by anything"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * The write is atomic
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn writing_a_part_leaves_no_temporary_behind() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        write_part(&parts.join("0.part"), b"chunk zero")
            .await
            .expect("writing");

        let mut leftovers = Vec::new();
        let mut entries = tokio::fs::read_dir(&parts).await.expect("listing");

        while let Some(entry) = entries.next_entry().await.expect("entry") {
            leftovers.push(entry.file_name().to_string_lossy().into_owned());
        }

        assert_eq!(
            leftovers,
            vec!["0.part".to_string()],
            "the temporary must be renamed away, leaving only the part"
        );
    }

    #[tokio::test]
    async fn rewriting_a_part_replaces_it_whole() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        // The second write is shorter than the first. If the replacement were
        // an in-place overwrite rather than a rename, the tail of the first
        // would survive and the file would be the wrong length.
        write_part(&parts.join("0.part"), b"the first and longer attempt")
            .await
            .expect("first write");
        write_part(&parts.join("0.part"), b"second")
            .await
            .expect("second write");

        let stored = tokio::fs::read(parts.join("0.part")).await.expect("read");

        assert_eq!(stored, b"second");
    }

    #[tokio::test]
    async fn discarding_a_part_removes_it_and_tolerates_a_missing_one() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");
        tokio::fs::write(parts.join("0.part"), b"bad bytes")
            .await
            .expect("writing");

        discard_part(&parts.join("0.part"))
            .await
            .expect("discarding an existing part");

        assert!(!parts.join("0.part").exists());

        // Asking twice is not a failure: the caller wants the file gone, and it
        // is gone.
        discard_part(&parts.join("0.part"))
            .await
            .expect("discarding an absent part is fine");
    }

    #[tokio::test]
    async fn storing_leaves_no_temporary_file_behind() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"chunk zero"));

        store(&dir, &manifest).await.expect("first store");
        store(&dir, &manifest).await.expect("second store");

        let mut leftovers = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await.expect("listing");

        while let Some(entry) = entries.next_entry().await.expect("entry") {
            leftovers.push(entry.file_name().to_string_lossy().into_owned());
        }

        assert_eq!(
            leftovers,
            vec![MANIFEST_FILE.to_string()],
            "the temporary must be gone, leaving only the manifest"
        );
        assert!(scratch.path().exists());
    }

    #[tokio::test]
    async fn rewriting_a_manifest_always_leaves_valid_json() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        // Grow the manifest repeatedly, as a live transfer does. Each state
        // must be readable whole; a torn write would leave `load` failing.
        for index in 0..8u64 {
            let payload = format!("chunk number {index}");

            manifest.record(entry("f-1", index, payload.as_bytes()));
            store(&dir, &manifest).await.expect("storing");

            let reloaded = load(&dir)
                .await
                .unwrap_or_else(|error| panic!("manifest unreadable after store {index}: {error}"))
                .expect("present");

            assert_eq!(reloaded.chunks.len() as u64, index + 1);
        }

        let final_manifest = load(&dir).await.expect("reading").expect("present");

        assert_eq!(final_manifest.chunks.len(), 8);
    }

    #[tokio::test]
    async fn storing_creates_the_transfer_directory_if_it_is_missing() {
        let scratch = TempDir::new();
        let dir = scratch.path().join("incoming").join("t-brand-new");

        assert!(!dir.exists());

        let mut manifest = Manifest::new("t-brand-new");
        manifest.record(entry("f-1", 0, b"chunk zero"));

        store(&dir, &manifest).await.expect("storing");

        assert!(manifest_path(&dir).is_file());
    }

    /*
     * ----------------------------------------------------------------------
     * The resume report
     *
     * `verified_completion` is what the receiver tells a sender it already
     * holds. It has to be a claim about disk, not about the manifest: anything
     * it overstates is a chunk the merger will never be given.
     * ----------------------------------------------------------------------
     */

    /// Record `chunks` for `file_id` and write each one's bytes to disk, so the
    /// transfer looks exactly as it would after those chunks were received.
    async fn record_and_write(
        manifest: &mut Manifest,
        transfer_dir: &Path,
        file_id: &str,
        chunks: &[(u64, &[u8])],
    ) {
        let parts = transfer_dir.join(file_id);

        tokio::fs::create_dir_all(&parts).await.expect("mkdir");

        for (index, bytes) in chunks {
            write_part(&part_path(&parts, *index), bytes)
                .await
                .expect("writing the part");

            manifest.record(entry(file_id, *index, bytes));
        }
    }

    #[tokio::test]
    async fn a_transfer_with_no_chunks_reports_empty_completion() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let empty = Manifest::new("t-manifest");

        assert!(
            verified_completion(&empty, &dir).await.is_empty(),
            "nothing received must report nothing, and must not fail"
        );
    }

    #[tokio::test]
    async fn completed_chunks_are_reported_grouped_by_file() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        record_and_write(
            &mut manifest,
            &dir,
            "f-1",
            &[(0, b"one zero".as_slice()), (1, b"one one".as_slice())],
        )
        .await;
        record_and_write(&mut manifest, &dir, "f-2", &[(0, b"two zero".as_slice())]).await;

        let report = verified_completion(&manifest, &dir).await;

        assert_eq!(report.len(), 2, "one group per file that has chunks");

        let first = report
            .iter()
            .find(|file| file.file_id == "f-1")
            .expect("f-1 is reported");
        let second = report
            .iter()
            .find(|file| file.file_id == "f-2")
            .expect("f-2 is reported");

        assert_eq!(
            first.chunks,
            vec![
                CompletedChunk {
                    chunk_index: 0,
                    chunk_length: 8,
                    chunk_hash: sha256_hex(b"one zero"),
                },
                CompletedChunk {
                    chunk_index: 1,
                    chunk_length: 7,
                    chunk_hash: sha256_hex(b"one one"),
                },
            ]
        );
        assert_eq!(second.chunks.len(), 1);
        assert_eq!(second.chunks[0].chunk_hash, sha256_hex(b"two zero"));
    }

    #[tokio::test]
    async fn reported_chunks_are_ordered_by_index() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        // Recorded out of order, as concurrent workers would.
        record_and_write(&mut manifest, &dir, "f-1", &[(2, b"two".as_slice())]).await;
        record_and_write(&mut manifest, &dir, "f-1", &[(0, b"zero".as_slice())]).await;
        record_and_write(&mut manifest, &dir, "f-1", &[(1, b"one".as_slice())]).await;

        let report = verified_completion(&manifest, &dir).await;

        let indices: Vec<u64> = report[0].chunks.iter().map(|c| c.chunk_index).collect();

        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn a_file_with_nothing_completed_is_absent_from_the_report() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"unwritten"));

        // Recorded, but its bytes were never written.
        let report = verified_completion(&manifest, &dir).await;

        assert!(
            report.is_empty(),
            "a file with no verified chunks must be absent, not present and empty"
        );
    }

    #[tokio::test]
    async fn an_altered_chunk_is_not_reported() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        record_and_write(
            &mut manifest,
            &dir,
            "f-1",
            &[(0, b"chunk zero".as_slice()), (1, b"chunk one!".as_slice())],
        )
        .await;

        // Chunk 1's bytes are changed underneath the manifest, same length.
        tokio::fs::write(part_path(&dir.join("f-1"), 1), b"CHUNK ONE!")
            .await
            .expect("altering the part");

        let report = verified_completion(&manifest, &dir).await;

        assert_eq!(report.len(), 1);
        assert_eq!(
            report[0].chunks.len(),
            1,
            "the altered chunk must not be reported as held"
        );
        assert_eq!(report[0].chunks[0].chunk_index, 0);
    }

    #[tokio::test]
    async fn a_truncated_chunk_is_not_reported() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        record_and_write(&mut manifest, &dir, "f-1", &[(0, b"chunk zero".as_slice())]).await;

        tokio::fs::write(part_path(&dir.join("f-1"), 0), b"chun")
            .await
            .expect("truncating the part");

        assert!(
            verified_completion(&manifest, &dir).await.is_empty(),
            "a short chunk must not be reported as held"
        );
    }

    #[tokio::test]
    async fn a_chunk_whose_part_file_vanished_is_not_reported() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        record_and_write(&mut manifest, &dir, "f-1", &[(0, b"chunk zero".as_slice())]).await;

        tokio::fs::remove_file(part_path(&dir.join("f-1"), 0))
            .await
            .expect("removing the part");

        assert!(
            verified_completion(&manifest, &dir).await.is_empty(),
            "a manifest entry with no file behind it must not be reported"
        );
    }

    #[tokio::test]
    async fn only_the_chunks_that_still_verify_are_reported() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");

        record_and_write(
            &mut manifest,
            &dir,
            "f-1",
            &[
                (0, b"intact".as_slice()),
                (1, b"altered!!!".as_slice()),
                (2, b"also intact".as_slice()),
            ],
        )
        .await;

        tokio::fs::write(part_path(&dir.join("f-1"), 1), b"ALTERED!!!")
            .await
            .expect("altering one part");

        let report = verified_completion(&manifest, &dir).await;

        let indices: Vec<u64> = report[0].chunks.iter().map(|c| c.chunk_index).collect();

        assert_eq!(
            indices,
            vec![0, 2],
            "the report must be the verified subset, not every recorded entry"
        );
    }

    #[tokio::test]
    async fn a_restart_reports_the_same_completed_chunks() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let first_run = {
            let mut manifest = Manifest::new("t-manifest");

            record_and_write(
                &mut manifest,
                &dir,
                "f-1",
                &[(0, b"chunk zero".as_slice()), (1, b"chunk one".as_slice())],
            )
            .await;

            store(&dir, &manifest).await.expect("storing");

            verified_completion(&manifest, &dir).await
        };

        // A new process: nothing in memory, only what is on disk.
        let recovered = load_or_default(&dir, "t-manifest").await;
        let second_run = verified_completion(&recovered, &dir).await;

        assert_eq!(second_run, first_run, "the report must survive a restart");
        assert_eq!(second_run[0].chunks.len(), 2);
    }

    /*
     * ----------------------------------------------------------------------
     * The digest itself
     * ----------------------------------------------------------------------
     */

    #[test]
    fn the_chunk_hash_is_sha256_of_the_plaintext() {
        // Pinned by value: the empty string's SHA-256, so a change of algorithm
        // or encoding is a deliberate act rather than a silent one.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * The sender's digests, and what they say is not worth keeping
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn the_senders_digests_survive_a_restart() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let chunks: &[(u64, &[u8])] = &[(0, b"abcd"), (1, b"efgh")];

        let mut manifest = Manifest::new("t-manifest");
        manifest.expected = Some(crate::transfer::integrity::ExpectedTransfer {
            transfer_id: "t-manifest".into(),
            files: vec![test_support::expected_file("f-1", chunks)],
        });

        store(&dir, &manifest).await.expect("storing");

        // Read it back from disk with nothing carried over in memory, which is
        // what a receiver that has restarted sees.
        let reloaded = load(&dir).await.expect("reading").expect("it is there");

        assert_eq!(
            reloaded.expected, manifest.expected,
            "the digests a file is checked against must not be lost with the process"
        );
    }

    #[tokio::test]
    async fn the_senders_digests_do_not_survive_a_manifest_that_cannot_be_read() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");
        manifest.expected = Some(crate::transfer::integrity::ExpectedTransfer {
            transfer_id: "t-manifest".into(),
            files: vec![test_support::expected_file("f-1", &[(0, b"abcd")])],
        });

        store(&dir, &manifest).await.expect("storing");

        fs::write(manifest_path(&dir), b"not a manifest")
            .await
            .expect("damaging the manifest");

        let reloaded = load_or_default(&dir, "t-manifest").await;

        assert!(
            reloaded.expected.is_none(),
            "a manifest that cannot be read is not trusted, digests included"
        );
    }

    /// A manifest and parts on disk for a three-chunk file, with the sender's
    /// description of it.
    async fn received(
        dir: &Path,
        chunks: &[(u64, &[u8])],
    ) -> (Manifest, crate::transfer::integrity::ExpectedFile) {
        let parts = dir.join("f-1");
        fs::create_dir_all(&parts).await.expect("mkdir");

        let mut manifest = Manifest::new("t-manifest");

        for (index, bytes) in chunks {
            write_part(&part_path(&parts, *index), bytes)
                .await
                .expect("writing a part");

            manifest.record(entry("f-1", *index, bytes));
        }

        let described = test_support::expected_file("f-1", chunks);

        (manifest, described)
    }

    #[tokio::test]
    async fn chunks_the_sender_vouches_for_are_kept() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let chunks: &[(u64, &[u8])] = &[(0, b"abcd"), (1, b"efgh"), (2, b"ij")];
        let (mut manifest, described) = received(&dir, chunks).await;

        let discarded =
            discard_untrusted(&mut manifest, &dir.join("f-1"), "f-1", 3, Some(&described)).await;

        assert!(discarded.is_empty(), "nothing was ever wrong");
        assert_eq!(manifest.chunks.len(), 3, "every entry is still held");
        assert_eq!(manifest.accounted(), (10, 3));
    }

    #[tokio::test]
    async fn a_chunk_the_sender_does_not_vouch_for_is_dropped() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let chunks: &[(u64, &[u8])] = &[(0, b"abcd"), (1, b"efgh"), (2, b"ij")];
        let (mut manifest, described) = received(&dir, chunks).await;

        /*
         * The bytes of chunk 1 are replaced, and the entry that recorded them is
         * replaced with it. The receiver's own check — does this `.part` still
         * match what I recorded? — now passes, and only the sender's digests can
         * tell that the file is not the one it described.
         */
        write_part(&part_path(&dir.join("f-1"), 1), b"WXYZ")
            .await
            .expect("rewriting the part");

        manifest.record(entry("f-1", 1, b"WXYZ"));

        let discarded =
            discard_untrusted(&mut manifest, &dir.join("f-1"), "f-1", 3, Some(&described)).await;

        assert_eq!(discarded, vec![1], "only the chunk that disagrees");
        assert_eq!(manifest.chunks.len(), 2);
        assert_eq!(
            manifest.accounted(),
            (6, 2),
            "the dropped chunk's bytes are gone from the count as well"
        );
        assert!(
            !part_path(&dir.join("f-1"), 1).exists(),
            "and its bytes are gone from disk"
        );
    }

    #[tokio::test]
    async fn a_chunk_that_is_no_longer_on_disk_is_dropped_and_reported() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let chunks: &[(u64, &[u8])] = &[(0, b"abcd"), (1, b"efgh")];
        let (mut manifest, described) = received(&dir, chunks).await;

        fs::remove_file(part_path(&dir.join("f-1"), 1))
            .await
            .expect("removing the part");

        let discarded =
            discard_untrusted(&mut manifest, &dir.join("f-1"), "f-1", 2, Some(&described)).await;

        assert_eq!(
            discarded,
            vec![1],
            "a chunk with no bytes behind it has to be sent again"
        );
        assert_eq!(manifest.chunks.len(), 1);
    }

    #[tokio::test]
    async fn with_no_per_chunk_digests_the_whole_file_is_dropped() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let chunks: &[(u64, &[u8])] = &[(0, b"abcd"), (1, b"efgh")];
        let (mut manifest, mut described) = received(&dir, chunks).await;

        // A sender that described the file but not the chunks that make it up:
        // the receiver knows the file is wrong and nothing more.
        described.chunks.clear();

        let discarded =
            discard_untrusted(&mut manifest, &dir.join("f-1"), "f-1", 2, Some(&described)).await;

        assert_eq!(discarded, vec![0, 1], "every chunk is asked for again");
        assert!(manifest.chunks.is_empty());
    }

    /*
     * ----------------------------------------------------------------------
     * The chunk journal
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_journaled_chunk_survives_a_restart_without_a_manifest() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let bytes = b"one chunk".as_slice();
        journal(&dir, &entry("f-1", 2, bytes))
            .await
            .expect("journaling");

        assert!(!manifest_path(&dir).exists(), "nothing compacted yet");

        let reloaded = load_or_default(&dir, "t-manifest").await;

        assert_eq!(
            reloaded
                .entry("f-1", 2)
                .map(|entry| entry.chunk_hash.as_str()),
            Some(sha256_hex(bytes).as_str()),
            "the journal is replayed over the (absent) manifest"
        );
    }

    #[tokio::test]
    async fn a_partial_journal_line_is_skipped_rather_than_failing_the_load() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        journal(&dir, &entry("f-1", 0, b"first"))
            .await
            .expect("journaling");

        // What a crash mid-append leaves: half a line.
        let mut torn = std::fs::read_to_string(journal_path(&dir)).expect("reading the journal");

        torn.push_str("{\"file_id\":\"f-1\",\"chunk_index\":1,\"chunk_le");

        std::fs::write(journal_path(&dir), torn).expect("tearing the journal");

        let reloaded = load_or_default(&dir, "t-manifest").await;

        assert_eq!(reloaded.chunks.len(), 1);
        assert_eq!(
            reloaded.entry("f-1", 0).expect("the whole line").chunk_index,
            0
        );
    }

    #[tokio::test]
    async fn storing_folds_the_journal_into_the_manifest_and_empties_it() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"first"));
        manifest.record(entry("f-1", 1, b"second"));

        store(&dir, &manifest).await.expect("storing");

        assert!(!journal_path(&dir).exists(), "the journal is spent");

        let reloaded = load(&dir)
            .await
            .expect("reading")
            .expect("the manifest is there");

        assert_eq!(reloaded.chunks.len(), 2);
    }

    #[tokio::test]
    async fn storing_a_discarded_chunk_does_not_resurrect_it() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();

        journal(&dir, &entry("f-1", 0, b"first"))
            .await
            .expect("journaling");

        journal(&dir, &entry("f-1", 1, b"second"))
            .await
            .expect("journaling");

        // The second chunk was found to be wrong and dropped: the manifest
        // written now is the whole truth, and the journal behind it must not
        // bring back what it left out.
        let mut manifest = Manifest::new("t-manifest");
        manifest.record(entry("f-1", 0, b"first"));

        store(&dir, &manifest).await.expect("storing");

        let reloaded = load_or_default(&dir, "t-manifest").await;

        assert_eq!(reloaded.chunks.len(), 1);
        assert!(reloaded.entry("f-1", 1).is_none());
    }

    #[tokio::test]
    async fn a_chunk_with_its_own_stamp_is_not_read_back() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        std::fs::create_dir_all(&parts).expect("creating the parts directory");

        let bytes = b"one chunk".as_slice();
        let part = part_path(&parts, 0);

        write_part(&part, bytes).await.expect("writing the part");

        let stamped = std::fs::metadata(&part).expect("the part is there");
        let written_nanos = modified_nanos(&stamped);

        let mut recorded = entry("f-1", 0, bytes);
        recorded.written_nanos = written_nanos;

        assert!(
            part_matches(&part, &recorded).await,
            "the bytes are what was written and the stamp says untouched"
        );

        /*
         * An entry with no stamp — written by an older build — is still
         * verified, and still by hashing.
         */
        let mut unstamped = entry("f-1", 0, bytes);
        unstamped.written_nanos = None;

        assert!(part_matches(&part, &unstamped).await);
    }

    #[tokio::test]
    async fn a_stamped_chunk_whose_bytes_changed_does_not_match() {
        let scratch = TempDir::new();
        let dir = scratch.transfer_dir();
        let parts = dir.join("f-1");

        std::fs::create_dir_all(&parts).expect("creating the parts directory");

        let bytes = b"one chunk".as_slice();
        let part = part_path(&parts, 0);

        write_part(&part, bytes).await.expect("writing the part");

        let mut recorded = entry("f-1", 0, bytes);
        recorded.written_nanos =
            modified_nanos(&std::fs::metadata(&part).expect("the part is there"));

        // Same length, different bytes, written after the stamp was taken.
        write_part(&part, b"another!!").await.expect("altering");

        assert!(
            !part_matches(&part, &recorded).await,
            "the bytes are not the ones the entry describes"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * The in-memory map and the on-disk array
     * ----------------------------------------------------------------------
     */

    #[test]
    fn chunks_round_trip_to_a_sorted_json_array() {
        let mut manifest = Manifest::new("t-manifest");

        // Deliberately out of order, with a replace in the middle.
        manifest.record(entry("f-2", 1, b"b"));
        manifest.record(entry("f-1", 3, b"c"));
        manifest.record(entry("f-1", 0, b"a"));
        manifest.record(entry("f-2", 0, b"old"));
        manifest.record(entry("f-2", 0, b"new"));

        let json = serde_json::to_value(&manifest).expect("serialising");
        let entries = json["chunks"].as_array().expect("an array");

        let order: Vec<(&str, u64)> = entries
            .iter()
            .map(|entry| {
                (
                    entry["file_id"].as_str().expect("a file id"),
                    entry["chunk_index"].as_u64().expect("an index"),
                )
            })
            .collect();

        assert_eq!(order, vec![("f-1", 0), ("f-1", 3), ("f-2", 0), ("f-2", 1)]);
        assert_eq!(manifest.chunks.len(), 4);

        let reloaded: Manifest = serde_json::from_value(json.clone()).expect("deserialising");

        assert_eq!(reloaded, manifest);
        assert_eq!(
            serde_json::to_value(&reloaded).expect("again"),
            json,
            "a load and a save must produce the same bytes"
        );
    }

    #[test]
    fn an_older_array_in_arrival_order_reads_and_normalises() {
        let legacy = serde_json::json!({
            "version": MANIFEST_VERSION,
            "transfer_id": "t-manifest",
            "chunks": [
                serde_json::to_value(entry("f-2", 0, b"two")).expect("json"),
                serde_json::to_value(entry("f-1", 0, b"one")).expect("json"),
            ],
        });

        let manifest: Manifest = serde_json::from_value(legacy).expect("reading a v1 manifest");
        let normalised = serde_json::to_value(&manifest).expect("serialising");

        let ids: Vec<&str> = normalised["chunks"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|entry| entry["file_id"].as_str().expect("a file id"))
            .collect();

        assert_eq!(ids, vec!["f-1", "f-2"]);
    }

    #[test]
    fn pruning_from_a_snapshot_keeps_a_chunk_retried_meanwhile() {
        let mut manifest = Manifest::new("t-manifest");

        manifest.record(entry("f-1", 0, b"first"));

        let snapshot = manifest.file_entries("f-1", 1);

        // A retry lands between the verification and the removal.
        manifest.record(entry("f-1", 0, b"retried"));

        apply_prune(&mut manifest, "f-1", &snapshot);

        assert_eq!(
            manifest
                .entry("f-1", 0)
                .expect("still recorded")
                .chunk_hash,
            sha256_hex(b"retried")
        );
    }

    #[test]
    fn forgetting_one_file_leaves_the_other_files_alone() {
        let mut manifest = Manifest::new("t-manifest");

        manifest.record(entry("f-1", 0, b"a"));
        manifest.record(entry("f-1", 1, b"b"));
        manifest.record(entry("f-2", 0, b"c"));

        manifest.remove_file("f-1");

        assert_eq!(manifest.chunks.len(), 1);
        assert!(manifest.entry("f-1", 0).is_none());
        assert!(manifest.entry("f-2", 0).is_some());
    }
}
