//! What a transfer's files are supposed to be, and the check that a
//! reconstructed file really is.
//!
//! Per-chunk hashing (see [`crate::transfer::manifest`]) proves that the bytes
//! the receiver holds are the bytes it was sent. It cannot prove that the bytes
//! it was sent are the bytes the sender meant to send: the receiver records each
//! chunk under the digest of what arrived, so a chunk corrupted in flight — or a
//! manifest damaged after the fact — verifies against itself and passes.
//!
//! What closes that gap is an expectation the receiver did not write. The
//! sender digests its own files before the first chunk moves, and sends the
//! result with the transfer's metadata. The receiver checks the assembled file
//! against it before publishing anything, and a file that does not match is not
//! reported as delivered — see [`crate::transfer::writer::assemble_and_publish`].
//!
//! Two digests are kept per file, and they do different jobs:
//!
//! * the whole-file digest is the final check, and the only one that can catch a
//!   file assembled from chunks that each verified on their own;
//! * the per-chunk digests say *which* chunks are wrong once the whole-file
//!   check has failed, so the sender can be asked for those and not for the
//!   whole file.
//!
//! Both are computed in one pass over each source file. The sender reads its
//! files once more than it otherwise would, before the first chunk goes out; the
//! alternative is a transfer that cannot tell a delivered file from a plausible
//! one.

use crate::models::file_info::{modified_nanos, FileInfo};
use crate::transfer::errors::{Result, TransferError};
use crate::transfer::manifest::sha256_hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};
use tokio::{fs, sync::Semaphore, task::JoinSet};

/// How many files may be read and hashed at once while planning.
///
/// Hashing is CPU-bound and the reads are blocking-pool work, so a handful of
/// files in parallel keeps both busy on a machine with a multi-core CPU and a
/// device that can serve overlapping reads. More than this stops helping on a
/// spinning disk and only increases memory (`chunk_size` per in-flight file).
const MAX_CONCURRENT_FILE_HASHES: usize = 4;

/// How much of one file is hashed before planning logs its progress.
///
/// Without this a single large file emits nothing between "starting" and
/// "finished", which is exactly the silence that makes a start look stuck.
const PLAN_PROGRESS_BYTES: u64 = 256 * 1024 * 1024;

/// How many chunks of one file are read and hashed at once.
///
/// Reading is blocking file I/O and hashing is CPU work, so a handful in flight
/// keeps both busy on an SSD. What bounds memory is
/// [`MAX_CONCURRENT_CHUNK_READS`], not this.
const PARALLEL_CHUNK_READS: usize = 4;

/// The most chunks being read at once across every file of a plan.
///
/// Memory bound: this many `chunk_size` buffers — at most 32 MiB each, so
/// 128 MiB — however many files the transfer has.
const MAX_CONCURRENT_CHUNK_READS: usize = 4;

/// The cap on concurrent chunk reads, shared by every file being hashed.
fn chunk_read_slots() -> &'static Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

    SLOTS.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CHUNK_READS)))
}

/// Where a plan reports what it has read, for a shell that wants to show it.
///
/// Called with the bytes hashed so far and the bytes this plan will hash in
/// total. Both are transfer-wide: several files may be hashed at once.
pub type PlanProgress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// The counter a plan's files share, so a reporter sees one transfer-wide
/// total rather than one file's progress at a time.
#[derive(Clone)]
struct PlanReporter {
    hashed: Arc<AtomicU64>,
    total: u64,
    sink: PlanProgress,
}

impl PlanReporter {
    fn add(&self, bytes: u64) {
        let hashed = self.hashed.fetch_add(bytes, Ordering::Relaxed) + bytes;

        (self.sink)(hashed.min(self.total), self.total);
    }
}

/// One chunk, as the sender intends to send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedChunk {
    /// Its index within the file.
    pub index: u64,
    /// How many bytes it holds.
    pub length: u64,
    /// Hex SHA-256 of those bytes.
    pub sha256: String,
}

