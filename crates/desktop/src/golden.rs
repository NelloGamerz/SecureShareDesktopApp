//! Golden payload fixtures for the Phase 2 walking-skeleton refactor.
//!
//! Phase 2 moves `TransferProgress`, `TransferStatus` and `ConnectionStatus`
//! out of this crate and into `vilsend-core`, and routes every emitted event
//! through an `EventSink` port. The whole point of that change is that the
//! webview sees exactly the same bytes afterwards, so this module pins the
//! JSON that goes on the wire *today* and asserts it again afterwards.
//!
//! The fixtures live in `tests/fixtures/golden-payloads/` at the repository
//! root, deliberately outside this crate: they are shared with the
//! `vilsend-core` test that pins the same encodings after the move.
//!
//! Regenerate with `UPDATE_GOLDEN=1 cargo test -p vilsend golden`.
//!
//! The fixtures pin the *serialisation*, not the arithmetic that produces the
//! numbers — `transfer/progress.rs` and `models/progress.rs` derive their
//! speed and ETA from an `Instant`, which cannot be reproduced byte-for-byte.
//! The arithmetic is pinned separately, against a supplied elapsed time, in
//! `vilsend-core`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::models::progress::TransferProgress;
use crate::models::transfer::TransferStatus;
use crate::models::ConnectionStatus;

/// Every transfer event the application emits, with the payload it carries.
///
/// All six share one payload type, which is why one `EventSink` port can
/// serve the whole transfer module.
pub const TRANSFER_EVENT_NAMES: [&str; 6] = [
    "transfer-progress",
    "transfer-completed",
    "transfer-failed",
    "transfer-paused",
    "transfer-resumed",
    "transfer-cancelled",
];

fn fixtures_dir() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    loop {
        let candidate = dir.join("tests").join("fixtures").join("golden-payloads");

        if candidate.is_dir() {
            return candidate;
        }

        assert!(
            dir.pop(),
            "could not find tests/fixtures/golden-payloads above {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

fn assert_fixture(name: &str, actual: String) {
    let path = fixtures_dir().join(name);

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("create fixture directory");
        std::fs::write(&path, &actual).expect("write fixture");
        return;
    }

    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));

    assert_eq!(
        expected, actual,
        "wire format for {name} changed — this is a breaking change for the webview"
    );
}

fn check_serialised<T: serde::Serialize>(name: &str, value: &T) {
    let mut json = serde_json::to_string_pretty(value).expect("value is serialisable");
    json.push('\n');

    assert_fixture(name, json);
}

/// All variants of a `Serialize`-only enum, keyed by variant name.
fn variants<T, F>(entries: F) -> BTreeMap<String, Value>
where
    T: serde::Serialize,
    F: IntoIterator<Item = (&'static str, T)>,
{
    entries
        .into_iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                serde_json::to_value(value).expect("variant is serialisable"),
            )
        })
        .collect()
}

/// An in-flight upload at the half-way mark.
fn uploading_progress() -> TransferProgress {
    TransferProgress {
        transfer_id: "8f14e45f-ea42-4d7e-9b3a-1b2c3d4e5f60".into(),
        uploaded_bytes: 512,
        total_bytes: 1024,
        percentage: 50.0,
        speed: 256.0,
        eta: Some(2),
        status: TransferStatus::Uploading,
    }
}

/// An in-flight download at the quarter mark, before the first chunk lands.
fn downloading_progress() -> TransferProgress {
    TransferProgress {
        transfer_id: "3c59dc04-8e88-4b0a-9f5e-6d7c8b9a0f11".into(),
        uploaded_bytes: 3_000_000,
        total_bytes: 12_000_000,
        percentage: 25.0,
        speed: 1_000_000.0,
        eta: Some(9),
        status: TransferStatus::Downloading,
    }
}

/// A transfer that finished with no measurable elapsed time: `speed` is zero
/// and `eta` is absent, which is the shape the UI branches on.
fn completed_progress() -> TransferProgress {
    TransferProgress {
        transfer_id: "8f14e45f-ea42-4d7e-9b3a-1b2c3d4e5f60".into(),
        uploaded_bytes: 1024,
        total_bytes: 1024,
        percentage: 100.0,
        speed: 0.0,
        eta: None,
        status: TransferStatus::Completed,
    }
}

#[test]
fn transfer_progress_wire_format() {
    check_serialised("transfer-progress-uploading.json", &uploading_progress());
    check_serialised(
        "transfer-progress-downloading.json",
        &downloading_progress(),
    );
    check_serialised("transfer-progress-completed.json", &completed_progress());
}

#[test]
fn transfer_status_wire_format() {
    let all = variants([
        ("Queued", TransferStatus::Queued),
        ("Uploading", TransferStatus::Uploading),
        ("Paused", TransferStatus::Paused),
        ("Completed", TransferStatus::Completed),
        ("Failed", TransferStatus::Failed),
        ("Cancelled", TransferStatus::Cancelled),
        ("Pending", TransferStatus::Pending),
        ("Downloading", TransferStatus::Downloading),
    ]);

    check_serialised("transfer-status.json", &all);
}

#[test]
fn connection_status_wire_format() {
    let all = variants([
        ("Disconnected", ConnectionStatus::Disconnected),
        ("Connecting", ConnectionStatus::Connecting),
        ("Connected", ConnectionStatus::Connected),
        ("Reconnecting", ConnectionStatus::Reconnecting),
        ("Error", ConnectionStatus::Error("websocket closed".into())),
    ]);

    check_serialised("connection-status.json", &all);
}

#[test]
fn transfer_event_names_are_stable() {
    let names: Vec<&str> = TRANSFER_EVENT_NAMES.to_vec();

    check_serialised("transfer-event-names.json", &names);
}
