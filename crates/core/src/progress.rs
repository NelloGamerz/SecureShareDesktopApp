//! Transfer progress snapshots.
//!
//! Two producers existed in the shell and they are **not** the same
//! arithmetic:
//!
//! | | upload (`transfer/progress.rs`) | download (`models/progress.rs`) |
//! |---|---|---|
//! | elapsed | floored at 1 ms | used as measured, may be 0 |
//! | `total_bytes == 0` | 100 % | 0 % |
//! | percentage | `bytes * 100.0 / total` | `(bytes / total) * 100.0` |
//! | ETA rounding | `ceil` | truncation |
//! | bytes ≥ total | `saturating_sub`, so ETA 0 | ETA absent |
//!
//! Phase 2 is a pure refactor, so both are preserved exactly — including the
//! rounding difference, which is observable in the last float digit. Both are
//! characterised by tests rather than tidied up; unifying them is a behaviour
//! change for whoever wants it.

use serde::Serialize;

use crate::status::TransferStatus;

/// One progress sample, serialised exactly as the webview receives it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TransferProgress {
    pub transfer_id: String,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
    pub percentage: f64,
    pub speed: f64,
    pub eta: Option<u64>,
    pub status: TransferStatus,
}

/// Builds the progress sample for an upload.
///
/// `elapsed_secs` is supplied rather than measured so this is testable against
/// a fake clock; the caller owns the `Instant`.
pub fn upload_progress(
    transfer_id: impl Into<String>,
    uploaded_bytes: u64,
    total_bytes: u64,
    elapsed_secs: f64,
    status: TransferStatus,
) -> TransferProgress {
    let elapsed = elapsed_secs.max(0.001);
    let speed = uploaded_bytes as f64 / elapsed;
    let remaining = total_bytes.saturating_sub(uploaded_bytes);

    TransferProgress {
        transfer_id: transfer_id.into(),
        uploaded_bytes,
        total_bytes,
        percentage: if total_bytes == 0 {
            100.0
        } else {
            uploaded_bytes as f64 * 100.0 / total_bytes as f64
        },
        speed,
        eta: if speed > 0.0 {
            Some((remaining as f64 / speed).ceil() as u64)
        } else {
            None
        },
        status,
    }
}

/// Builds the progress sample for a download.
///
/// The receiver reports the byte count it has written in the `uploaded_bytes`
/// field, which is what the webview reads; that name is kept.
pub fn download_progress(
    transfer_id: impl Into<String>,
    received_bytes: u64,
    total_bytes: u64,
    elapsed_secs: f64,
    status: TransferStatus,
) -> TransferProgress {
    let percentage = if total_bytes == 0 {
        0.0
    } else {
        (received_bytes as f64 / total_bytes as f64) * 100.0
    };

    let speed = if elapsed_secs > 0.0 {
        received_bytes as f64 / elapsed_secs
    } else {
        0.0
    };

    let eta = if speed > 0.0 && total_bytes > received_bytes {
        Some(((total_bytes - received_bytes) as f64 / speed) as u64)
    } else {
        None
    };

    TransferProgress {
        transfer_id: transfer_id.into(),
        uploaded_bytes: received_bytes,
        total_bytes,
        percentage,
        speed,
        eta,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These are characterisation tests: they pin what the shell did before the
    // types moved, including the parts that look like mistakes. They are not a
    // specification to be tidied up — changing one of these numbers changes
    // what the user sees.

    const ID: &str = "transfer-1";

    fn upload(sent: u64, total: u64, elapsed_secs: f64) -> TransferProgress {
        upload_progress(ID, sent, total, elapsed_secs, TransferStatus::Uploading)
    }

    fn download(received: u64, total: u64, elapsed_secs: f64) -> TransferProgress {
        download_progress(
            ID,
            received,
            total,
            elapsed_secs,
            TransferStatus::Downloading,
        )
    }

    #[test]
    fn upload_is_complete_when_the_total_is_unknown() {
        let progress = upload(0, 0, 1.0);

        assert_eq!(progress.percentage, 100.0);
        assert_eq!(progress.speed, 0.0);
        assert_eq!(progress.eta, None);
    }

    #[test]
    fn download_is_not_complete_when_the_total_is_unknown() {
        let progress = download(0, 0, 1.0);

        assert_eq!(progress.percentage, 0.0);
        assert_eq!(progress.speed, 0.0);
        assert_eq!(progress.eta, None);
    }

    #[test]
    fn upload_eta_rounds_up() {
        let progress = upload(300, 1_000, 1.0);

        assert_eq!(progress.percentage, 30.0);
        assert_eq!(progress.speed, 300.0);
        // 700 bytes left at 300 B/s is 2.33 s, reported as 3.
        assert_eq!(progress.eta, Some(3));
    }

    #[test]
    fn download_eta_truncates() {
        let progress = download(300, 1_000, 1.0);

        assert_eq!(progress.percentage, 30.0);
        assert_eq!(progress.speed, 300.0);
        // The same 2.33 s, reported as 2 — the two sides disagree on rounding.
        assert_eq!(progress.eta, Some(2));
    }

    #[test]
    fn upload_clamps_a_zero_elapsed_time_to_one_millisecond() {
        let progress = upload(1_000, 2_000, 0.0);

        // Without the clamp this would be a division by zero.
        assert_eq!(progress.speed, 1_000_000.0);
        assert_eq!(progress.eta, Some(1));
    }

    #[test]
    fn download_reports_no_speed_before_any_time_has_passed() {
        let progress = download(500, 1_000, 0.0);

        assert_eq!(progress.percentage, 50.0);
        assert_eq!(progress.speed, 0.0);
        assert_eq!(progress.eta, None);
    }

    #[test]
    fn upload_eta_is_zero_once_more_than_the_total_has_been_sent() {
        let progress = upload(1_500, 1_000, 1.0);

        // `saturating_sub` floors the remaining bytes at zero.
        assert_eq!(progress.eta, Some(0));
        assert_eq!(progress.percentage, 150.0);
        assert_eq!(progress.speed, 1_500.0);
    }

    #[test]
    fn download_eta_is_absent_once_everything_has_arrived() {
        let progress = download(1_000, 1_000, 1.0);

        assert_eq!(progress.percentage, 100.0);
        assert_eq!(progress.eta, None);
    }

    #[test]
    fn the_two_sides_round_a_percentage_differently() {
        // `sent * 100.0 / total` and `(received / total) * 100.0` are not the
        // same floating-point expression. One byte of three is the smallest
        // case where the difference is visible.
        assert_eq!(upload(1, 3, 1.0).percentage, 33.333333333333336);
        assert_eq!(download(1, 3, 1.0).percentage, 33.33333333333333);
    }

    #[test]
    fn the_reported_status_is_the_one_that_was_passed_in() {
        let progress = download_progress(ID, 1, 2, 1.0, TransferStatus::Paused);

        assert_eq!(progress.status, TransferStatus::Paused);
        assert_eq!(progress.transfer_id, ID);
        // The receiver reports what it has written in the `uploaded_bytes`
        // field; the frontend reads that name.
        assert_eq!(progress.uploaded_bytes, 1);
    }
}
