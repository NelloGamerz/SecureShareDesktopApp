//! Deciding which chunks a resume may skip.
//!
//! A resume is chunk-based, not offset-based. The receiver is asked what it
//! holds, and every chunk it claims is checked against the sender's own copy
//! before it is believed. Nothing here trusts a claim: the receiver's answer is
//! a set of *proposals*, and a proposal survives only if the sender can match it
//! to a real chunk of a real file and reproduce its digest from the file on
//! disk.
//!
//! That check is what makes the difference between resuming and corrupting. A
//! receiver that has been through a restart, is running an older build, or has
//! stale files under a reused transfer id can report a chunk that does not match
//! what the sender is about to send. Skipping such a chunk would leave the
//! merged file with the wrong bytes and no error — the merge would succeed. So
//! anything that cannot be confirmed is sent again, which is always safe.
//!
//! Four things must line up for a claim to be honored:
//!
//! 1. `file_id` names a file in this transfer,
//! 2. `chunk_index` names a chunk that file actually has,
//! 3. `chunk_length` equals the length the sender's own plan gives that chunk,
//! 4. `chunk_hash` equals SHA-256 of the bytes the sender would send for it.
//!
//! The fourth is the load-bearing one: the first three are metadata either side
//! could agree on while the bytes underneath differ.

use crate::models::chunk::ChunkJob;
use crate::transfer::http_client::TransferResumeResponse;
use crate::transfer::integrity::ExpectedTransfer;
use crate::transfer::manifest::sha256_hex;
use crate::transfer::state::ChunkKey;
use std::collections::{HashMap, HashSet};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt},
};

/// What a receiver's report amounts to once it has been checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Confirmation {
    /// The chunks the receiver holds *and* the sender can prove it should.
    pub confirmed: HashSet<ChunkKey>,
    /// How many claims were made.
    pub claimed: usize,
    /// How many claims were thrown away, and why they were not believed.
    pub rejected: Vec<RejectedClaim>,
}

impl Confirmation {
    /// Whether every claim the receiver made was believed.
    pub fn is_trusted(&self) -> bool {
        self.rejected.is_empty()
    }
}

/// A claim that did not survive checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedClaim {
    pub file_id: String,
    pub chunk_index: u64,
    pub reason: Rejection,
}

/// Why a claim was not believed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// No chunk of this transfer has that file and index.
    UnknownChunk,
    /// The receiver's length does not match the sender's plan for that chunk.
    LengthMismatch { expected: u64, reported: u64 },
    /// The receiver's digest does not match the bytes on the sender's disk.
    HashMismatch { expected: String, reported: String },
    /// The sender could not read its own bytes to check the claim.
    Unreadable(String),
}

/// The chunks a receiver's report entitles the sender to skip.
///
/// Every claim is checked against `jobs`, which is the sender's own plan for the
/// transfer, and against the bytes on disk. A claim that fails any check is
/// dropped — the chunk is then sent again, which costs bandwidth and cannot cost
/// correctness.
///
/// A report for a different transfer is ignored entirely.
/// The no-plan form of [`confirmed_chunks_with_plan`]: every claim is checked
/// against the bytes on disk.
///
/// Kept as the plain name a caller with no plan to offer wants (and the name the
/// tests exercise the disk-reading path under); the upload path always has a
/// plan by the time it validates claims.
#[allow(dead_code)]
pub async fn confirmed_chunks(
    report: Option<&TransferResumeResponse>,
    transfer_id: &str,
    jobs: &[ChunkJob],
) -> Confirmation {
    confirmed_chunks_with_plan(report, transfer_id, jobs, None).await
}

