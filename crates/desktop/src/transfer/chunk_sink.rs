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
    manifest, network_report,
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

/// What the durability step cost, returned to the caller so a v1 receiver can
/// put it on the response and the sender can attribute its `http` time.
///
/// Measured from locals [`accept`] already computes; no extra clock reads.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SinkTimings {
    pub lock_wait: std::time::Duration,
    pub part_write: std::time::Duration,
    pub journal: std::time::Duration,
    pub accepted_for: std::time::Duration,
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
    total_chunks: u64,
    plaintext: Vec<u8>,
) -> Result<(Accepted, SinkTimings), SinkError> {
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

    let lock = state.transfer_lock(transfer_id).await;
    let key = (transfer_id.to_owned(), file_id.to_owned(), chunk_index);
    let assembly = manifest::assembly_path(&parts);

    /*
     * Phase 1, under the transfer lock: claim this chunk, or wait for the
     * attempt that already holds it. The lock covers the manifest check and the
     * reservation only; the part write and the journal flush happen outside it,
     * which is what lets distinct chunks of one transfer proceed concurrently.
     *
     * The in-flight check comes first: a write that has recorded its entry in
     * memory but not yet fsynced it must not be acknowledged by a duplicate.
     */
    let lock_wait = loop {
        let started = std::time::Instant::now();
        let held = lock.lock().await;
        let lock_wait = started.elapsed();

        if let Some(waited) = state.reserve_chunk(&key).await {
            drop(held);

            // Ok means it is durable now — the next pass returns
            // AlreadyPresent. Err means it failed, and this attempt takes over.
            let _ = waited.await;

            continue;
        }

        /*
         * A chunk may only be skipped when the manifest has an entry for it
         * *and* the bytes on disk still match that entry. A `.part` with no
         * entry — left by an older build, or by a crash before the manifest was
         * committed — is not evidence of anything and gets written again.
         */
        let (candidate, placement) = {
            let handle = state.manifest_handle(transfer_id).await;
            let mut record = handle.lock().await;

            /*
             * Where this chunk's bytes belong is decided per file, from the
             * sender's own layout: `offset = index * chunk_size`. The manifest
             * remembers the decision so two chunks of one file cannot disagree.
             * See `manifest::Manifest::place`.
             */
            let placement = record.place(file_id, chunk_index, total_chunks, chunk_length);

            let started = std::time::Instant::now();
            let entry = record.entry(file_id, chunk_index);
            state.record_manifest_lookup(transfer_id, started.elapsed());

            let candidate = match entry {
                Some(entry)
                    if entry.chunk_length == chunk_length && entry.chunk_hash == chunk_hash =>
                {
                    Some(entry.clone())
                }

                _ => None,
            };

            (candidate, placement)
        };

        /*
         * The bytes are read and hashed with no manifest lock held: the entry is
         * a snapshot, and nothing can delete this chunk meanwhile because this
         * attempt holds its in-flight reservation.
         */
        let already_complete = match (&candidate, placement) {
            (Some(entry), manifest::Placement::Direct(offset)) => {
                manifest::chunk_matches(&assembly, offset, entry).await
            }

            (Some(entry), _) => manifest::part_matches(&part, entry).await,

            (None, _) => false,
        };

        if already_complete {
            drop(held);

            state.release_chunk(&key, Ok(())).await;

            tracing::debug!(
                transfer_id,
                chunk = chunk_index,
                "chunk already present and matches the manifest"
            );

            return Ok((
                Accepted::AlreadyPresent,
                SinkTimings {
                    lock_wait,
                    accepted_for: accepted_started.elapsed(),
                    ..SinkTimings::default()
                },
            ));
        }

        drop(held);

        break (lock_wait, placement);
    };

    let (lock_wait, placement) = lock_wait;

    let outcome = store_chunk(
        state,
        download,
        transfer_id,
        file_id,
        chunk_index,
        total_chunks,
        &part,
        &assembly,
        placement,
        accepted_started,
        lock_wait,
        hash,
        plaintext,
        chunk_length,
        chunk_hash,
    )
    .await;

    let signal = outcome
        .as_ref()
        .map(|_| ())
        .map_err(|error| error.to_string());

    state.release_chunk(&key, signal).await;

    outcome
}