/// One file, as the sender intends to send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedFile {
    /// The file's id, which is what a chunk's `File-Id` header names.
    pub file_id: String,
    /// The path the sender will ask the receiver to write it to.
    pub relative_path: String,
    /// The size the transfer will carry for it.
    pub size: u64,
    /// Hex SHA-256 of the whole file, in the order its chunks are sent.
    pub sha256: String,
    /// The digests of the chunks that make it up, by index.
    ///
    /// Absent from a sender that only describes whole files; the receiver then
    /// knows a file is wrong but not which chunk to ask for.
    #[serde(default)]
    pub chunks: Vec<ExpectedChunk>,
}

impl ExpectedFile {
    /// The expectation for one chunk of this file, if the sender described it.
    pub fn chunk(&self, index: u64) -> Option<&ExpectedChunk> {
        self.chunks.iter().find(|chunk| chunk.index == index)
    }
}

/// The digests a whole transfer is checked against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedTransfer {
    /// The transfer these digests belong to, so a manifest that arrives for the
    /// wrong one can be noticed rather than acted on.
    pub transfer_id: String,
    /// One entry per file being sent.
    #[serde(default)]
    pub files: Vec<ExpectedFile>,
}

impl ExpectedTransfer {
    /// The expectation for one file of this transfer.
    pub fn file(&self, file_id: &str) -> Option<&ExpectedFile> {
        self.files.iter().find(|file| file.file_id == file_id)
    }
}

/// Read every file once, digesting it whole and chunk by chunk.
///
/// The chunk boundaries are [`crate::transfer::chunker::jobs`]'s, so the digests
/// describe exactly the bytes that will be sent — an empty file is one empty
/// chunk, and the last chunk of a file is its tail. Only the first `file.size`
/// bytes are read, which is all the transfer will carry.
///
/// Fails rather than guesses when a file is shorter than the transfer says it
/// is: a plan built from bytes that are not there would describe a file the
/// sender cannot send.
pub async fn plan(
    transfer_id: &str,
    files: &[FileInfo],
    chunk_size: usize,
) -> Result<ExpectedTransfer> {
    plan_reusing(transfer_id, files, chunk_size, None, None).await
}

/// [`plan`], telling `progress` how much it has read as it goes.
pub async fn plan_with_progress(
    transfer_id: &str,
    files: &[FileInfo],
    chunk_size: usize,
    progress: Option<PlanProgress>,
) -> Result<ExpectedTransfer> {
    plan_reusing(transfer_id, files, chunk_size, None, progress).await
}

/// Work out the digests for `files`, reusing what `previous` already described.
///
/// `previous` is a plan from an earlier run, or the one this session already
/// worked out. An entry is reused only when *all* of the following hold for its
/// file:
///
/// * the file still has the size and relative path the plan describes,
/// * its chunk count and every chunk's index and length still match the plan,
/// * its current modification time equals the one recorded when it was scanned.
///
/// Anything else is hashed again from the file as it stands. The stamp is what
/// makes this safe: a file edited after planning is re-read rather than
/// described by digests of bytes it no longer holds. A file whose stamp is
/// unknown — an older state record, or a platform that reports no modification
/// time — is likewise hashed again.
///
/// Files that do need reading are hashed with bounded parallelism
/// ([`MAX_CONCURRENT_FILE_HASHES`]); the answer is assembled in the input order,
/// so which file's error is reported is the same as it was sequentially.
pub async fn plan_reusing(
    transfer_id: &str,
    files: &[FileInfo],
    chunk_size: usize,
    previous: Option<&ExpectedTransfer>,
    progress: Option<PlanProgress>,
) -> Result<ExpectedTransfer> {
    let started = std::time::Instant::now();
    let chunk_size = chunk_size.max(1) as u64;

    // A plan for another transfer says nothing about these files.
    let previous = previous.filter(|plan| plan.transfer_id == transfer_id);

    let mut outcomes: Vec<Option<Result<ExpectedFile>>> = Vec::with_capacity(files.len());
    let mut to_hash: Vec<(usize, FileInfo)> = Vec::new();
    let mut reused = 0usize;

    for (position, file) in files.iter().enumerate() {
        let reusable = match previous.and_then(|plan| plan.file(&file.id)) {
            Some(plan)
                if expected_file_fits(plan, file, chunk_size) && file_is_unchanged(file).await =>
            {
                Some(plan)
            }

            _ => None,
        };

        match reusable {
            Some(plan) => {
                reused += 1;
                outcomes.push(Some(Ok(plan.clone())));
            }

            None => {
                to_hash.push((position, file.clone()));
                outcomes.push(None);
            }
        }
    }

    /*
     * Progress is measured against the bytes this plan has to read, not the
     * whole transfer: a reused entry costs nothing and must not make the first
     * report look like it is already half done.
     */
    let reporter = progress.map(|sink| PlanReporter {
        hashed: Arc::new(AtomicU64::new(0)),
        total: to_hash.iter().map(|(_, file)| file.size).sum(),
        sink,
    });

    let hashed = hash_files(transfer_id, &to_hash, chunk_size, reporter).await;

    for (position, outcome) in hashed {
        if position == usize::MAX {
            // A hashing task panicked. There is no file to attribute it to, so
            // report it before any per-file result can be mistaken for the
            // cause.
            return match outcome {
                Ok(_) => unreachable!("a failed hashing task carries no plan"),
                Err(error) => Err(error),
            };
        }

        outcomes[position] = Some(outcome);
    }

    let mut described = Vec::with_capacity(files.len());

    for outcome in outcomes {
        described.push(outcome.expect("every position is either reused or hashed")?);
    }

    tracing::info!(
        transfer_id,
        files = files.len(),
        reused,
        hashed = to_hash.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "digest plan ready"
    );

    Ok(ExpectedTransfer {
        transfer_id: transfer_id.to_string(),
        files: described,
    })
}

