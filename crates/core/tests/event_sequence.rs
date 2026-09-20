//! Event sequencing, driven end to end through the port.
//!
//! Event order was untestable before `EventSink` existed, because the only way
//! an event left the engine was `app.emit` on a live `AppHandle`. These tests
//! drive the real progress arithmetic into a `RecordingEventSink` and assert
//! what a shell would have received.

use vilsend_core::{
    download_progress, upload_progress, DomainEvent, EventSink, RecordingEventSink, TransferStatus,
};

const ID: &str = "8f14e45f-ea42-4d7e-9b3a-1b2c3d4e5f60";

fn names(events: &[DomainEvent]) -> Vec<&'static str> {
    events.iter().map(DomainEvent::wire_name).collect()
}

#[test]
fn a_simulated_upload_emits_progress_then_completion() {
    let sink = RecordingEventSink::new();
    let total = 1_000_000;

    for sent in [0, 250_000, 500_000, 750_000, 1_000_000] {
        sink.emit(DomainEvent::TransferProgress {
            progress: upload_progress(ID, sent, total, 1.0, TransferStatus::Uploading),
        });
    }

    sink.emit(DomainEvent::TransferCompleted {
        progress: upload_progress(ID, total, total, 4.0, TransferStatus::Completed),
    });

    let events = sink.take();

    assert_eq!(
        names(&events),
        vec![
            "transfer-progress",
            "transfer-progress",
            "transfer-progress",
            "transfer-progress",
            "transfer-progress",
            "transfer-completed",
        ]
    );

    let percentages: Vec<f64> = events.iter().map(|e| e.progress().percentage).collect();

    assert_eq!(percentages, vec![0.0, 25.0, 50.0, 75.0, 100.0, 100.0]);

    let statuses: Vec<TransferStatus> =
        events.iter().map(|e| e.progress().status.clone()).collect();

    assert_eq!(statuses.last(), Some(&TransferStatus::Completed));
    assert!(sink.is_empty(), "take() must drain the sink");
}

#[test]
fn a_simulated_download_reports_a_failure_with_the_last_progress_it_had() {
    let sink = RecordingEventSink::new();
    let total = 4_000;

    sink.emit(DomainEvent::TransferProgress {
        progress: download_progress(ID, 1_000, total, 1.0, TransferStatus::Downloading),
    });

    sink.emit(DomainEvent::TransferFailed {
        progress: download_progress(ID, 1_000, total, 1.0, TransferStatus::Failed),
    });

    let events = sink.take();

    assert_eq!(names(&events), vec!["transfer-progress", "transfer-failed"]);

    // The failure carries the progress made before it, not an empty payload.
    let failed = events[1].progress();

    assert_eq!(failed.uploaded_bytes, 1_000);
    assert_eq!(failed.percentage, 25.0);
    assert_eq!(failed.status, TransferStatus::Failed);
}

#[test]
fn pause_resume_and_cancel_are_distinguishable_events_on_the_same_payload() {
    let sink = RecordingEventSink::new();
    let progress = || upload_progress(ID, 500, 1_000, 1.0, TransferStatus::Paused);

    sink.emit(DomainEvent::TransferPaused {
        progress: progress(),
    });
    sink.emit(DomainEvent::TransferResumed {
        progress: progress(),
    });
    sink.emit(DomainEvent::TransferCancelled {
        progress: progress(),
    });

    let events = sink.take();

    assert_eq!(
        names(&events),
        vec!["transfer-paused", "transfer-resumed", "transfer-cancelled"]
    );

    // All three carry an identical payload — the name is the only difference,
    // which is exactly why the name had to stop being a string literal.
    assert!(events
        .iter()
        .all(|event| event.progress() == events[0].progress()));
}