/// Confirm claims against a digest plan instead of reading every chunk back.
///
/// `expected` must be a plan this run has already established describes the
/// files as they are now — see [`crate::transfer::integrity::plan_is_current`].
/// When it is, the per-chunk digest in it *is* the digest a read would compute,
/// so the read is skipped and only claims the plan cannot describe fall back to
/// [`chunk_digest`]. Passing `None` is exactly [`confirmed_chunks`]: every claim
/// is checked against the bytes on disk.
///
/// This is not a weaker check. The comparison a claim has to survive is
/// unchanged — `chunk_hash == SHA-256(the bytes the sender would send)` — and a
/// plan whose files have changed is never passed here; it is recomputed first.
pub async fn confirmed_chunks_with_plan(
    report: Option<&TransferResumeResponse>,
    transfer_id: &str,
    jobs: &[ChunkJob],
    expected: Option<&ExpectedTransfer>,
) -> Confirmation {
    let Some(report) = report else {
        return Confirmation::default();
    };

    if report.transfer_id != transfer_id {
        tracing::warn!(
            expected = transfer_id,
            reported = %report.transfer_id,
            "receiver reported on a different transfer; sending everything"
        );

        return Confirmation::default();
    }

    /*
     * Keyed once rather than scanned per claim. A report lists every chunk the
     * receiver holds, so a linear `find` made resume validation quadratic in the
     * transfer's own size.
     */
    let by_key: HashMap<(&str, u64), &ChunkJob> = jobs
        .iter()
        .map(|job| ((job.file_id.as_str(), job.index), job))
        .collect();

    let mut confirmation = Confirmation::default();

    for file in &report.files {
        for chunk in &file.chunks {
            confirmation.claimed += 1;

            let key = (file.file_id.clone(), chunk.chunk_index);

            let Some(job) = by_key
                .get(&(file.file_id.as_str(), chunk.chunk_index))
                .copied()
            else {
                confirmation.rejected.push(RejectedClaim {
                    file_id: file.file_id.clone(),
                    chunk_index: chunk.chunk_index,
                    reason: Rejection::UnknownChunk,
                });

                continue;
            };

            if job.length as u64 != chunk.chunk_length {
                confirmation.rejected.push(RejectedClaim {
                    file_id: file.file_id.clone(),
                    chunk_index: chunk.chunk_index,
                    reason: Rejection::LengthMismatch {
                        expected: job.length as u64,
                        reported: chunk.chunk_length,
                    },
                });

                continue;
            }

            /*
             * Where the plan describes this chunk, its digest stands in for the
             * one a read would compute. The plan is validated against the files
             * before it is passed in, and the chunk's length was just checked, so
             * this is the same comparison with the file already read once.
             */
            let planned = expected
                .and_then(|plan| plan.file(&file.file_id))
                .and_then(|plan| {
                    /*
                     * `ExpectedFile::chunk` is a linear scan; the plan is in
                     * index order, so a report of every chunk would scan the
                     * plan once per chunk. Index directly and fall back to the
                     * lookup only if the order is not what this build writes.
                     */
                    plan.chunks
                        .get(chunk.chunk_index as usize)
                        .filter(|planned| planned.index == chunk.chunk_index)
                        .or_else(|| plan.chunk(chunk.chunk_index))
                })
                .filter(|planned| planned.length == chunk.chunk_length)
                .map(|planned| planned.sha256.clone());

            let digest = match planned {
                Some(digest) => digest,

                None => match chunk_digest(job).await {
                    Ok(digest) => digest,

                    Err(error) => {
                        confirmation.rejected.push(RejectedClaim {
                            file_id: file.file_id.clone(),
                            chunk_index: chunk.chunk_index,
                            reason: Rejection::Unreadable(error.to_string()),
                        });

                        continue;
                    }
                },
            };

            if digest == chunk.chunk_hash {
                confirmation.confirmed.insert(key);
            } else {
                confirmation.rejected.push(RejectedClaim {
                    file_id: file.file_id.clone(),
                    chunk_index: chunk.chunk_index,
                    reason: Rejection::HashMismatch {
                        expected: digest,
                        reported: chunk.chunk_hash.clone(),
                    },
                });
            }
        }
    }

    if !confirmation.rejected.is_empty() {
        tracing::warn!(
            transfer_id,
            claimed = confirmation.claimed,
            confirmed = confirmation.confirmed.len(),
            rejected = confirmation.rejected.len(),
            "receiver claims did not all match local chunk metadata; \
             the ones that did not will be sent again"
        );
    }

    tracing::info!(
        transfer_id,
        claimed = confirmation.claimed,
        confirmed = confirmation.confirmed.len(),
        "receiver resume state checked"
    );

    confirmation
}

