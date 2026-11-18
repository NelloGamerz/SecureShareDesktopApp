//! The desktop implementation of the [`EventSink`] port (ADR-0012).
//!
//! A domain event becomes a Tauri event with the same name and the same
//! payload it had before the port existed. Progress is coalesced first: the
//! engine emits once per chunk, and the webview only needs the latest sample.

use std::{collections::HashMap, time::Duration};

use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use vilsend_core::{DomainEvent, EventSink, TransferProgress};

/// How often buffered progress samples reach the webview.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// Flush one transfer early once this many samples have been buffered.
///
/// The ticker alone would hold the newest sample for up to
/// [`PROGRESS_INTERVAL`]; this bounds how stale a fast transfer can look
/// without emitting once per chunk.
const PROGRESS_BATCH_CHUNKS: u64 = 16;

/// Emits domain events as Tauri events.
///
/// `emit` only enqueues, so it stays non-blocking on the transfer threads. A
/// task owns the app handle and does the emitting.
pub struct TauriEventSink {
    events: mpsc::UnboundedSender<DomainEvent>,
}

impl TauriEventSink {
    pub fn new(app: AppHandle) -> Self {
        let (events, rx) = mpsc::unbounded_channel();

        tauri::async_runtime::spawn(run(app, rx));

        Self { events }
    }
}

impl EventSink for TauriEventSink {
    fn emit(&self, event: DomainEvent) {
        // The receiver only stops when the app is shutting down.
        let _ = self.events.send(event);
    }
}

/// One transfer's newest sample and how many samples are buffered behind it.
struct Pending {
    progress: TransferProgress,
    samples: u64,
}

/// Holds the newest progress sample for each running transfer.
#[derive(Default)]
struct ProgressBuffer {
    pending: HashMap<String, Pending>,
}

impl ProgressBuffer {
    /// Buffer a sample, returning it for immediate emission when that
    /// transfer's batch is full.
    fn insert(&mut self, progress: TransferProgress) -> Option<TransferProgress> {
        let transfer_id = progress.transfer_id.clone();

        let entry = self.pending.entry(transfer_id.clone()).or_insert_with(|| Pending {
            progress: progress.clone(),
            samples: 0,
        });

        entry.progress = progress;
        entry.samples += 1;

        if entry.samples >= PROGRESS_BATCH_CHUNKS {
            return self.pending.remove(&transfer_id).map(|pending| pending.progress);
        }

        None
    }

    fn take(&mut self) -> Vec<TransferProgress> {
        self.pending
            .drain()
            .map(|(_, pending)| pending.progress)
            .collect()
    }

    fn remove(&mut self, transfer_id: &str) {
        self.pending.remove(transfer_id);
    }
}

async fn run(app: AppHandle, mut rx: mpsc::UnboundedReceiver<DomainEvent>) {
    let mut buffer = ProgressBuffer::default();
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                for progress in buffer.take() {
                    emit(&app, &DomainEvent::TransferProgress { progress });
                }
            }
            event = rx.recv() => {
                let Some(event) = event else { return };

                match event {
                    // Keep the latest sample; the tick sends it, unless a full
                    // batch makes this one due now.
                    DomainEvent::TransferProgress { progress } => {
                        if let Some(progress) = buffer.insert(progress) {
                            emit(&app, &DomainEvent::TransferProgress { progress });
                        }
                    }
                    // A state change is authoritative — drop the buffered
                    // sample for that transfer so it cannot arrive after this.
                    _ => {
                        buffer.remove(event.progress().transfer_id.as_str());
                        emit(&app, &event);
                    }
                }
            }
        }
    }
}

fn emit(app: &AppHandle, event: &DomainEvent) {
    let (name, payload) = wire_payload(event);

    if let Err(error) = app.emit(name, payload) {
        tracing::warn!(error=%error, "failed to emit transfer event");
    }
}

/// The event name and payload the webview receives for a domain event.
///
/// Split out of the emit path so the mapping can be asserted against the
/// golden payload fixtures without a Tauri runtime.
pub fn wire_payload(event: &DomainEvent) -> (&'static str, &TransferProgress) {
    (event.wire_name(), event.progress())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vilsend_core::TransferStatus;

    fn progress(transfer_id: &str, uploaded_bytes: u64) -> TransferProgress {
        TransferProgress {
            transfer_id: transfer_id.into(),
            uploaded_bytes,
            total_bytes: 100,
            percentage: uploaded_bytes as f64,
            speed: 0.0,
            eta: None,
            status: TransferStatus::Uploading,
        }
    }

    #[test]
    fn coalescing_keeps_only_the_latest_sample_per_transfer() {
        let mut buffer = ProgressBuffer::default();

        buffer.insert(progress("t-1", 10));
        buffer.insert(progress("t-2", 20));
        buffer.insert(progress("t-1", 30));

        let mut taken = buffer.take();
        taken.sort_by(|a, b| a.transfer_id.cmp(&b.transfer_id));

        assert_eq!(taken.len(), 2, "one sample per transfer");
        assert_eq!(taken[0].transfer_id, "t-1");
        assert_eq!(taken[0].uploaded_bytes, 30, "the newest sample wins");
        assert_eq!(taken[1].uploaded_bytes, 20);

        assert!(buffer.take().is_empty(), "taking drains the buffer");
    }

    #[test]
    fn removing_a_transfer_drops_its_buffered_sample() {
        let mut buffer = ProgressBuffer::default();

        buffer.insert(progress("t-1", 10));

        buffer.remove("t-1");

        assert!(buffer.take().is_empty());
    }

    #[test]
    fn a_full_batch_flushes_the_newest_sample_immediately() {
        let mut buffer = ProgressBuffer::default();

        for uploaded in 0..15 {
            assert!(
                buffer.insert(progress("t-1", uploaded)).is_none(),
                "fewer than 16 samples wait for the tick"
            );
        }

        let flushed = buffer
            .insert(progress("t-1", 15))
            .expect("the sixteenth sample makes the batch due");

        assert_eq!(flushed.uploaded_bytes, 15, "the newest sample is the one sent");
        assert!(buffer.take().is_empty(), "the flush drained that transfer");
    }

    #[test]
    fn batches_are_per_transfer() {
        let mut buffer = ProgressBuffer::default();

        for uploaded in 0..15 {
            buffer.insert(progress("t-1", uploaded));
            buffer.insert(progress("t-2", uploaded));
        }

        assert!(
            buffer.insert(progress("t-2", 15)).is_some(),
            "t-2 filled its own batch"
        );
        assert!(
            buffer.insert(progress("t-1", 15)).is_some(),
            "and so did t-1, independently"
        );
    }
}
