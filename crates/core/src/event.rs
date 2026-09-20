//! The event port (ADR-0012).
//!
//! Before this, transfer progress reached the UI through exactly one seam —
//! `app.emit(name, progress)` in the desktop shell — with the event *names* as
//! string literals scattered across six call sites. Renaming one, or
//! misspelling one, produced no compile error and no runtime error: it just
//! silently stopped firing in the webview.
//!
//! `DomainEvent` makes that class of bug a compile error, and `EventSink`
//! makes the seam reusable by a CLI, an SDK or a mobile shell.

use crate::progress::TransferProgress;

/// Everything the core reports to a shell.
///
/// Every variant carries a [`TransferProgress`] because every transfer event
/// the application emits today carries exactly that payload. Phase 2 is a pure
/// refactor, so the enum reproduces the existing payloads field for field
/// rather than introducing the richer per-variant shapes ADR-0012 sketches
/// (`TransferFailed { error: ErrorKind }`, `TransferPaused { id }`): those
/// would change what the webview receives, which is precisely what this phase
/// forbids.
///
/// `#[non_exhaustive]` so a shell that chooses to match on this can be built
/// against a newer core. Shells that go through [`DomainEvent::wire_name`] and
/// [`DomainEvent::progress`] need no catch-all arm.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum DomainEvent {
    TransferProgress { progress: TransferProgress },
    TransferCompleted { progress: TransferProgress },
    TransferFailed { progress: TransferProgress },
    TransferPaused { progress: TransferProgress },
    TransferResumed { progress: TransferProgress },
    TransferCancelled { progress: TransferProgress },
}

impl DomainEvent {
    /// The name this event is emitted under.
    ///
    /// **Deviates from ADR-0012 §1**, which puts the wire name in the shell so
    /// that each shell owns its own naming. It lives here instead because the
    /// name is the same in every shell that emits a Tauri-compatible payload,
    /// because a typo is then impossible anywhere rather than in one place,
    /// and because the exhaustive `match` below is what makes the mapping
    /// testable from `vilsend-core` without a Tauri runtime.
    ///
    /// These strings are the frontend's `listen(...)` names. They are not
    /// free to change: `tests/fixtures/golden-payloads/transfer-event-names.json`
    /// pins them.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Self::TransferProgress { .. } => "transfer-progress",
            Self::TransferCompleted { .. } => "transfer-completed",
            Self::TransferFailed { .. } => "transfer-failed",
            Self::TransferPaused { .. } => "transfer-paused",
            Self::TransferResumed { .. } => "transfer-resumed",
            Self::TransferCancelled { .. } => "transfer-cancelled",
        }
    }

    /// The payload this event carries.
    pub fn progress(&self) -> &TransferProgress {
        match self {
            Self::TransferProgress { progress }
            | Self::TransferCompleted { progress }
            | Self::TransferFailed { progress }
            | Self::TransferPaused { progress }
            | Self::TransferResumed { progress }
            | Self::TransferCancelled { progress } => progress,
        }
    }
}

/// Where a shell receives domain events.
///
/// Implementations must not block: the transfer engine calls this from the
/// upload and download paths.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: DomainEvent);
}

/// An [`EventSink`] that keeps what it is given.
///
/// Shipped rather than duplicated per test module because every shell needs it
/// and it is a mutex and a `Vec`. Event *sequencing* was untestable before
/// this port existed.
#[derive(Debug, Default)]
pub struct RecordingEventSink {
    events: std::sync::Mutex<Vec<DomainEvent>>,
}

impl RecordingEventSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes everything recorded so far, leaving the sink empty.
    pub fn take(&self) -> Vec<DomainEvent> {
        let mut events = self
            .events
            .lock()
            .expect("recording event sink mutex poisoned");

        std::mem::take(&mut *events)
    }

    pub fn len(&self) -> usize {
        self.events
            .lock()
            .expect("recording event sink mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl EventSink for RecordingEventSink {
    fn emit(&self, event: DomainEvent) {
        self.events
            .lock()
            .expect("recording event sink mutex poisoned")
            .push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::TransferStatus;

    fn sample() -> TransferProgress {
        TransferProgress {
            transfer_id: "t-1".into(),
            uploaded_bytes: 1,
            total_bytes: 2,
            percentage: 50.0,
            speed: 1.0,
            eta: Some(1),
            status: TransferStatus::Uploading,
        }
    }

    fn every_variant() -> Vec<DomainEvent> {
        let progress = sample();

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
            DomainEvent::TransferCancelled { progress },
        ]
    }

    #[test]
    fn wire_names_match_the_event_names_the_frontend_listens_for() {
        let names: Vec<&str> = every_variant().iter().map(DomainEvent::wire_name).collect();

        assert_eq!(
            names,
            vec![
                "transfer-progress",
                "transfer-completed",
                "transfer-failed",
                "transfer-paused",
                "transfer-resumed",
                "transfer-cancelled",
            ]
        );
    }

    #[test]
    fn every_wire_name_is_distinct() {
        let mut names: Vec<&str> = every_variant().iter().map(DomainEvent::wire_name).collect();

        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), 6);
    }

    #[test]
    fn every_variant_exposes_its_payload() {
        for event in every_variant() {
            assert_eq!(event.progress().transfer_id, "t-1");
        }
    }

    #[test]
    fn recording_sink_keeps_events_in_order_and_drains_on_take() {
        let sink = RecordingEventSink::new();

        sink.emit(DomainEvent::TransferResumed { progress: sample() });
        sink.emit(DomainEvent::TransferCancelled { progress: sample() });

        assert_eq!(sink.len(), 2);
        assert!(!sink.is_empty());

        let names: Vec<&str> = sink.take().iter().map(DomainEvent::wire_name).collect();

        assert_eq!(names, vec!["transfer-resumed", "transfer-cancelled"]);
        assert!(sink.is_empty());
    }
}