/// Whether a file still looks exactly as it did when it was scanned.
///
/// Both halves matter: the size catches a rewrite, and the modification time
/// catches a rewrite of the same length. A missing stamp is not evidence, so
/// it is treated as changed.
pub async fn file_is_unchanged(file: &FileInfo) -> bool {
    let Some(recorded) = file.modified_nanos else {
        return false;
    };

    let Ok(metadata) = fs::metadata(&file.path).await else {
        return false;
    };

    metadata.len() == file.size && modified_nanos(&metadata) == Some(recorded)
}

/// Whether `plan` describes the file as [`plan_file`] would describe it.
///
/// Structure only — no disk access. It rejects a truncated, reordered or
/// hand-edited plan before its digests are trusted, and it is what makes a
/// persisted plan safe to use for a file whose stamp still matches.
fn expected_file_fits(plan: &ExpectedFile, file: &FileInfo, chunk_size: u64) -> bool {
    if plan.size != file.size || plan.relative_path != file.relative_path {
        return false;
    }

    let chunks = file.size.div_ceil(chunk_size).max(1);

    if plan.chunks.len() as u64 != chunks {
        return false;
    }

    plan.chunks.iter().enumerate().all(|(position, chunk)| {
        let index = position as u64;
        let length = (file.size - index * chunk_size).min(chunk_size);

        chunk.index == index && chunk.length == length && !chunk.sha256.is_empty()
    })
}

/// Read the modification stamps `files` carry right now.
///
/// Called immediately *before* a plan is computed, so the stamps describe a
/// moment no later than the bytes the plan was hashed from. If a file changes
/// while it is being hashed, the stamp is the older one and the next resume
/// re-hashes it — which is the safe direction for the stamp to be wrong in.
pub async fn current_stamps(files: &[FileInfo]) -> Vec<(String, Option<u64>)> {
    let mut stamps = Vec::with_capacity(files.len());

    for file in files {
        let stamp = fs::metadata(&file.path)
            .await
            .ok()
            .and_then(|metadata| modified_nanos(&metadata));

        stamps.push((file.id.clone(), stamp));
    }

    stamps
}

/// Whether a persisted plan describes these files as they are right now.
///
/// Used before a plan is reused wholesale: the per-file checks are the same ones
/// [`plan_reusing`] applies, with the transfer id and the file list's shape
/// added. A plan that fails this is discarded and recomputed; one that passes
/// needs no further hashing until a file changes again.
///
/// The stamp is a heuristic, so the guarantee does not rest on it: a change that
/// somehow escaped it means the sender sends bytes the plan does not describe,
/// and the receiver refuses to publish a file whose whole-file digest does not
/// match. The failure mode is a refused transfer, never a silently wrong file.
pub async fn plan_is_current(
    transfer_id: &str,
    expected: &ExpectedTransfer,
    files: &[FileInfo],
    chunk_size: usize,
) -> bool {
    if expected.transfer_id != transfer_id || expected.files.len() != files.len() {
        return false;
    }

    let chunk_size = chunk_size.max(1) as u64;

    for file in files {
        let Some(plan) = expected.file(&file.id) else {
            return false;
        };

        if !expected_file_fits(plan, file, chunk_size) || !file_is_unchanged(file).await {
            return false;
        }
    }

    true
}

