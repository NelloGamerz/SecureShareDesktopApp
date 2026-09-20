//! The desktop implementation of the [`EventSink`] port (ADR-0012).
//!
//! This is the whole of the desktop shell's side of the walking skeleton: a
//! domain event becomes a Tauri event with the same name and the same payload
//! it had before the port existed.

use tauri::{AppHandle, Emitter};
use vilsend_core::{DomainEvent, EventSink, TransferProgress};

/// Emits domain events as Tauri events.
pub struct TauriEventSink {
    app: AppHandle,
}

impl TauriEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl EventSink for TauriEventSink {
    fn emit(&self, event: DomainEvent) {
        let (name, payload) = wire_payload(&event);

        if let Err(error) = self.app.emit(name, payload) {
            tracing::warn!(error=%error, "failed to emit transfer event");
        }
    }
}

/// The event name and payload the webview receives for a domain event.
///
/// Split out of [`TauriEventSink::emit`] so the mapping can be asserted
/// against the golden payload fixtures without a Tauri runtime.
pub fn wire_payload(event: &DomainEvent) -> (&'static str, &TransferProgress) {
    (event.wire_name(), event.progress())
}
