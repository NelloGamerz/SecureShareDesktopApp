//! Golden payload fixtures for the shell's side of the event port.
//!
//! Phase 2 moved `TransferProgress`, `TransferStatus` and `ConnectionStatus`
//! into `vilsend-core` and put every emitted event behind an `EventSink`. The
//! point of that change is that the webview sees exactly the same bytes
//! afterwards, so this module pins what [`TauriEventSink`] is handed for each
//! domain event against fixtures captured *before* the refactor.
//!
//! The fixtures live in `tests/fixtures/golden-payloads/` at the repository
//! root and are shared with `vilsend-core`, which pins the encodings of the
//! types themselves. This module pins the mapping on top of them.
//!
//! Regenerate with `UPDATE_GOLDEN=1 cargo test -p vilsend golden`.
//!
//! [`TauriEventSink`]: crate::events::TauriEventSink

use std::path::PathBuf;

use vilsend_core::{DomainEvent, TransferProgress, TransferStatus};

use crate::events::sink::wire_payload;

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
        expected,
        actual,
        "wire format for {name} changed — this is a breaking change for the webview"
    );
}

fn check_serialised<T: serde::Serialize>(name: &str, value: &T) {
    let mut json = serde_json::to_string_pretty(value).expect("value is serialisable");
    json.push('\n');

    assert_fixture(name, json);
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

/// An in-flight download at the quarter mark.
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

/// A transfer that finished with no measurable elapsed time.
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

/// Every transfer event the application emits, carrying `progress`.
fn every_event(progress: &TransferProgress) -> Vec<DomainEvent> {
    vec![
        DomainEvent::TransferProgress {
            progress: progress.clone(),
        },
        DomainEvent::TransferCompleted {
            progress: progress.clone(),
        },
        DomainEvent::TransferFailed {
            progress: progress.clone(),
        },
        DomainEvent::TransferPaused {
            progress: progress.clone(),
        },
        DomainEvent::TransferResumed {
            progress: progress.clone(),
        },
        DomainEvent::TransferCancelled {
            progress: progress.clone(),
        },
    ]
}

#[test]
fn the_sink_uses_the_names_the_frontend_listens_for() {
    let names: Vec<&str> = every_event(&uploading_progress())
        .iter()
        .map(|event| wire_payload(event).0)
        .collect();

    check_serialised("transfer-event-names.json", &names);
}

#[test]
fn the_sink_sends_an_upload_payload_unchanged() {
    let event = DomainEvent::TransferProgress {
        progress: uploading_progress(),
    };
    let (name, payload) = wire_payload(&event);

    assert_eq!(name, "transfer-progress");
    check_serialised("transfer-progress-uploading.json", payload);
}

#[test]
fn the_sink_sends_a_download_payload_unchanged() {
    let event = DomainEvent::TransferProgress {
        progress: downloading_progress(),
    };
    let (name, payload) = wire_payload(&event);

    assert_eq!(name, "transfer-progress");
    check_serialised("transfer-progress-downloading.json", payload);
}

#[test]
fn the_sink_sends_a_completion_payload_unchanged() {
    let event = DomainEvent::TransferCompleted {
        progress: completed_progress(),
    };
    let (name, payload) = wire_payload(&event);

    assert_eq!(name, "transfer-completed");
    check_serialised("transfer-progress-completed.json", payload);
}

#[test]
fn every_event_keeps_its_payload_through_the_sink() {
    let progress = uploading_progress();

    for event in every_event(&progress) {
        let (_, payload) = wire_payload(&event);

        assert_eq!(payload, &progress);
    }
}