/// Read and hash the files that were not reused, at most
/// [`MAX_CONCURRENT_FILE_HASHES`] at a time.
///
/// Returns `(position, outcome)` pairs in completion order. `usize::MAX` marks a
/// task that panicked; see [`plan_reusing`].
async fn hash_files(
    transfer_id: &str,
    work: &[(usize, FileInfo)],
    chunk_size: u64,
    reporter: Option<PlanReporter>,
) -> Vec<(usize, Result<ExpectedFile>)> {
    if work.len() <= 1 {
        let mut results = Vec::with_capacity(work.len());

        for (position, file) in work {
            results.push((
                *position,
                plan_file(transfer_id, file, chunk_size, reporter.as_ref()).await,
            ));
        }

        return results;
    }

    let parallel = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(MAX_CONCURRENT_FILE_HASHES)
        .min(work.len())
        .max(1);

    let slots = Arc::new(Semaphore::new(parallel));
    let mut tasks = JoinSet::new();

    for (position, file) in work {
        let permit = slots
            .clone()
            .acquire_owned()
            .await
            .expect("the hashing semaphore is never closed");

        let position = *position;
        let file = file.clone();
        let transfer_id = transfer_id.to_string();
        let reporter = reporter.clone();

        tasks.spawn(async move {
            let _permit = permit;
            (
                position,
                plan_file(&transfer_id, &file, chunk_size, reporter.as_ref()).await,
            )
        });
    }

    let mut results = Vec::with_capacity(work.len());

    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(result) => results.push(result),

            Err(error) => {
                tracing::error!(%error, "a file hashing task failed");

                results.push((
                    usize::MAX,
                    Err(TransferError::Invalid(format!(
                        "a file hashing task failed: {error}"
                    ))),
                ));
            }
        }
    }

    results
}

async fn plan_file(
    transfer_id: &str,
    file: &FileInfo,
    chunk_size: u64,
    reporter: Option<&PlanReporter>,
) -> Result<ExpectedFile> {
    let started = std::time::Instant::now();

    tracing::info!(
        transfer_id,
        file_id = %file.id,
        path = %file.path.display(),
        size = file.size,
        "hashing a source file to plan its digests"
    );

    /*
     * Opened once here so an unreadable file fails with the message it has
     * always failed with, rather than with whichever chunk read noticed first.
     */
    std::fs::File::open(&file.path).map_err(|error| unreadable(transfer_id, file, error))?;

    let total_chunks = file.size.div_ceil(chunk_size).max(1);
    let mut whole = Sha256::new();
    let mut chunks = Vec::with_capacity(total_chunks as usize);
    let mut hashed = 0u64;
    let mut next_report = PLAN_PROGRESS_BYTES;
    let mut start = 0u64;

    /*
     * Chunks are read in blocks of [`PARALLEL_CHUNK_READS`] and joined in the
     * order they were spawned, so the bytes reach `whole` in file order and the
     * whole-file digest is byte-for-byte what the sequential pass produced.
     * A block, not a sliding window, keeps a slow chunk from making the ones
     * behind it pile up in memory.
     */
    while start < total_chunks {
        let end = (start + PARALLEL_CHUNK_READS as u64).min(total_chunks);
        let mut reads = Vec::with_capacity((end - start) as usize);

        for index in start..end {
            let slot = chunk_read_slots()
                .clone()
                .acquire_owned()
                .await
                .expect("the chunk read semaphore is never closed");

            let path = file.path.clone();
            let offset = index * chunk_size;
            let length = (file.size - offset).min(chunk_size) as usize;

            reads.push(tokio::task::spawn_blocking(move || {
                let _slot = slot;

                /*
                 * Declared after the slot so it is dropped before it: a probe
                 * that outlived its slot would be counted while the next read
                 * already holds the slot it freed.
                 */
                #[cfg(test)]
                let _probe = read_probe::Probe::enter();

                read_chunk(&path, offset, length)
            }));
        }

        let mut block_bytes = 0u64;

        for (position, read) in reads.into_iter().enumerate() {
            let index = start + position as u64;

            let (bytes, sha256) = read
                .await
                .map_err(|error| {
                    TransferError::Invalid(format!(
                        "a task hashing {} failed: {error}",
                        file.path.display()
                    ))
                })?
                .map_err(|error| unreadable(transfer_id, file, error))?;

            whole.update(&bytes);

            hashed += bytes.len() as u64;
            block_bytes += bytes.len() as u64;

            chunks.push(ExpectedChunk {
                index,
                length: bytes.len() as u64,
                sha256,
            });
        }

        if let Some(reporter) = reporter {
            reporter.add(block_bytes);
        }

        if hashed >= next_report {
            tracing::info!(
                transfer_id,
                file_id = %file.id,
                hashed,
                total = file.size,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "still hashing a source file"
            );

            next_report = next_report.saturating_add(PLAN_PROGRESS_BYTES);
        }

        start = end;
    }

    tracing::info!(
        transfer_id,
        file_id = %file.id,
        size = file.size,
        chunks = chunks.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "finished hashing a source file"
    );

    Ok(ExpectedFile {
        file_id: file.id.clone(),
        relative_path: file.relative_path.clone(),
        size: file.size,
        sha256: hex::encode(whole.finalize()),
        chunks,
    })
}