/// Phases 2 and 3 of [`accept`]: write the part outside the transfer lock, then
/// re-enter only to record and journal it.
///
/// The journal entry is recorded in memory before the flush and taken back out
/// if the flush fails, so a retry cannot be answered from a record that is not
/// on disk.
#[allow(clippy::too_many_arguments)]
async fn store_chunk(
    state: &ReceiverState,
    download: &DownloadState,
    transfer_id: &str,
    file_id: &str,
    chunk_index: u64,
    total_chunks: u64,
    part: &Path,
    assembly: &Path,
    placement: manifest::Placement,
    accepted_started: std::time::Instant,
    lock_wait: std::time::Duration,
    hash: std::time::Duration,
    plaintext: Vec<u8>,
    chunk_length: u64,
    chunk_hash: String,
) -> Result<(Accepted, SinkTimings), SinkError> {
    /*
     * Whatever is on disk under this name is not this chunk: there was no entry
     * for it, or the entry disagreed with what arrived, or the bytes did not
     * hash to what the entry recorded. Remove it before writing the
     * replacement, so a failure part-way through leaves nothing behind that a
     * later run could read as progress.
     *
     * No transfer lock is held, and the in-flight reservation from phase 1
     * guarantees no other attempt is writing this same chunk.
     */
    let write_started = std::time::Instant::now();

    /*
     * The stamp the written file now carries. Recording it is what lets a chunk
     * the sender offers again be recognised as already held without reading it
     * back — see `manifest::part_matches`.
     *
     * Only the `.part` layout has a per-chunk stamp. In the direct layout every
     * chunk shares one file whose modification time moves with each write, so a
     * range is always read back and hashed instead.
     */
    let (offset, written_nanos) = match placement {
        manifest::Placement::Direct(offset) => {
            manifest::write_chunk_at(assembly, offset, plaintext)
                .await
                .map_err(|e| {
                    storage_failure(
                        state,
                        download,
                        transfer_id,
                        Some(chunk_index),
                        format!("could not write the chunk: {e}"),
                    )
                })?;

            (Some(offset), None)
        }

        /*
         * The sender's layout does not fit this chunk — a length that disagrees
         * with the transfer's chunk size, or a tail longer than one chunk. It
         * would be written over a neighbour's range, so it is refused instead.
         */
        manifest::Placement::Invalid => {
            return Err(storage_failure(
                state,
                download,
                transfer_id,
                Some(chunk_index),
                format!("chunk {chunk_index} does not fit the sender's chunk layout"),
            ));
        }

        manifest::Placement::Parts => {
            /*
             * `write_part` writes a temporary and renames it into place once its
             * bytes are flushed and synced, so a crash mid-write cannot leave a
             * short `.part` under the real name — and when it returns, the bytes
             * are durably on disk.
             */
            manifest::discard_part(part).await.map_err(|e| {
                storage_failure(
                    state,
                    download,
                    transfer_id,
                    Some(chunk_index),
                    format!("could not discard an invalid chunk: {e}"),
                )
            })?;

            manifest::write_part(part, &plaintext).await.map_err(|e| {
                storage_failure(
                    state,
                    download,
                    transfer_id,
                    Some(chunk_index),
                    format!("could not write the chunk: {e}"),
                )
            })?;

            (
                None,
                tokio::fs::metadata(part)
                    .await
                    .ok()
                    .as_ref()
                    .and_then(modified_nanos),
            )
        }
    };

    let part_write = write_started.elapsed();

    let entry = manifest::ChunkEntry {
        file_id: file_id.to_owned(),
        chunk_index,
        chunk_length,
        chunk_hash,
        offset,
        written_nanos,
    };

    /*
     * Record the chunk in memory, then journal it. The in-memory record must
     * be visible to a concurrent compaction before the journal is flushed —
     * `manifest::store` takes the same record lock, and only entries already in
     * it are written into the manifest. If the flush fails the entry is taken
     * back out, so a retry re-writes rather than being answered from memory.
     */
    {
        let handle = state.manifest_handle(transfer_id).await;
        let mut record = handle.lock().await;

        /*
         * Reserve once for the whole file, from the chunk count the sender
         * stated, so the map does not reallocate its way through the transfer.
         * A no-op after the first chunk.
         */
        record.reserve(total_chunks);

        let started = std::time::Instant::now();
        record.record(entry.clone());
        state.record_manifest_lookup(transfer_id, started.elapsed());
    }

    let journal = state
        .journal_for(transfer_id, std::sync::Arc::clone(&download.report))
        .await;

    let journal_started = std::time::Instant::now();

    let flushed = journal.append(&entry).await;

    let journal_wait = journal_started.elapsed();

    if let Err(error) = flushed {
        let handle = state.manifest_handle(transfer_id).await;

        handle.lock().await.forget(file_id, chunk_index);

        return Err(storage_failure(
            state,
            download,
            transfer_id,
            Some(chunk_index),
            format!("could not journal the chunk: {error}"),
        ));
    }

    /*
     * No compaction here: the journal *is* the durable record during the
     * transfer, and `finish` folds it into the manifest once when the transfer
     * ends. Rewriting the manifest per chunk meant an fsync of a document that
     * grows linearly with the transfer, under the transfer lock, which is what
     * every other chunk queued behind.
     */

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

    write_network_report(state, download, TransferStatus::Downloading, false).await;

    tracing::debug!(
        transfer_id,
        chunk = chunk_index,
        file = %file_id,
        lock_wait_ms = lock_wait.as_secs_f64() * 1000.0,
        hash_ms = hash.as_secs_f64() * 1000.0,
        part_write_ms = part_write.as_secs_f64() * 1000.0,
        journal_ms = journal_wait.as_secs_f64() * 1000.0,
        total_ms = accepted_for.as_secs_f64() * 1000.0,
        "chunk persisted"
    );

    tracing::info!(
        transfer_id,
        chunk = chunk_index,
        path = %part.display(),
        "chunk saved"
    );

    Ok((
        Accepted::Stored,
        SinkTimings {
            lock_wait,
            part_write,
            journal: journal_wait,
            accepted_for,
        },
    ))
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

    /*
     * A file received at its offsets is verified and published as ranges of one
     * assembly file; anything else is the `.part` layout.
     */
    let assembly = manifest::assembly_path(&parts);

    let handle = state.manifest_handle(transfer_id).await;

    /*
     * Held for the whole read-modify-write of this transfer's record, and for
     * the merge. It is this transfer's lock, not the map's: another transfer's
     * completion — or its chunk writes — never waits on this.
     */
    let record = handle.lock().await;

    let direct = record.file_uses_assembly(file_id);

    let lookup_started = std::time::Instant::now();
    let all_recorded = manifest::all_chunks_recorded(&record, file_id, total_chunks);
    state.record_manifest_lookup(transfer_id, lookup_started.elapsed());

    if !all_recorded {
        return Ok(Completion::Incomplete);
    }

    /*
     * The full check reads and hashes every chunk before the merge reads every
     * chunk again. That second read is redundant when the sender described the
     * file: the merge already digests the file it assembles and refuses to
     * publish one that does not match, and `discard_untrusted` — reached on
     * that mismatch — is what says which chunks have to be sent again.
     *
     * The verification reads disk, so it runs with the manifest lock released:
     * the transfer lock (held above) is what serialises publication, and chunk
     * writers only need the manifest mutex for their own one-entry update.
     */
    let speaks_for_file = sender_speaks_for_the_whole_file(&record, file_id, total_chunks);

    let expected_file = record
        .expected
        .as_ref()
        .and_then(|expected| expected.file(file_id))
        .cloned();

    drop(record);

    let verification_started = std::time::Instant::now();

    let everything_on_disk = if direct {
        /*
         * Without a described size there is nothing to compare the assembly
         * against, so the per-chunk check below is always run.
         */
        match expected_file.as_ref() {
            Some(expected) => manifest::assembly_covers(&assembly, expected.size).await,
            None => false,
        }
    } else {
        all_parts_present(&parts, total_chunks).await
    };

    let verified_in_full = !(speaks_for_file && everything_on_disk);

    let verification = if verified_in_full {
        let entries = {
            let record = handle.lock().await;

            record.file_entries(file_id, total_chunks)
        };

        if direct {
            manifest::verify_entries_direct(&assembly, total_chunks, &entries).await
        } else {
            manifest::verify_entries(&parts, file_id, total_chunks, &entries).await
        }
    } else {
        manifest::Verification {
            complete: true,
            pruned: false,
            prune: Vec::new(),
        }
    };

    let mut record = handle.lock().await;

    if verification.pruned {
        manifest::apply_prune(&mut record, file_id, &verification.prune);
    }

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
        state
            .store_manifest(transfer_id, &transfer_dir, &record, true)
            .await
            .map_err(|e| {
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

            /*
             * A published file is no longer progress the receiver still holds,
             * so its chunk records go too. One `DownloadState` covers the whole
             * transfer — several files arrive under one id — and the transfer is
             * done only once no file has chunks left and every expected byte has
             * arrived. Marking it `Completed` on the first file, as this did
             * while a transfer was one file, made every later file's chunks be
             * answered as already delivered and never written.
             */
            record.remove_file(file_id);

            let transfer_complete = record.chunks.is_empty()
                && download.received_bytes.load(Ordering::Relaxed) >= download.total_bytes;

            {
                let mut phases = download.phases.lock().expect("phases lock poisoned");

                phases.final_assembly_seconds = Some(merge_started.elapsed().as_secs_f64());
            }

            download.report.file_completed(file_id);

            if transfer_complete {
                /*
                 * The journal was this transfer's durable record all along;
                 * fold it into the manifest once, now that no chunk needs it.
                 * The transfer lock is released first: this fsync is not on any
                 * chunk's path and must never be something the next chunk waits
                 * behind.
                 *
                 * The manifest's own lock (`record`) stays held across the
                 * write, as `manifest::store` requires.
                 */
                drop(_held);

                if let Err(error) = state
                    .store_manifest(transfer_id, &transfer_dir, &record, false)
                    .await
                {
                    tracing::warn!(
                        transfer_id,
                        %error,
                        "could not write the final manifest; a restart will replay the journal"
                    );
                }

                *download.status.lock().expect("download status poisoned") =
                    TransferStatus::Completed;

                download.report.set_manifest_totals(
                    record.chunk_count(),
                    record.memory_estimate(),
                    state.manifest_lookup_summary(transfer_id),
                );

                finalize_download(state, download, TransferStatus::Completed, None, None, None)
                    .await;

                state.events.emit(DomainEvent::TransferCompleted {
                    progress: progress::make_download(download),
                });
            }

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
            if transfer_complete {
                state.forget_transfer(transfer_id).await;
            }

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
             * re-write the chunks, which is the safe answer. A transfer with
             * files still to publish keeps its manifest in memory.
             */
            if transfer_complete {
                state.forget_manifest(transfer_id).await;
            }

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
            /*
             * `assemble_and_publish` wrote the pruned manifest. Its duration is
             * inside `assemble_and_publish`, so only the count is attributed
             * here.
             *
             * shortcut: the rejected-assembly store's duration is not in
             * recv_compaction_total_seconds, only its count; upgrade if that
             * path ever becomes common.
             */
            state
                .compaction_stats(transfer_id)
                .count
                .fetch_add(1, Ordering::Relaxed);

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

/// Keep the receiver's network report current.
///
/// Throttled to [`network_report::NETWORK_REPORT_INTERVAL`] on the chunk path —
/// a write per chunk would cost more than the report is worth — and forced on
/// the first and final write so the file always has a start and an end.
pub(crate) async fn write_network_report(
    state: &ReceiverState,
    download: &DownloadState,
    status: TransferStatus,
    force: bool,
) {
    {
        let mut last = download
            .network_written
            .lock()
            .expect("network report lock poisoned");

        if !force && last.elapsed() < network_report::NETWORK_REPORT_INTERVAL {
            return;
        }

        *last = std::time::Instant::now();
    }

    let report = {
        let mut report = download
            .network_report
            .lock()
            .expect("network report lock poisoned");

        report.update(
            crate::transfer::report::status_name(status),
            download.received_bytes.load(Ordering::Relaxed),
            download.total_bytes,
            download.received_chunks.load(Ordering::Relaxed),
            0,
        );

        report.clone()
    };

    if let Err(error) = network_report::write(&state.root.join("reports"), &report).await {
        tracing::warn!(
            transfer_id = %download.transfer_id,
            %error,
            "could not write the network report"
        );
    }
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
    write_network_report(state, download, status.clone(), true).await;

    /*
     * The receiver's own compaction cost, for its report. The sender gets the
     * same numbers on the chunk headers instead.
     */
    {
        let stats = state.compaction_stats(&download.transfer_id);

        download.report.set_compaction_totals(
            stats.count.load(Ordering::Relaxed),
            stats.total_ns.load(Ordering::Relaxed),
            stats.under_lock_ns.load(Ordering::Relaxed),
        );
    }

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
            "transport (the sender chooses the path; the receiver is not told which)"
                .into(),
            "transport_manifest (built for the accept call, not retained by the receiver)"
                .into(),
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

    struct TestHostWithDownloads(std::path::PathBuf);

    impl crate::transfer::writer::ReceiverHost for TestHostWithDownloads {
        fn downloads_root(&self) -> std::result::Result<std::path::PathBuf, String> {
            Ok(self.0.clone())
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

    fn unique_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "vilsend-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is after 1970")
                .as_nanos()
        ))
    }

    /// A folder arrives as several files under one transfer id. The transfer is
    /// done when the last file publishes, not the first: marking it complete
    /// early makes the receiver answer every remaining chunk as already
    /// delivered, and only one file of the folder is ever written.
    #[tokio::test]
    async fn a_transfer_completes_after_its_last_file_not_its_first() {
        let root = unique_root("sink-multi");
        let downloads = root.join("downloads");

        let state = ReceiverState::new(
            std::sync::Arc::new(TestHostWithDownloads(downloads.clone())),
            std::sync::Arc::new(vilsend_core::RecordingEventSink::new()),
            root.clone(),
        );

        let download = DownloadState {
            total_bytes: 10,
            ..download("t-multi")
        };

        for (file_id, bytes) in [("f-1", b"first".as_slice()), ("f-2", b"secnd")] {
            assert!(matches!(
                accept(&state, &download, "t-multi", file_id, 0, 1, bytes.to_vec()).await,
                Ok((Accepted::Stored, _))
            ));
        }

        let first = finish(&state, &download, "t-multi", "f-1", 1, "nested/first.bin").await;

        assert!(matches!(first, Ok(Completion::Published)));
        assert_ne!(
            *download.status.lock().expect("status"),
            TransferStatus::Completed,
            "one file of two is not the whole transfer"
        );

        let second = finish(&state, &download, "t-multi", "f-2", 1, "nested/second.bin").await;

        assert!(matches!(second, Ok(Completion::Published)));
        assert_eq!(
            *download.status.lock().expect("status"),
            TransferStatus::Completed
        );
        assert!(downloads.join("nested/first.bin").exists());
        assert!(downloads.join("nested/second.bin").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file is written at its offsets as its chunks arrive, and the assembly
    /// file is published directly — no `.part` files and no second copy. The
    /// sender here describes no digests, exactly as the shipped one does.
    #[tokio::test]
    async fn chunks_written_at_their_offsets_publish_without_a_second_copy() {
        let root = unique_root("sink-direct");
        let downloads = root.join("downloads");

        let state = ReceiverState::new(
            std::sync::Arc::new(TestHostWithDownloads(downloads.clone())),
            std::sync::Arc::new(vilsend_core::RecordingEventSink::new()),
            root.clone(),
        );

        let download = DownloadState {
            total_bytes: 10,
            ..download("t-direct")
        };

        let chunks: [(u64, &[u8]); 3] = [(0, b"abcd"), (1, b"efgh"), (2, b"ij")];

        // The tail arrives after one middle chunk, so the chunk size is known
        // by the time it is placed.
        for (index, bytes) in [chunks[1], chunks[2], chunks[0]] {
            assert!(matches!(
                accept(
                    &state,
                    &download,
                    "t-direct",
                    "f-1",
                    index,
                    3,
                    bytes.to_vec()
                )
                .await,
                Ok((Accepted::Stored, _))
            ));
        }

        let parts = state.transfer_dir("t-direct").join("f-1");

        assert!(manifest::assembly_path(&parts).exists());
        assert!(
            !manifest::part_path(&parts, 0).exists(),
            "the direct layout writes no parts"
        );

        let completion = finish(&state, &download, "t-direct", "f-1", 3, "a.bin").await;

        assert!(matches!(completion, Ok(Completion::Published)));
        assert_eq!(
            std::fs::read(downloads.join("a.bin")).expect("published"),
            b"abcdefghij"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file whose tail is the very first chunk it sees has no chunk size to be
    /// placed by, so it keeps the `.part` layout and is merged at completion —
    /// still delivered, still correct.
    #[tokio::test]
    async fn a_tail_arriving_first_falls_back_to_parts_and_still_publishes() {
        let root = unique_root("sink-tail-first");
        let downloads = root.join("downloads");

        let state = ReceiverState::new(
            std::sync::Arc::new(TestHostWithDownloads(downloads.clone())),
            std::sync::Arc::new(vilsend_core::RecordingEventSink::new()),
            root.clone(),
        );

        let download = DownloadState {
            total_bytes: 10,
            ..download("t-tail-first")
        };

        let chunks: [(u64, &[u8]); 3] = [(0, b"abcd"), (1, b"efgh"), (2, b"ij")];

        for (index, bytes) in [chunks[2], chunks[0], chunks[1]] {
            assert!(matches!(
                accept(
                    &state,
                    &download,
                    "t-tail-first",
                    "f-1",
                    index,
                    3,
                    bytes.to_vec()
                )
                .await,
                Ok((Accepted::Stored, _))
            ));
        }

        let parts = state.transfer_dir("t-tail-first").join("f-1");

        assert!(
            !manifest::assembly_path(&parts).exists(),
            "no assembly file for a file that had to keep parts"
        );
        assert!(manifest::part_path(&parts, 0).exists());

        let completion = finish(&state, &download, "t-tail-first", "f-1", 3, "a.bin").await;

        assert!(matches!(completion, Ok(Completion::Published)));
        assert_eq!(
            std::fs::read(downloads.join("a.bin")).expect("published"),
            b"abcdefghij"
        );

        let _ = std::fs::remove_dir_all(&root);
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
            network_report: std::sync::Mutex::new(crate::transfer::network_report::NetworkReport::new(
                crate::transfer::report::Role::Receiver,
                transfer_id,
                Vec::new(),
                None,
            )),
            network_written: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }

    /// An acknowledged chunk survives a process restart: what a fresh
    /// `ReceiverState` reads back comes from the journal on disk, not memory.
    ///
    /// This is the batching invariant stated as a test. `accept` is not allowed
    /// to return until its entry is fsynced, so even the chunk whose batch was
    /// smallest must be in the journal by the time it is acknowledged.
    #[tokio::test]
    async fn an_acknowledged_chunk_is_durable_across_a_restart() {
        let root = unique_root("sink-durable");

        let state = ReceiverState::new(
            std::sync::Arc::new(TestHost),
            std::sync::Arc::new(vilsend_core::RecordingEventSink::new()),
            root.clone(),
        );

        let transfer = download("t-durable");

        // Two chunks of one size, the way the sender plans them.
        for (index, bytes) in [(0u64, b"first".as_slice()), (1, b"forth")] {
            assert!(matches!(
                accept(
                    &state,
                    &transfer,
                    "t-durable",
                    "f-1",
                    index,
                    2,
                    bytes.to_vec()
                )
                .await,
                Ok((Accepted::Stored, _))
            ));
        }

        // Chunk traffic is journal-only: no manifest write, so no fsync of a
        // growing document behind the transfer lock.
        assert_eq!(
            state
                .compaction_stats("t-durable")
                .count
                .load(Ordering::Relaxed),
            0
        );

        let restarted = ReceiverState::new(
            std::sync::Arc::new(TestHost),
            std::sync::Arc::new(vilsend_core::RecordingEventSink::new()),
            root.clone(),
        );

        let handle = restarted.manifest_handle("t-durable").await;

        {
            let record = handle.lock().await;

            for index in [0u64, 1] {
                let entry = record
                    .entry("f-1", index)
                    .expect("an acknowledged chunk must be in the journal");

                let assembly =
                    manifest::assembly_path(&restarted.transfer_dir("t-durable").join("f-1"));

                assert!(
                    manifest::chunk_matches(
                        &assembly,
                        entry.offset.expect("a direct chunk records its offset"),
                        entry,
                    )
                    .await
                );
            }
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A duplicate of a chunk that is still being written waits for the first
    /// attempt and is then answered from the durable record, so exactly one
    /// part write (and one part fsync) happens for the index.
    #[tokio::test]
    async fn a_duplicate_chunk_is_written_once() {
        let root = unique_root("sink-duplicate");

        let state = ReceiverState::new(
            std::sync::Arc::new(TestHost),
            std::sync::Arc::new(vilsend_core::RecordingEventSink::new()),
            root.clone(),
        );

        let transfer = download("t-dup");

        let (first, second) = tokio::join!(
            accept(&state, &transfer, "t-dup", "f-1", 0, 1, b"payload".to_vec()),
            accept(&state, &transfer, "t-dup", "f-1", 0, 1, b"payload".to_vec()),
        );

        let stored = [&first, &second]
            .iter()
            .filter(|result| matches!(result, Ok((Accepted::Stored, _))))
            .count();

        let present = [&first, &second]
            .iter()
            .filter(|result| matches!(result, Ok((Accepted::AlreadyPresent, _))))
            .count();

        assert_eq!(stored, 1, "one attempt owns the write");
        assert_eq!(present, 1, "the duplicate is answered from the record");

        let _ = std::fs::remove_dir_all(&root);
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
            accept(&state, &first, "t-a", "f-1", 0, 1, b"first".to_vec()),
            accept(&state, &second, "t-b", "f-1", 0, 1, b"second".to_vec()),
        );

        assert!(matches!(a, Ok((Accepted::Stored, _))));
        assert!(matches!(b, Ok((Accepted::Stored, _))));

        let a = state.manifest_handle("t-a").await.lock().await.clone();
        let b = state.manifest_handle("t-b").await.lock().await.clone();

        assert_eq!(a.chunks.len(), 1);
        assert_eq!(b.chunks.len(), 1);
        assert_ne!(
            a.entry("f-1", 0).expect("a recorded").chunk_hash,
            b.entry("f-1", 0).expect("b recorded").chunk_hash,
            "each transfer recorded its own bytes"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
