//! The durable chunk-write core, shared by both protocols.
//!
//! # Why this is its own module
//!
//! Writing a chunk durably is the receiver's one irreversible act, and it has a
//! precise shape: the `.part` bytes must be on disk *before* the manifest entry
//! that describes them, or a crash leaves the receiver claiming a chunk it
//! cannot produce. That ordering is a correctness property, and it must exist
//! once.
//!
//! Before this module it existed inside the `/transfer/chunk` axum handler,
//! which meant the framed v2 session either had to duplicate it — two
//! implementations of a durability invariant, which drift — or could not write
//! chunks at all. Both callers now come through [`accept`] and [`finish`].
//!
//! # Locking
//!
//! The receiver used to serialise every chunk of every transfer through one
//! process-wide `tokio::sync::Mutex`. That is survivable when each request is
//! short and independent, and it is not survivable for a long-lived stream: a
//! session that stalls holding it blocks every other transfer on the device.
//!
//! So the lock is **per transfer** ([`ReceiverState::transfer_lock`]), acquired
//! only for that transfer's own manifest read-modify-write, and **never held
//! across a network await**. Callers send their acknowledgement *after*
//! [`accept`] returns.
//!
//! # What the caller owns
//!
//! Everything protocol-shaped: parsing a request or a frame, decrypting,
//! validating headers, and mapping an outcome onto a status code or a frame.
//! This module sees plaintext and a chunk's identity, and nothing else.

use std::path::Path;
use std::sync::atomic::Ordering;

use vilsend_core::DomainEvent;

use crate::models::{file_info::modified_nanos, progress, transfer::TransferStatus};
use crate::transfer::{
    manager::DESCRIBE_DIGESTS,
    manifest,
    state::DownloadState,
    writer::{assemble_and_publish, Assembly, ReceiverState},
};

/// What happened to a chunk that was handed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accepted {
    /// The bytes and their manifest entry are durably in place.
    Stored,
    /// The receiver already had this exact chunk, verified against its own
    /// record. Nothing was written, and nothing needed to be.
    AlreadyPresent,
}

/// A failure, classified so a caller can map it to its own protocol's error
/// rather than parsing a message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SinkError {
    /// The transfer is cancelled.
    #[error("transfer {0} was cancelled")]
    Cancelled(String),

    /// The receiver could not write or record the chunk.
    ///
    /// Distinct from [`SinkError::Refused`] because it says nothing about the
    /// chunk: the same bytes may succeed on a retry.
    #[error("{0}")]
    Storage(String),
}

/// Why a transfer's assembly was refused.
///
/// A v1 sender distinguishes these by status code — `409` for pruned chunks,
/// `422` for an integrity mismatch — so collapsing them into one would change
/// what an installed client is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Chunks the receiver already held no longer match their record and were
    /// discarded.
    Pruned,
    /// Every chunk was present, and the file they assemble into is still not the
    /// one the sender described.
    IntegrityMismatch,
}

/// How a transfer's assembly ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Completion {
    /// Not every chunk is recorded yet. The common case.
    Incomplete,
    /// The file was merged, verified and published.
    Published,
    /// Every chunk was present but the file they assemble into is not the one
    /// the sender described. The offending chunks were dropped.
    Refused {
        kind: Refusal,
        lost: Vec<u64>,
        message: String,
    },
}

/// Report a storage fault and build the error for it.
///
/// Moved here from the v1 handler so that the side effects a failed chunk write
/// has travel with the write itself: the download is marked failed and the
/// shell is told. A second protocol that wrote chunks without this would leave
/// the frontend showing a transfer that is still running.
fn storage_failure(
    state: &ReceiverState,
    download: &DownloadState,
    transfer_id: &str,
    chunk_index: Option<u64>,
    message: String,
) -> SinkError {
    *download.status.lock().expect("download status poisoned") = TransferStatus::Failed;

    state.events.emit(DomainEvent::TransferFailed {
        progress: progress::make_download(download),
    });

    tracing::error!(transfer_id, chunk = ?chunk_index, "{message}");

    SinkError::Storage(message)
}