/// Read one chunk from its own handle and digest it, on the blocking pool.
///
/// Its own handle because a shared file position would race between the reads;
/// one `open` per chunk is negligible against a `chunk_size` read.
fn read_chunk(path: &Path, offset: u64, length: usize) -> std::io::Result<(Vec<u8>, String)> {
    let mut source = std::fs::File::open(path)?;

    source.seek(SeekFrom::Start(offset))?;

    let mut bytes = vec![0u8; length];

    source.read_exact(&mut bytes)?;

    let sha256 = sha256_hex(&bytes);

    Ok((bytes, sha256))
}

/// The error a file that cannot be read to the end has always produced.
fn unreadable(transfer_id: &str, file: &FileInfo, error: std::io::Error) -> TransferError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        TransferError::Invalid(format!(
            "{} is shorter than the transfer says it is: {error}",
            file.path.display()
        ))
    } else {
        TransferError::Invalid(format!(
            "cannot read {} to work out what transfer {transfer_id} will send: {error}",
            file.path.display()
        ))
    }
}

/// Test-only observation of how many chunk reads are running at once.
#[cfg(test)]
mod read_probe {
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    pub static PEAK: AtomicUsize = AtomicUsize::new(0);

    pub struct Probe;

    impl Probe {
        pub fn enter() -> Self {
            let active = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;

            PEAK.fetch_max(active, Ordering::SeqCst);

            Self
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::chunker;
    use crate::transfer::manifest::test_support::TempDir;

    /// A file of `size` bytes with a reproducible pattern.
    async fn source(scratch: &TempDir, name: &str, size: usize) -> FileInfo {
        let path = scratch.path().join(name);
        let bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();

        fs::write(&path, &bytes).await.expect("writing the source");

        let stamp = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| modified_nanos(&metadata));

        FileInfo {
            id: format!("f-{name}"),
            path,
            relative_path: name.to_string(),
            size: size as u64,
            modified_nanos: stamp,
        }
    }

    #[tokio::test]
    async fn a_plan_describes_the_bytes_the_chunker_will_send() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let expected = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        let planned = &expected.files[0];

        assert_eq!(planned.file_id, "f-a.bin");
        assert_eq!(planned.relative_path, "a.bin");
        assert_eq!(planned.size, 10);

        let jobs = chunker::jobs("t-1", std::slice::from_ref(&file), 4);

        assert_eq!(planned.chunks.len(), jobs.len());

