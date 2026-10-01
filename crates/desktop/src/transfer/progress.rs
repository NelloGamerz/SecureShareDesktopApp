use crate::transfer::state::UploadState;
use std::sync::atomic::Ordering;
use vilsend_core::TransferProgress;

/// Samples an upload into a progress payload.
///
/// The arithmetic moved to `vilsend_core::upload_progress`, which takes the
/// elapsed time as an argument so that it can be tested against a fake clock
/// rather than a real `Instant`. The reads are kept in their original order:
/// the elapsed time is measured before the byte counter is read.
///
/// Both figures come from the plan. The total is what the transfer's files add
/// up to, and the byte count is what the acknowledged chunks are worth in that
/// plan — so a resume shows the chunks it is skipping as done from the moment it
/// is accepted, and a stale or duplicated acknowledgement cannot be counted.
/// The count is clamped at the total it is measured against, because a
/// percentage past 100 is never a truthful thing to show.
pub fn make(state: &UploadState) -> TransferProgress {
    let elapsed_secs = state.started_at.elapsed().as_secs_f64();

    let total = state.planned_bytes();
    let uploaded = state.uploaded_bytes.load(Ordering::Relaxed).min(total);

    vilsend_core::upload_progress(
        state.transfer_id.clone(),
        uploaded,
        total,
        elapsed_secs,
        state.status.lock().unwrap().clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        file_info::FileInfo,
        transfer::{ConnectionType, TransferStatus},
    };
    use crate::transfer::state::TransferCredentials;
    use std::{
        collections::HashSet,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, AtomicU32, AtomicU64},
            Mutex,
        },
    };

    /// An upload of files of the given sizes, split into `chunk_size` pieces,
    /// with nothing acknowledged yet.
    fn uploading(sizes: &[u64], chunk_size: usize) -> UploadState {
        let files = sizes
            .iter()
            .enumerate()
            .map(|(index, size)| FileInfo {
                id: format!("f-{index}"),
                path: PathBuf::from(format!("file-{index}.bin")),
                relative_path: format!("file-{index}.bin"),
                size: *size,
            })
            .collect();

        UploadState {
            transfer_id: "t-1".into(),
            credentials: Mutex::new(TransferCredentials::default()),
            endpoint: "https://receiver.example".into(),
            network_type: ConnectionType::Lan,
            files,
            chunk_size,
            concurrency: 2,
            total_bytes: 0,
            uploaded_bytes: AtomicU64::new(0),
            total_chunks: 0,
            uploaded_chunks: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            retry_count: AtomicU32::new(0),
            max_retries: 3,
            status: Mutex::new(TransferStatus::Uploading),
            started_at: std::time::Instant::now(),
            completed: Mutex::new(HashSet::new()),
            running: AtomicBool::new(false),
            resume_lock: tokio::sync::Mutex::new(()),
            expected: Mutex::new(None),
        }
    }

    fn keys(pairs: &[(&str, u64)]) -> HashSet<crate::transfer::state::ChunkKey> {
        pairs
            .iter()
            .map(|(file_id, index)| (file_id.to_string(), *index))
            .collect()
    }

    #[test]
    fn a_fresh_transfer_reports_no_progress() {
        // Two files: four four-byte chunks and one two-byte tail.
        let state = uploading(&[8, 2], 4);

        let progress = make(&state);

        assert_eq!(progress.total_bytes, 10, "the total is the files' own size");
        assert_eq!(progress.uploaded_bytes, 0);
        assert_eq!(progress.percentage, 0.0);
        assert!(!state.is_complete());
    }

    #[test]
    fn a_completed_transfer_reports_every_byte() {
        let state = uploading(&[8, 2], 4);

        for (file_id, index) in [("f-0", 0), ("f-0", 1), ("f-1", 0)] {
            assert!(state.mark_completed(file_id, index));
        }

        let progress = make(&state);

        assert_eq!(progress.uploaded_bytes, 10);
        assert_eq!(progress.total_bytes, 10);
        assert_eq!(progress.percentage, 100.0);
        assert!(state.is_complete(), "every expected chunk is acknowledged");
    }

    #[test]
    fn progress_is_capped_at_a_hundred_percent() {
        let state = uploading(&[8], 4);

        state.mark_completed("f-0", 0);
        state.mark_completed("f-0", 1);

        // A counter that has been inflated anyway — by an older build that
        // double-counted a repeated acknowledgement, say — is not a reason to
        // show the user more than the whole file.
        state.uploaded_bytes.store(4_000, Ordering::Relaxed);

        let progress = make(&state);

        assert_eq!(progress.uploaded_bytes, 8, "clamped to the total");
        assert_eq!(progress.percentage, 100.0);
    }

    #[test]
    fn a_chunk_the_plan_does_not_have_is_never_counted() {
        let state = uploading(&[8], 4);

        assert!(
            !state.mark_completed("f-0", 99),
            "an index past the end of the file is not a chunk of this transfer"
        );
        assert!(
            !state.mark_completed("f-nobody", 0),
            "a file that is not in the transfer has no chunks"
        );

        assert!(state.mark_completed("f-0", 0));
        assert!(
            !state.mark_completed("f-0", 0),
            "the same chunk acknowledged twice is still one chunk"
        );

        assert_eq!(state.uploaded_bytes.load(Ordering::Relaxed), 4);
        assert_eq!(state.uploaded_chunks.load(Ordering::Relaxed), 1);
        assert_eq!(make(&state).percentage, 50.0);
    }

    #[test]
    fn replacing_the_completed_set_keeps_only_what_the_plan_accounts_for() {
        let state = uploading(&[8], 4);

        let (bytes, chunks) = state.set_completed(keys(&[
            ("f-0", 0),
            ("f-0", 99),   // past the end of the file
            ("f-gone", 0), // not a file of this transfer
        ]));

        assert_eq!(bytes, 4, "only the real chunk is worth anything");
        assert_eq!(chunks, 1);
        assert_eq!(state.uploaded_bytes.load(Ordering::Relaxed), 4);
        assert_eq!(state.uploaded_chunks.load(Ordering::Relaxed), 1);
        assert_eq!(make(&state).percentage, 50.0);
        assert!(!state.is_complete());
    }

    #[test]
    fn a_transfer_with_an_empty_file_still_has_its_one_chunk() {
        // `chunker::jobs` gives even an empty file a single chunk, so the plan
        // and the accounting have to agree that it exists.
        let state = uploading(&[0], 4);

        assert_eq!(state.planned_chunks(), 1);
        assert_eq!(state.chunk_length("f-0", 0), Some(0));
        assert_eq!(state.chunk_length("f-0", 1), None);

        assert!(state.mark_completed("f-0", 0));
        assert!(state.is_complete());
    }
}