/// Write one chunk durably and record it.
///
/// The order is the invariant: `write_part` (fsync + rename), *then* the
/// manifest entry, *then* `store` (fsync + rename). A crash between any two
/// leaves a chunk the receiver does not count — which costs a resend — never a
/// counted chunk it cannot produce.
pub(crate) async fn accept(
    state: &ReceiverState,
    download: &DownloadState,
    transfer_id: &str,
    file_id: &str,
    chunk_index: u64,
    plaintext: Vec<u8>,
) -> Result<Accepted, SinkError> {
    /*
     * Cancellation is checked before any lock is taken, and the pause wait
     * before it. Neither belongs under a lock: a paused transfer has nothing to
     * do, and holding a lock while it waits is how one transfer's pause becomes
     * another transfer's stall.
     */
    wait_out_a_pause(download, transfer_id).await?;

    let accepted_started = std::time::Instant::now();

    download
        .report
        .chunk_started(file_id, plaintext.len() as u64);

    let transfer_dir = state.transfer_dir(transfer_id);
    let parts = transfer_dir.join(file_id);

    tokio::fs::create_dir_all(&parts).await.map_err(|e| {
        storage_failure(
            state,
            download,
            transfer_id,
            Some(chunk_index),
            format!("could not create the chunk directory: {e}"),
        )
    })?;

    let part = manifest::part_path(&parts, chunk_index);

    /*
     * Hashed on the blocking pool, and *before* the transfer lock: a chunk is a
     * few milliseconds of pure CPU, and holding the lock across it would make
     * every other chunk of the transfer wait for a hash it does not depend on.
     * The buffer is moved in and back out rather than copied.
     */
    let chunk_length = plaintext.len() as u64;

    let hash_started = std::time::Instant::now();

    let (plaintext, chunk_hash) = tokio::task::spawn_blocking(move || {
        let digest = manifest::sha256_hex(&plaintext);

        (plaintext, digest)
    })
    .await
    .map_err(|error| {
        storage_failure(
            state,
            download,
            transfer_id,
            Some(chunk_index),
            format!("the chunk hashing task failed: {error}"),
        )
    })?;

    let hash = hash_started.elapsed();

    let lock_started = std::time::Instant::now();

    let lock = state.transfer_lock(transfer_id).await;
    let _held = lock.lock().await;

    /*
     * The manifest is read from disk the first time a transfer is touched after
     * a restart, so the receiver's knowledge of what it holds survives the
     * process. Only the handle is taken under the map lock; the record itself
     * is locked for this transfer alone, so one transfer's disk work never
     * holds up another's.
     */
    let handle = state.manifest_handle(transfer_id).await;
    let mut record = handle.lock().await;

    let lock_wait = lock_started.elapsed();

    /*
     * A chunk may only be skipped when the manifest has an entry for it *and*
     * the bytes on disk still match that entry. A `.part` with no entry — left
     * by an older build, or by a crash before the manifest was committed — is
     * not evidence of anything and gets written again.
     */
    let already_complete = match record.entry(file_id, chunk_index) {
        Some(entry) => {
            entry.chunk_length == chunk_length
                && entry.chunk_hash == chunk_hash
                && manifest::part_matches(&part, entry).await
        }

        None => false,
    };

    if already_complete {
        tracing::debug!(
            transfer_id,
            chunk = chunk_index,
            "chunk already present and matches the manifest"
        );

        return Ok(Accepted::AlreadyPresent);
    }

    /*
     * Whatever is on disk under this name is not this chunk: there was no entry
     * for it, or the entry disagreed with what arrived, or the bytes did not
     * hash to what the entry recorded. Remove it before writing the
     * replacement, so a failure part-way through leaves nothing behind that a
     * later run could read as progress.
     */
    manifest::discard_part(&part).await.map_err(|e| {
        storage_failure(
            state,
            download,
            transfer_id,
            Some(chunk_index),
            format!("could not discard an invalid chunk: {e}"),
        )
    })?;

    /*
     * `write_part` writes a temporary and renames it into place once its bytes
     * are flushed and synced, so a crash mid-write cannot leave a short `.part`
     * under the real name — and when it returns, the bytes are durably on disk.
     */
    let write_started = std::time::Instant::now();

    manifest::write_part(&part, &plaintext).await.map_err(|e| {
        storage_failure(
            state,
            download,
            transfer_id,
            Some(chunk_index),
            format!("could not write the chunk: {e}"),
        )
    })?;

    let part_write = write_started.elapsed();

    /*
     * The stamp the written file now carries. Recording it is what lets a chunk
     * the sender offers again be recognised as already held without reading it
     * back — see `manifest::part_matches`.
     */
    let written_nanos = tokio::fs::metadata(&part)
        .await
        .ok()
        .as_ref()
        .and_then(modified_nanos);

    let entry = manifest::ChunkEntry {
        file_id: file_id.to_owned(),
        chunk_index,
        chunk_length,
        chunk_hash,
        written_nanos,
    };

    /*
     * Record the chunk only now, after its bytes are durably in place, so there
     * is no ordering in which a crash leaves a record for a chunk the receiver
     * cannot produce.
     *
     * The journal line is what makes the record durable, and it is one append
     * rather than a rewrite of every entry the transfer holds. It is folded
     * into the manifest when it grows large enough to be worth replaying in
     * bulk.
     */
    record.record(entry.clone());

    let journal_started = std::time::Instant::now();

    let journal_length = manifest::journal(&transfer_dir, &entry)
        .await
        .map_err(|e| {
            storage_failure(
                state,
                download,
                transfer_id,
                Some(chunk_index),
                format!("could not journal the chunk: {e}"),
            )
        })?;

    let journal = journal_started.elapsed();

    if manifest::journal_is_full(journal_length) {
        manifest::store(&transfer_dir, &record).await.map_err(|e| {
            storage_failure(
                state,
                download,
                transfer_id,
                Some(chunk_index),
                format!("could not compact the chunk journal: {e}"),
            )
        })?;
    }

    drop(record);

    // Progress moves only after the chunk is written and recorded — the same
    // rule as everything above, one level up: the counters describe bytes the
    // receiver can actually produce.
    let accepted_for = accepted_started.elapsed();

    download.report.chunk_finished(file_id, accepted_for, 0);
    download.report.add_persistence(accepted_for);

    download
        .received_bytes
        .fetch_add(chunk_length, Ordering::Relaxed);

    download.received_chunks.fetch_add(1, Ordering::Relaxed);

    *download.status.lock().expect("download status poisoned") = TransferStatus::Downloading;

    state.events.emit(DomainEvent::TransferProgress {
        progress: progress::make_download(download),
    });

    tracing::debug!(
        transfer_id,
        chunk = chunk_index,
        file = %file_id,
        lock_wait_ms = lock_wait.as_secs_f64() * 1000.0,
        hash_ms = hash.as_secs_f64() * 1000.0,
        part_write_ms = part_write.as_secs_f64() * 1000.0,
        journal_ms = journal.as_secs_f64() * 1000.0,
        total_ms = accepted_for.as_secs_f64() * 1000.0,
        "chunk persisted"
    );

    tracing::info!(
        transfer_id,
        chunk = chunk_index,
        path = %part.display(),
        "chunk saved"
    );

    Ok(Accepted::Stored)
}