        for (chunk, job) in planned.chunks.iter().zip(&jobs) {
            assert_eq!(chunk.index, job.index);
            assert_eq!(chunk.length, job.length as u64);
            assert_eq!(
                chunk.sha256,
                crate::transfer::resume::chunk_digest(job)
                    .await
                    .expect("digesting the job"),
                "the plan must describe the chunk the uploader will read"
            );
        }
    }

    #[tokio::test]
    async fn the_whole_file_digest_is_the_file_s() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let expected = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        let bytes = fs::read(&file.path).await.expect("reading the source");

        assert_eq!(
            expected.files[0].sha256,
            sha256_hex(&bytes),
            "the final check has to be against the file the sender holds"
        );
    }

    #[tokio::test]
    async fn a_plan_of_many_blocks_keeps_order_and_the_whole_file_digest() {
        // 32 chunks at 128 bytes, four chunks to a block: eight blocks.
        let scratch = TempDir::new();
        let file = source(&scratch, "big.bin", 4096).await;

        let expected = plan("t-1", std::slice::from_ref(&file), 128)
            .await
            .expect("planning");

        let planned = &expected.files[0];
        let jobs = chunker::jobs("t-1", std::slice::from_ref(&file), 128);

        assert_eq!(planned.chunks.len(), jobs.len());

        for (position, (chunk, job)) in planned.chunks.iter().zip(&jobs).enumerate() {
            assert_eq!(
                chunk.index, position as u64,
                "chunks are reported in file order, not read order"
            );

            assert_eq!(
                chunk.sha256,
                crate::transfer::resume::chunk_digest(job)
                    .await
                    .expect("digesting the job")
            );
        }

        let bytes = fs::read(&file.path).await.expect("reading the source");

        assert_eq!(
            planned.sha256,
            sha256_hex(&bytes),
            "the whole-file digest is over the bytes in file order"
        );
    }

    #[tokio::test]
    async fn planning_reports_the_bytes_it_has_read() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 1024).await;

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);

        plan_with_progress(
            "t-1",
            std::slice::from_ref(&file),
            256,
            Some(Arc::new(move |hashed, total| {
                sink.lock()
                    .expect("progress lock poisoned")
                    .push((hashed, total));
            })),
        )
        .await
        .expect("planning");

        let seen = seen.lock().expect("progress lock poisoned").clone();

        assert!(
            !seen.is_empty(),
            "a plan of this size reports at least once"
        );
        assert!(
            seen.iter().all(|(_, total)| *total == 1024),
            "the total never changes while planning"
        );
        assert!(
            seen.windows(2).all(|pair| pair[0].0 <= pair[1].0),
            "the hashed count only ever moves forward"
        );
        assert_eq!(
            seen.last().copied(),
            Some((1024, 1024)),
            "the last report is the whole file"
        );
    }

    #[tokio::test]
    async fn a_plan_never_reads_more_chunks_at_once_than_its_cap() {
        read_probe::PEAK.store(0, std::sync::atomic::Ordering::SeqCst);

        // Sixteen 256 KiB chunks; enough reads to overlap many times over.
        let scratch = TempDir::new();
        let file = source(&scratch, "big.bin", 4 * 1024 * 1024).await;

        plan("t-1", std::slice::from_ref(&file), 256 * 1024)
            .await
            .expect("planning");

        let peak = read_probe::PEAK.load(std::sync::atomic::Ordering::SeqCst);

        assert!(peak >= 1, "the plan did read chunks");
        assert!(
            peak <= MAX_CONCURRENT_CHUNK_READS,
            "at most {MAX_CONCURRENT_CHUNK_READS} chunk buffers may be live, saw {peak}"
        );
    }

    #[tokio::test]
    async fn an_empty_file_is_one_empty_chunk() {
        let scratch = TempDir::new();
        let file = source(&scratch, "empty.bin", 0).await;

        let expected = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        let planned = &expected.files[0];

        assert_eq!(planned.chunks.len(), 1, "the chunker sends one chunk");
        assert_eq!(planned.chunks[0].length, 0);
        assert_eq!(planned.chunks[0].sha256, sha256_hex(b""));
        assert_eq!(planned.sha256, sha256_hex(b""));
    }

    /*
     * ----------------------------------------------------------------------
     * A plan is reused only while the files still match it
     * ----------------------------------------------------------------------
     */

    /// Move a file's modification time forward, the way an edit does.
    fn edited_now(path: &std::path::Path) {
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(30);

        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("opening the source to stamp it")
            .set_modified(later)
            .expect("setting the modification time");
    }

    #[tokio::test]
    async fn an_unchanged_file_reuses_the_plan_it_was_given() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let first = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        assert!(
            plan_is_current("t-1", &first, std::slice::from_ref(&file), 4).await,
            "nothing has touched the file"
        );

        let again = plan_reusing("t-1", std::slice::from_ref(&file), 4, Some(&first), None)
            .await
            .expect("replanning");

        assert_eq!(again, first, "an unchanged file keeps its digests");
    }

    #[tokio::test]
    async fn a_file_changed_after_planning_is_hashed_again() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let first = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        // Same length, different bytes, and a stamp that no longer matches.
        fs::write(&file.path, b"0123456789")
            .await
            .expect("rewriting the source");
        edited_now(&file.path);

        assert!(
            !plan_is_current("t-1", &first, std::slice::from_ref(&file), 4).await,
            "an edited file must not be described by the old plan"
        );

        let again = plan_reusing("t-1", std::slice::from_ref(&file), 4, Some(&first), None)
            .await
            .expect("replanning");

        assert_eq!(
            again.files[0].sha256,
            sha256_hex(b"0123456789"),
            "the new plan has to describe the bytes the file holds now"
        );
        assert_ne!(again, first);
    }

    #[tokio::test]
    async fn a_file_whose_stamp_is_unknown_is_hashed_again() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let first = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        // An older record, or a platform that reports no modification time.
        let mut unstamped = file.clone();
        unstamped.modified_nanos = None;

        assert!(
            !plan_is_current("t-1", &first, std::slice::from_ref(&unstamped), 4).await,
            "a missing stamp is not evidence that nothing changed"
        );
    }

    #[tokio::test]
    async fn a_plan_for_another_transfer_or_a_short_chunk_list_is_not_current() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let first = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        assert!(
            !plan_is_current("t-other", &first, std::slice::from_ref(&file), 4).await,
            "a plan names the transfer it belongs to"
        );

        let mut truncated = first.clone();
        truncated.files[0].chunks.pop();

        assert!(
            !plan_is_current("t-1", &truncated, std::slice::from_ref(&file), 4).await,
            "a plan that no longer covers every chunk cannot describe the file"
        );
    }

    #[tokio::test]
    async fn an_unchanged_empty_file_reuses_its_one_empty_chunk() {
        let scratch = TempDir::new();
        let file = source(&scratch, "empty.bin", 0).await;

        let first = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        assert!(plan_is_current("t-1", &first, std::slice::from_ref(&file), 4).await);

        let again = plan_reusing("t-1", std::slice::from_ref(&file), 4, Some(&first), None)
            .await
            .expect("replanning");

        assert_eq!(again, first);
        assert_eq!(again.files[0].chunks.len(), 1);
        assert_eq!(again.files[0].chunks[0].sha256, sha256_hex(b""));
    }

    #[tokio::test]
    async fn planning_several_files_keeps_their_order_and_digests() {
        let scratch = TempDir::new();
        let files = vec![
            source(&scratch, "a.bin", 5).await,
            source(&scratch, "b.bin", 9).await,
            source(&scratch, "empty.bin", 0).await,
        ];

        let expected = plan("t-1", &files, 4).await.expect("planning");

        assert_eq!(expected.files.len(), files.len());

        for (planned, file) in expected.files.iter().zip(&files) {
            assert_eq!(planned.file_id, file.id, "input order is kept");
            assert_eq!(
                planned.sha256,
                sha256_hex(&fs::read(&file.path).await.expect("reading the source"))
            );
        }
    }

    #[tokio::test]
    async fn planning_a_file_that_is_shorter_than_it_claims_fails() {
        let scratch = TempDir::new();
        let mut file = source(&scratch, "short.bin", 4).await;

        // The scan said ten bytes; the file has four.
        file.size = 10;

        assert!(
            plan("t-1", std::slice::from_ref(&file), 4).await.is_err(),
            "a plan must not describe bytes the sender does not have"
        );
    }
}