/// SHA-256 of the plaintext bytes of one chunk, as the receiver records them.
///
/// This reads the chunk the same way an upload does, so the digest is of exactly
/// the bytes that would be sent.
pub async fn chunk_digest(job: &ChunkJob) -> std::io::Result<String> {
    let mut file = fs::File::open(&job.path).await?;

    file.seek(std::io::SeekFrom::Start(job.offset)).await?;

    let mut bytes = vec![0u8; job.length];

    file.read_exact(&mut bytes).await?;

    Ok(sha256_hex(&bytes))
}

/// The jobs still to be sent, and what the chunks being skipped are worth.
///
/// Returns the jobs whose chunk is not confirmed, plus the byte and chunk counts
/// of the ones being skipped — a resume's progress has to account for both, or
/// it would report a transfer that is nearly finished as nearly empty.
pub fn missing_jobs(
    jobs: Vec<ChunkJob>,
    confirmed: &HashSet<ChunkKey>,
) -> (Vec<ChunkJob>, u64, u64) {
    let mut skipped_bytes = 0u64;
    let mut skipped_chunks = 0u64;
    let mut missing = Vec::new();

    for job in jobs {
        if confirmed.contains(&(job.file_id.clone(), job.index)) {
            skipped_bytes += job.length as u64;
            skipped_chunks += 1;
        } else {
            missing.push(job);
        }
    }

    (missing, skipped_bytes, skipped_chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::manifest::test_support::TempDir;
    use std::path::PathBuf;

    /// Lay out a source file and the jobs that would send it.
    fn plan(scratch: &TempDir, name: &str, contents: &[u8], chunk_size: usize) -> Vec<ChunkJob> {
        let path = scratch.path().join(name);
        std::fs::write(&path, contents).expect("writing the source file");

        let file = crate::models::file_info::FileInfo {
            id: format!("f-{name}"),
            path,
            relative_path: name.to_string(),
            size: contents.len() as u64,
            modified_nanos: None,
        };

        crate::transfer::chunker::jobs("t-1", &[file], chunk_size)
    }

    /// A report claiming every job is complete, with the real digest.
    async fn full_report(jobs: &[ChunkJob]) -> TransferResumeResponse {
        let mut files: Vec<crate::transfer::http_client::FileResumeState> = Vec::new();

        for job in jobs {
            let chunk = crate::transfer::http_client::ChunkResumeState {
                chunk_index: job.index,
                chunk_length: job.length as u64,
                chunk_hash: chunk_digest(job).await.expect("digest"),
            };

            match files.iter_mut().find(|file| file.file_id == job.file_id) {
                Some(file) => file.chunks.push(chunk),

                None => files.push(crate::transfer::http_client::FileResumeState {
                    file_id: job.file_id.clone(),
                    chunks: vec![chunk],
                }),
            }
        }

        TransferResumeResponse {
            transfer_id: "t-1".into(),
            files,
        }
    }

    /*
     * ----------------------------------------------------------------------
     * Nothing claimed, nothing skipped
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_receiver_that_reports_nothing_leaves_every_chunk_to_send() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        // `None` is what a receiver with no record of the transfer answers.
        let confirmation = confirmed_chunks(None, "t-1", &jobs).await;

        assert!(confirmation.confirmed.is_empty());

        let (missing, skipped_bytes, skipped_chunks) = missing_jobs(jobs, &confirmation.confirmed);

        assert_eq!(missing.len(), 3, "all three chunks must be sent");
        assert_eq!(skipped_bytes, 0);
        assert_eq!(skipped_chunks, 0);
    }

    #[tokio::test]
    async fn an_empty_report_leaves_every_chunk_to_send() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let report = TransferResumeResponse {
            transfer_id: "t-1".into(),
            files: Vec::new(),
        };

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(confirmation.confirmed.is_empty());
        assert_eq!(missing_jobs(jobs, &confirmation.confirmed).0.len(), 3);
    }

    /*
     * ----------------------------------------------------------------------
     * A report that checks out
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_full_report_skips_every_chunk() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let report = full_report(&jobs).await;
        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(confirmation.is_trusted(), "{:?}", confirmation.rejected);
        assert_eq!(confirmation.confirmed.len(), 3);

        let (missing, skipped_bytes, skipped_chunks) = missing_jobs(jobs, &confirmation.confirmed);

        assert!(missing.is_empty(), "nothing left to send");
        assert_eq!(skipped_bytes, 10, "every byte is accounted for");
        assert_eq!(skipped_chunks, 3);
    }

    #[tokio::test]
    async fn a_partial_report_skips_only_what_it_confirms() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let mut report = full_report(&jobs).await;

        // The receiver has chunks 0 and 2 but not 1.
        report.files[0]
            .chunks
            .retain(|chunk| chunk.chunk_index != 1);

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert_eq!(confirmation.confirmed.len(), 2);

        let (missing, skipped_bytes, skipped_chunks) = missing_jobs(jobs, &confirmation.confirmed);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].index, 1, "only the missing chunk is queued");

        // Chunk 0 is four bytes and chunk 2 is the two-byte tail, so six bytes
        // are already there and four still have to go.
        assert_eq!(skipped_bytes, 6);
        assert_eq!(skipped_chunks, 2);
        assert_eq!(missing[0].length, 4);
    }

    #[tokio::test]
    async fn missing_jobs_keeps_the_original_order() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let mut report = full_report(&jobs).await;
        report.files[0]
            .chunks
            .retain(|chunk| chunk.chunk_index != 0);

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;
        let (missing, _, _) = missing_jobs(jobs, &confirmation.confirmed);

        assert_eq!(missing[0].index, 0);
        assert_eq!(
            missing[0].offset, 0,
            "offsets come from the plan, not a claim"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * Claims that do not check out
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_claim_for_a_chunk_that_does_not_exist_is_rejected() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let report = TransferResumeResponse {
            transfer_id: "t-1".into(),
            files: vec![crate::transfer::http_client::FileResumeState {
                file_id: jobs[0].file_id.clone(),
                chunks: vec![crate::transfer::http_client::ChunkResumeState {
                    chunk_index: 99,
                    chunk_length: 4,
                    chunk_hash: sha256_hex(b"0123"),
                }],
            }],
        };

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(confirmation.confirmed.is_empty());
        assert_eq!(confirmation.rejected.len(), 1);
        assert_eq!(confirmation.rejected[0].reason, Rejection::UnknownChunk);
        assert_eq!(
            missing_jobs(jobs, &confirmation.confirmed).0.len(),
            3,
            "an unknown chunk is not a reason to skip anything"
        );
    }

    #[tokio::test]
    async fn a_claim_for_an_unknown_file_is_rejected() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let report = TransferResumeResponse {
            transfer_id: "t-1".into(),
            files: vec![crate::transfer::http_client::FileResumeState {
                file_id: "f-someone-elses-file".into(),
                chunks: vec![crate::transfer::http_client::ChunkResumeState {
                    chunk_index: 0,
                    chunk_length: 4,
                    chunk_hash: sha256_hex(b"0123"),
                }],
            }],
        };

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(confirmation.confirmed.is_empty());
        assert_eq!(confirmation.rejected[0].reason, Rejection::UnknownChunk);
    }

    #[tokio::test]
    async fn a_claim_with_the_wrong_length_is_rejected() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let mut report = full_report(&jobs).await;
        report.files[0].chunks[0].chunk_length = 999;

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(!confirmation
            .confirmed
            .contains(&(jobs[0].file_id.clone(), 0)));
        assert_eq!(
            confirmation.rejected[0].reason,
            Rejection::LengthMismatch {
                expected: 4,
                reported: 999
            }
        );
        assert_eq!(confirmation.confirmed.len(), 2, "the others still count");
    }

    #[tokio::test]
    async fn a_claim_with_the_wrong_hash_is_rejected() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let mut report = full_report(&jobs).await;

        // Right length, wrong bytes: the case the other three checks cannot see.
        report.files[0].chunks[0].chunk_hash = sha256_hex(b"XXXX");

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(
            !confirmation
                .confirmed
                .contains(&(jobs[0].file_id.clone(), 0)),
            "a chunk the receiver holds different bytes for must be sent again"
        );

        let (missing, _, _) = missing_jobs(jobs, &confirmation.confirmed);

        assert!(
            missing.iter().any(|job| job.index == 0),
            "and it must be back in the queue"
        );
    }

    #[tokio::test]
    async fn a_claim_that_cannot_be_checked_is_rejected() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let report = full_report(&jobs).await;

        // The source file is gone since the plan was made.
        std::fs::remove_file(&jobs[0].path).expect("removing the source");

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(confirmation.confirmed.is_empty());
        assert!(confirmation
            .rejected
            .iter()
            .all(|rejected| matches!(rejected.reason, Rejection::Unreadable(_))));
    }

    #[tokio::test]
    async fn a_report_for_another_transfer_is_ignored_entirely() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let mut report = full_report(&jobs).await;
        report.transfer_id = "t-someone-else".into();

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(
            confirmation.confirmed.is_empty(),
            "a report about a different transfer says nothing about this one"
        );
        assert_eq!(missing_jobs(jobs, &confirmation.confirmed).0.len(), 3);
    }

    #[tokio::test]
    async fn a_trusted_report_with_a_stray_claim_still_sends_the_stray_one() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        let mut report = full_report(&jobs).await;

        // One good claim, one invented.
        report.files[0]
            .chunks
            .push(crate::transfer::http_client::ChunkResumeState {
                chunk_index: 77,
                chunk_length: 4,
                chunk_hash: sha256_hex(b"made up"),
            });

        let confirmation = confirmed_chunks(Some(&report), "t-1", &jobs).await;

        assert!(!confirmation.is_trusted());
        assert_eq!(confirmation.confirmed.len(), 3, "the real ones still count");

        assert!(
            missing_jobs(jobs, &confirmation.confirmed).0.is_empty(),
            "an invented claim does not put a real chunk back in the queue"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * The digest is of the bytes that would be sent
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_chunks_digest_is_of_its_own_bytes_at_its_own_offset() {
        let scratch = TempDir::new();
        let jobs = plan(&scratch, "a.bin", b"0123456789", 4);

        assert_eq!(
            chunk_digest(&jobs[0]).await.expect("digest"),
            sha256_hex(b"0123")
        );
        assert_eq!(
            chunk_digest(&jobs[1]).await.expect("digest"),
            sha256_hex(b"4567")
        );
        assert_eq!(
            chunk_digest(&jobs[2]).await.expect("digest"),
            sha256_hex(b"89")
        );
    }

    #[tokio::test]
    async fn a_digest_of_asource_that_is_too_short_is_an_error_not_a_hash() {
        let scratch = TempDir::new();

        let path = scratch.path().join("short.bin");
        std::fs::write(&path, b"ab").expect("writing");

        // A job claiming more bytes than the file now has — the file was
        // truncated since the plan was made.
        let job = ChunkJob {
            transfer_id: "t-1".into(),
            file_id: "f-short".into(),
            index: 0,
            total: 1,
            relative_path: "short.bin".into(),
            path: PathBuf::from(&path),
            offset: 0,
            length: 10,
        };

        assert!(chunk_digest(&job).await.is_err());
    }

    /*
     * ----------------------------------------------------------------------
     * A claim can be checked against a plan instead of the disk
     * ----------------------------------------------------------------------
     */

    /// The jobs for a source file, and the digest plan describing it.
    async fn expected_for(
        scratch: &TempDir,
        name: &str,
        contents: &[u8],
        chunk_size: usize,
    ) -> (Vec<ChunkJob>, ExpectedTransfer) {
        let jobs = plan(scratch, name, contents, chunk_size);

        let file = crate::models::file_info::FileInfo {
            id: format!("f-{name}"),
            path: scratch.path().join(name),
            relative_path: name.to_string(),
            size: contents.len() as u64,
            modified_nanos: None,
        };

        let expected = crate::transfer::integrity::plan("t-1", &[file], chunk_size)
            .await
            .expect("planning");

        (jobs, expected)
    }

    /// A receiver report made from the plan's own digests.
    fn report_from_plan(expected: &ExpectedTransfer, indices: &[u64]) -> TransferResumeResponse {
        let file = &expected.files[0];

        TransferResumeResponse {
            transfer_id: expected.transfer_id.clone(),
            files: vec![crate::transfer::http_client::FileResumeState {
                file_id: file.file_id.clone(),
                chunks: file
                    .chunks
                    .iter()
                    .filter(|chunk| indices.contains(&chunk.index))
                    .map(|chunk| crate::transfer::http_client::ChunkResumeState {
                        chunk_index: chunk.index,
                        chunk_length: chunk.length,
                        chunk_hash: chunk.sha256.clone(),
                    })
                    .collect(),
            }],
        }
    }

    #[tokio::test]
    async fn a_claim_matching_the_plan_is_confirmed_without_reading_the_source() {
        let scratch = TempDir::new();
        let (jobs, expected) = expected_for(&scratch, "a.bin", b"0123456789", 4).await;

        /*
         * The source is gone. A claim confirmed from it would have produced an
         * `Unreadable` rejection, so a trusted confirmation proves the plan was
         * used and the file was never opened.
         */
        std::fs::remove_file(&jobs[0].path).expect("removing the source");

        let report = report_from_plan(&expected, &[0, 1, 2]);

        let confirmation =
            confirmed_chunks_with_plan(Some(&report), "t-1", &jobs, Some(&expected)).await;

        assert!(confirmation.is_trusted());
        assert_eq!(confirmation.confirmed.len(), 3);
    }

    #[tokio::test]
    async fn a_claim_that_disagrees_with_the_plan_is_rejected() {
        let scratch = TempDir::new();
        let (jobs, expected) = expected_for(&scratch, "a.bin", b"0123456789", 4).await;

        let mut report = report_from_plan(&expected, &[0]);
        report.files[0].chunks[0].chunk_hash = sha256_hex(b"some other bytes");

        std::fs::remove_file(&jobs[0].path).expect("removing the source");

        let confirmation =
            confirmed_chunks_with_plan(Some(&report), "t-1", &jobs, Some(&expected)).await;

        assert!(!confirmation.is_trusted());
        assert!(confirmation.confirmed.is_empty());
        assert!(matches!(
            confirmation.rejected[0].reason,
            Rejection::HashMismatch { .. }
        ));
    }

    #[tokio::test]
    async fn a_claim_the_plan_cannot_describe_falls_back_to_the_source() {
        let scratch = TempDir::new();
        let (jobs, mut expected) = expected_for(&scratch, "a.bin", b"0123456789", 4).await;

        let report = report_from_plan(&expected, &[1]);

        // A plan missing that one chunk says nothing about it; the read does.
        expected.files[0].chunks.retain(|chunk| chunk.index != 1);

        let confirmation =
            confirmed_chunks_with_plan(Some(&report), "t-1", &jobs, Some(&expected)).await;

        assert!(confirmation.is_trusted());
        assert!(confirmation.confirmed.contains(&("f-a.bin".to_string(), 1)));
    }
}