/// Assemble the file once every chunk of it is recorded.
///
/// Two passes, deliberately: the cheap one asks the manifest for an entry per
/// index and reads nothing from disk, and only if it passes does the expensive
/// one re-hash every chunk. Doing the full check per arriving chunk would make a
/// transfer quadratic in its own size.
pub(crate) async fn finish(
    state: &ReceiverState,
    download: &DownloadState,
    transfer_id: &str,
    file_id: &str,
    total_chunks: u64,
    relative_path: &str,
) -> Result<Completion, SinkError> {
    let lock = state.transfer_lock(transfer_id).await;
    let _held = lock.lock().await;

    /*
     * A file publishes once, and the check is inside the lock on purpose:
     * `finish` runs after every chunk, so another call can be waiting here
     * while the publishing call sets `Completed` and removes the parts. Checking
     * before the lock would let that waiter through to find every part gone and
     * prune a file that was just delivered.
     */
    if matches!(
        *download.status.lock().expect("download status poisoned"),
        TransferStatus::Completed
    ) {
        return Ok(Completion::Published);
    }

    let transfer_dir = state.transfer_dir(transfer_id);
    let parts = transfer_dir.join(file_id);

    let handle = state.manifest_handle(transfer_id).await;

    /*
     * Held for the whole read-modify-write of this transfer's record, and for
     * the merge. It is this transfer's lock, not the map's: another transfer's
     * completion — or its chunk writes — never waits on this.
     */
    let mut record = handle.lock().await;

    if !manifest::all_chunks_recorded(&record, file_id, total_chunks) {
        return Ok(Completion::Incomplete);
    }

    /*
     * The full check reads and hashes every chunk before the merge reads every
     * chunk again. That second read is redundant when the sender described the
     * file: the merge already digests the file it assembles and refuses to
     * publish one that does not match, and `discard_untrusted` — reached on
     * that mismatch — is what says which chunks have to be sent again.
     *
     * The cheap presence check keeps the old behaviour for the one case a
     * digest cannot express: a `.part` that is *gone*. Merging an absent chunk
     * is an I/O error, whereas the full check prunes it and answers `409` so
     * the sender resends it. That is rare, so it is allowed to pay for the
     * full pass; the ordinary completion is not.
     */
    let verification_started = std::time::Instant::now();

    let verified_in_full = !(sender_speaks_for_the_whole_file(&record, file_id, total_chunks)
        && all_parts_present(&parts, total_chunks).await);

    let verification = if verified_in_full {
        manifest::verify_and_prune(&mut record, &parts, file_id, total_chunks).await
    } else {
        manifest::Verification {
            complete: true,
            pruned: false,
        }
    };

    {
        let mut phases = download.phases.lock().expect("phases lock poisoned");

        phases.verification_seconds = Some(verification_started.elapsed().as_secs_f64());
    }

    tracing::info!(
        transfer_id,
        file_id,
        total_chunks,
        verified_in_full,
        elapsed_ms = verification_started.elapsed().as_millis() as u64,
        "completion check finished"
    );

    if verification.pruned {
        /*
         * Chunks that did not match their record have been removed from disk and
         * from the manifest. Persist that, or a restart would reload the old
         * entries and skip bytes we know are wrong.
         */
        manifest::store(&transfer_dir, &record).await.map_err(|e| {
            storage_failure(
                state,
                download,
                transfer_id,
                None,
                format!("could not write the pruned manifest: {e}"),
            )
        })?;

        let lost = (0..total_chunks)
            .filter(|index| record.entry(file_id, *index).is_none())
            .collect::<Vec<_>>();

        /*
         * Deliberately *not* latched as refused. The response names exactly the
         * chunks that were dropped, and the sender resends those in this same
         * session — latching would answer every one of those resends with
         * "open a session", turning a recoverable chunk loss into a failed
         * transfer. The latch is still used for an integrity mismatch, where
         * the sender has to be told the file cannot be assembled as it stands.
         */

        tracing::warn!(
            transfer_id,
            file_id,
            ?lost,
            "chunks no longer match the manifest and were discarded; they have to be sent again"
        );

        let message = format!(
            "chunks {lost:?} of {file_id} no longer match what was recorded for them \
             and were discarded; they must be sent again"
        );

        return Ok(Completion::Refused {
            kind: Refusal::Pruned,
            lost,
            message,
        });
    }

    if !verification.complete {
        return Ok(Completion::Incomplete);
    }

    let downloads = state.host.downloads_root().map_err(|e| {
        storage_failure(
            state,
            download,
            transfer_id,
            None,
            format!("no download root: {e}"),
        )
    })?;

    let destination = downloads.join(relative_path);

    tracing::info!(transfer_id, total_chunks, "all chunks received");

    let merge_started = std::time::Instant::now();

    let outcome = match assemble_and_publish(
        &mut record,
        &transfer_dir,
        file_id,
        total_chunks,
        &destination,
    )
    .await
    {
        Ok(Assembly::Published) => {
            // Whatever was refused before, the file is delivered now.
            state.refused.lock().await.remove(transfer_id);

            *download.status.lock().expect("download status poisoned") = TransferStatus::Completed;

            {
                let mut phases = download.phases.lock().expect("phases lock poisoned");

                phases.final_assembly_seconds = Some(merge_started.elapsed().as_secs_f64());
            }

            download.report.file_completed(file_id);

            finalize_download(state, download, TransferStatus::Completed, None, None, None).await;

            state.events.emit(DomainEvent::TransferCompleted {
                progress: progress::make_download(download),
            });

            /*
             * The completed state stays in the map until the next sweep, so a
             * chunk still in flight when the file published is answered "already
             * delivered" (200) instead of "no such transfer" (404). Publishing
             * is triggered by whichever chunk's request completes the file, and
             * the sender's other in-flight requests are not that one.
             *
             * It is small and bounded by the sweep interval: `run_cleanup`
             * drops finished states on its next pass.
             */

            // Likewise its write lock: the transfer is delivered and has no next
            // chunk.
            state.forget_transfer(transfer_id).await;

            tracing::info!(
                transfer_id,
                destination = %destination.display(),
                "merge complete"
            );

            /*
             * A failure removing the parts is logged and swallowed rather than
             * reported: the file is already published and verified, so the
             * transfer has succeeded. Turning a leftover temporary into a failed
             * transfer would be a worse answer than a stale directory, and the
             * TTL sweep collects it.
             */
            if let Err(error) = tokio::fs::remove_dir_all(&parts).await {
                tracing::warn!(transfer_id, %error, "failed removing temp chunks");
            }

            /*
             * The parts are gone, so the manifest that described them is no
             * longer worth caching. It stays on disk — a transfer that restarts
             * under the same id will load it, find no backing `.part` files, and
             * re-write the chunks, which is the safe answer.
             */
            state.forget_manifest(transfer_id).await;

            Ok(Completion::Published)
        }

        /*
         * The chunks were all here and none of them could be shown to be wrong on
         * their own, and the file they assemble into is still not the one the
         * sender described. Nothing is marked complete and the file is not
         * delivered; the chunks behind the mismatch have already been dropped and
         * written down, so a resume asks for exactly those.
         */
        Ok(Assembly::Rejected { chunks }) => {
            tracing::error!(
                transfer_id,
                file_id,
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

            // Nothing else may be acknowledged for this transfer until the sender
            // has been told.
            state.refused.lock().await.insert(transfer_id.to_owned());

            {
                let mut phases = download.phases.lock().expect("phases lock poisoned");

                phases.final_assembly_seconds = Some(merge_started.elapsed().as_secs_f64());
            }

            finalize_download(
                state,
                download,
                TransferStatus::Failed,
                Some("assembly"),
                Some(format!(
                    "the assembled file did not match the sender's digests: chunks {chunks:?}"
                )),
                None,
            )
            .await;

            Ok(Completion::Refused {
                kind: Refusal::IntegrityMismatch,
                lost: chunks.clone(),
                message: format!(
                    "integrity check failed for {file_id}: chunks {chunks:?} do not match \
                     the sender's digests and must be sent again"
                ),
            })
        }

        Err(error) => Err(storage_failure(
            state,
            download,
            transfer_id,
            None,
            format!("merge failed: {error}"),
        )),
    };

    tracing::info!(
        transfer_id,
        file_id,
        elapsed_ms = merge_started.elapsed().as_millis() as u64,
        "file merge finished"
    );

    outcome
}

/// Whether the sender described the file well enough for the merge itself to
/// be the check.
///
/// Requires both a whole-file digest and a per-chunk digest for every chunk, so
/// that a mismatch can still name the chunks to resend. A sender that only
/// describes whole files leaves this `false` and gets the full check — the
/// merge could say the file is wrong but not which chunk to ask for.
fn sender_speaks_for_the_whole_file(
    record: &manifest::Manifest,
    file_id: &str,
    total: u64,
) -> bool {
    total > 0
        && record
            .expected
            .as_ref()
            .and_then(|expected| expected.file(file_id))
            .map(|file| !file.sha256.is_empty() && file.chunks.len() as u64 == total)
            .unwrap_or(false)
}

/// Whether every chunk file is on disk, by metadata only.
///
/// The bytes are not read: a chunk that is present but wrong is caught by the
/// merged file's own digest, which the merge computes anyway. A chunk that is
/// absent cannot be merged at all, and is what takes the full pruning path so
/// the sender is asked for it rather than shown an I/O failure.
async fn all_parts_present(parts: &Path, total: u64) -> bool {
    for index in 0..total {
        if tokio::fs::metadata(manifest::part_path(parts, index))
            .await
            .is_err()
        {
            return false;
        }
    }

    true
}

/// Wait while a download is paused, and refuse a cancelled one.
///
/// **The pause wait is not reachable today**: nothing in the receiver ever sets
/// `DownloadState::paused`, so this returns immediately. It is kept, and it is
/// checked *before* any lock is taken, because the previous arrangement held the
/// process-wide guard across an unbounded `while paused` loop that only checked
/// cancellation after it exited — a transfer paused and then cancelled would
/// have looped forever holding a lock every other transfer needed. Recorded as
/// blocker B in `docs/SECURITY.md`.
async fn wait_out_a_pause(download: &DownloadState, transfer_id: &str) -> Result<(), SinkError> {
    while download.paused.load(Ordering::Relaxed) {
        if download.cancelled.load(Ordering::Relaxed) {
            return Err(SinkError::Cancelled(transfer_id.to_owned()));
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    if download.cancelled.load(Ordering::Relaxed) {
        return Err(SinkError::Cancelled(transfer_id.to_owned()));
    }

    Ok(())
}

/// Whether `relative_path` is a destination the receiver will accept.
///
/// A path that is absolute, or that climbs out with `..`, would let a sender
/// write anywhere the process can reach. Kept here beside the write it protects
/// rather than in each protocol's request parser, so both protocols are held to
/// the same rule — a v2 session that skipped it would be a hole the v1 handler
/// does not have.
pub(crate) fn is_contained(relative_path: &str) -> bool {
    let path = Path::new(relative_path);

    /*
     * `is_absolute` is not enough, and the gap is platform-specific rather than
     * hypothetical: on Windows `Path::new("/etc/passwd").is_absolute()` is
     * **false**, because a Windows absolute path needs a drive letter or a UNC
     * prefix. The path is merely *rooted*, which still means "from the top of
     * something the sender did not choose".
     *
     * So the check is on components, not on one predicate: any `RootDir`,
     * `Prefix` or `ParentDir` component means the path is not the
     * contained-relative-path this receiver requires. `docs/FILE_TRANSFER.md`
     * describes the intent as rejecting "absolute paths and `..`"; this is that
     * intent, made true on both platforms.
     */
    path.components().all(|component| {
        matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) && !relative_path.is_empty()
}

/// Write the receiver's report for this transfer, once.
///
/// The receiver has no transfer-wide file plan — each file arrives on its own —
/// so the report describes what it actually handled and says so.
pub(crate) async fn finalize_download(
    state: &ReceiverState,
    download: &DownloadState,
    status: TransferStatus,
    failure_stage: Option<&str>,
    failure_reason: Option<String>,
    cancellation_reason: Option<String>,
) {
    let finish = crate::transfer::report::Finish {
        transfer_id: download.transfer_id.clone(),
        status,
        total_bytes: download.total_bytes,
        completed_bytes: download.received_bytes.load(Ordering::Relaxed),
        total_chunks: 0,
        completed_chunks: download.received_chunks.load(Ordering::Relaxed),
        files: download.report.file_descriptors(),
        retries: 0,
        failure_stage: failure_stage.map(str::to_owned),
        failure_reason,
        cancellation_reason,
        phases: download
            .phases
            .lock()
            .expect("phases lock poisoned")
            .clone(),
        unavailable: vec![
            "total_chunks (the receiver has no transfer-wide chunk plan)".into(),
            "phases.handshake_seconds (the start request is the handshake)".into(),
            "phases.publication_seconds (the rename is inside assembly)".into(),
            "per-file relative paths (the receiver reports by file id)".into(),
            "resources are process-wide; concurrent transfers share them".into(),
        ],
        recovered: false,
    };

    download
        .report
        .finalize(
            &state.root.join("reports"),
            finish,
            crate::transfer::session::COMPRESSION_ENABLED,
            DESCRIBE_DIGESTS,
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contained_path_is_accepted() {
        assert!(is_contained("file.bin"));
        assert!(is_contained("nested/destination.bin"));
        assert!(is_contained("a/b/c/d.bin"));
    }

    #[test]
    fn a_path_that_escapes_is_refused() {
        assert!(!is_contained("../escape.bin"));
        assert!(!is_contained("nested/../../escape.bin"));
        assert!(!is_contained("/etc/passwd"));
        assert!(!is_contained("C:\\Windows\\system32\\evil.dll"));
    }

    /// The two refusals answer the v1 wire with different status codes — `409`
    /// for pruned chunks, `422` for an integrity mismatch — so they must stay
    /// distinguishable. Collapsing them would change what an installed client is
    /// told, which is the one thing this refactor may not do.
    #[test]
    fn refusals_and_storage_faults_are_distinguishable() {
        let pruned = Refusal::Pruned;
        let mismatch = Refusal::IntegrityMismatch;

        assert_ne!(pruned, mismatch);

        let storage = SinkError::Storage("disk full".into());

        assert!(matches!(storage, SinkError::Storage(_)));
        assert!(!matches!(storage, SinkError::Cancelled(_)));
    }

    struct TestHost;

    impl crate::transfer::writer::ReceiverHost for TestHost {
        fn downloads_root(&self) -> std::result::Result<std::path::PathBuf, String> {
            Ok(std::env::temp_dir())
        }

        fn available_space(&self, _path: &std::path::Path) -> std::result::Result<u64, String> {
            Ok(u64::MAX)
        }

        fn device_private_key(&self) -> std::result::Result<String, String> {
            Ok(String::new())
        }

        fn device_public_key(&self) -> std::result::Result<String, String> {
            Ok(String::new())
        }
    }

    fn download(transfer_id: &str) -> DownloadState {
        DownloadState {
            transfer_id: transfer_id.to_owned(),
            total_bytes: 0,
            received_bytes: std::sync::atomic::AtomicU64::new(0),
            received_chunks: std::sync::atomic::AtomicU64::new(0),
            paused: std::sync::atomic::AtomicBool::new(false),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            started_at: std::time::Instant::now(),
            status: std::sync::Mutex::new(TransferStatus::Pending),
            report: crate::transfer::report::Telemetry::new(
                crate::transfer::report::Role::Receiver,
            ),
            phases: std::sync::Mutex::new(crate::transfer::report::PhaseTimings::default()),
        }
    }

    /// Two transfers writing at once each reach their own record.
    ///
    /// The global map is only ever taken to clone a transfer's handle — the
    /// disk work happens under that transfer's own lock. This pins the
    /// independence and, more importantly, that there is no lock ordering that
    /// deadlocks when two transfers are in flight together.
    #[tokio::test]
    async fn two_transfers_write_their_own_records_concurrently() {
        let root = std::env::temp_dir().join(format!(
            "vilsend-sink-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is after 1970")
                .as_nanos()
        ));

        let state = ReceiverState::new(
            std::sync::Arc::new(TestHost),
            std::sync::Arc::new(vilsend_core::RecordingEventSink::new()),
            root.clone(),
        );

        let first = download("t-a");
        let second = download("t-b");

        let (a, b) = tokio::join!(
            accept(&state, &first, "t-a", "f-1", 0, b"first".to_vec()),
            accept(&state, &second, "t-b", "f-1", 0, b"second".to_vec()),
        );

        assert!(matches!(a, Ok(Accepted::Stored)));
        assert!(matches!(b, Ok(Accepted::Stored)));

        let a = state.manifest_handle("t-a").await.lock().await.clone();
        let b = state.manifest_handle("t-b").await.lock().await.clone();

        assert_eq!(a.chunks.len(), 1);
        assert_eq!(b.chunks.len(), 1);
        assert_ne!(
            a.chunks[0].chunk_hash, b.chunks[0].chunk_hash,
            "each transfer recorded its own bytes"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
