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

use crate::transfer::atomic_file;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::fs;

/// The manifest's file name inside a transfer's directory.
pub const MANIFEST_FILE: &str = "manifest.json";

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
}

/// A transfer's completed chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version. See [`MANIFEST_VERSION`].
    pub version: u32,
    /// The transfer this manifest belongs to, so a manifest found under the
    /// wrong directory can be noticed.
    pub transfer_id: String,
    /// One entry per completed chunk. `#[serde(default)]` so a manifest with no
    /// chunks yet still parses.
    #[serde(default)]
    pub chunks: Vec<ChunkEntry>,
    /// What the sender says this transfer's files hash to.
    ///
    /// The one part of the manifest the receiver did not write: it arrives with
    /// the transfer's metadata and is kept beside the chunks so that it survives
    /// a restart along with them. Absent for a sender that describes no digests,
    /// in which case there is nothing to check an assembled file against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<crate::transfer::integrity::ExpectedTransfer>,
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
            .iter()
            .map(|chunk| chunk.chunk_length)
            .sum::<u64>();

        (bytes, self.chunks.len() as u64)
    }

    /// An empty manifest for `transfer_id`.
    pub fn new(transfer_id: impl Into<String>) -> Self {
        Self {
            version: MANIFEST_VERSION,
            transfer_id: transfer_id.into(),
            chunks: Vec::new(),
            expected: None,
        }
    }

    /// The record for one chunk, if it has one.
    pub fn entry(&self, file_id: &str, chunk_index: u64) -> Option<&ChunkEntry> {
        self.chunks
            .iter()
            .find(|entry| entry.file_id == file_id && entry.chunk_index == chunk_index)
    }

    /// Record a chunk as completed, replacing any earlier record for the same
    /// chunk.
    ///
    /// Replacing rather than appending matters: a chunk that is re-sent and
    /// accepted must supersede the entry from whenever it was last written, or
    /// the manifest would keep describing a `.part` that no longer exists in
    /// that shape.
    pub fn record(&mut self, entry: ChunkEntry) {
        match self
            .chunks
            .iter_mut()
            .find(|slot| slot.file_id == entry.file_id && slot.chunk_index == entry.chunk_index)
        {
            Some(slot) => *slot = entry,
            None => self.chunks.push(entry),
        }
    }

    /// Drop any record of one chunk, so it reads as never received.
    ///
    /// Used when a chunk's bytes are found not to match its entry: the claim
    /// has to go, or the next run would skip a chunk known to be wrong.
    pub fn forget(&mut self, file_id: &str, chunk_index: u64) {
        self.chunks
            .retain(|entry| !(entry.file_id == file_id && entry.chunk_index == chunk_index));
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
    match load(transfer_dir).await {
        Ok(Some(manifest)) => manifest,
        Ok(None) => Manifest::new(transfer_id),
        Err(error) => {
            tracing::warn!(
                transfer_id,
                path = %manifest_path(transfer_dir).display(),
                %error,
                "ignoring unreadable transfer manifest; completed chunks will be re-written"
            );

            Manifest::new(transfer_id)
        }
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
pub async fn store(transfer_dir: &Path, manifest: &Manifest) -> io::Result<()> {
    fs::create_dir_all(transfer_dir).await?;

    let bytes = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;

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

    match fs::read(part).await {
        Ok(bytes) => sha256_hex(&bytes) == entry.chunk_hash,
        Err(_) => false,
    }
}

/// The path of one chunk's `.part` file inside a file's parts directory.
pub fn part_path(parts_dir: &Path, chunk_index: u64) -> PathBuf {
    parts_dir.join(format!("{chunk_index}.part"))
}

/// The length and hex SHA-256 of a `.part` file, or `None` if it is not there.
///
/// [`part_matches`] judges a chunk against what the receiver recorded; this
/// hands back what is actually on disk, so a chunk can be judged against what
/// the *sender* said it would be. They differ exactly when the receiver's record
/// and the sender's are not describing the same bytes.
pub async fn part_digest(part: &Path) -> Option<(u64, String)> {
    let bytes = fs::read(part).await.ok()?;

    Some((bytes.len() as u64, sha256_hex(&bytes)))
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verification {
    /// Every chunk was present and hashed to its recorded digest.
    pub complete: bool,
    /// At least one chunk failed and was removed from disk and from `record`.
    /// The caller must persist the manifest again.
    pub pruned: bool,
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
pub async fn verify_and_prune(
    record: &mut Manifest,
    parts_dir: &Path,
    file_id: &str,
    total: u64,
) -> Verification {
    let mut complete = true;
    let mut pruned = false;

    for index in 0..total {
        if chunk_complete(record, parts_dir, file_id, index).await {
            continue;
        }

        complete = false;

        let part = part_path(parts_dir, index);
        let has_bytes = fs::metadata(&part).await.is_ok();
        let has_entry = record.entry(file_id, index).is_some();

        if !has_bytes && !has_entry {
            // Simply not here yet. There is no claim to withdraw and no bytes
            // to remove, so this is a gap rather than damage.
            continue;
        }

        /*
         * There is a claim, or bytes, or both, and they do not agree. Drop both
         * halves so nothing downstream can act on either.
         */
        let _ = discard_part(&part).await;

        record.forget(file_id, index);

        pruned = true;

        tracing::warn!(
            file_id,
            chunk = index,
            had_entry = has_entry,
            had_bytes = has_bytes,
            "chunk failed verification; discarded it and its manifest entry"
        );
    }

    Verification { complete, pruned }
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

    for entry in &record.chunks {
        /*
         * `chunk_complete` would look the entry up again by (file_id, index);
         * we already hold it, so this applies the same test — file present, at
         * the recorded length, hashing to the recorded digest — directly. That
         * keeps the scan linear in the number of chunks instead of quadratic.
         */
        let parts_dir = transfer_dir.join(&entry.file_id);

        if !part_matches(&part_path(&parts_dir, entry.chunk_index), entry).await {
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
            })
        );
        assert_eq!(reloaded.version, MANIFEST_VERSION);
        assert_eq!(reloaded.transfer_id, "t-manifest");
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
}
