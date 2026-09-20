//! The stable public API for VilSend. The crate's own documentation is its
//! README, so that the example in it is compiled and run by
//! `cargo test -p vilsend-sdk --doc` and cannot rot (`05-migration-plan.md`,
//! task 5.6).
#![doc = include_str!("../README.md")]
// The README is written for a person reading it on crates.io or in an editor;
// the doctest inside it is compiled, which is the point. Everything else here
// is ordinary library documentation.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod auth;
mod backend;
mod clock;
mod events;
mod files;
mod handle;
mod ids;
mod progress;
mod request;
mod state;

use std::fmt;
use std::sync::Arc;

use vilsend_core::EventSink;

use backend::memory::MemoryBackend;
use backend::Backend;
use events::EventBus;

pub use auth::{AuthFacade, AuthState};
pub use clock::{Clock, FakeClock};
pub use events::EventStream;
pub use files::MemoryFiles;
pub use handle::{Outcome, ReceivedFile, ReceivedOutcome, SentOutcome, TransferHandle};
pub use ids::{Destination, FileRef};
pub use progress::{DegradedReason, Progress, TransportKind};
pub use request::{Policy, ReceiveRequest, SendRequest};

// The domain vocabulary a caller needs and should not have to add a second
// dependency to reach (`01-target-architecture.md` §4.1: the SDK "re-exports a
// curated subset"). These are `vilsend-core`'s types, not copies of them: a
// shell that matches on an error the SDK returned is matching on the same enum
// the core produced.
pub use vilsend_core::{DomainEvent, ErrorKind, PeerRef, TransferId, TransferStatus, VilsendError};

/// The [`PeerRef`] the in-memory link answers to.
///
/// A convention rather than a requirement — the link keys listeners by whatever
/// peer the receiving side named — but naming it means an example reads as an
/// example rather than as a magic string.
pub const LOOPBACK_PEER: &str = "memory";

/// A configured VilSend client.
///
/// Built by [`VilsendBuilder`], and deliberately opaque: nothing about how it
/// is implemented is part of its contract, so nothing about the implementation
/// can leak into a caller's assumptions.
///
/// # The four verbs
///
/// [`send`](Vilsend::send), [`receive`](Vilsend::receive),
/// [`auth`](Vilsend::auth) and [`events`](Vilsend::events). That is the whole
/// surface, on purpose: `05-migration-plan.md` § "Phase 5 — Risks" warns that
/// "the facade becomes a god-object … If a fifth is needed, it is probably a
/// separate facade."
pub struct Vilsend {
    backend: Box<dyn Backend>,
}

impl fmt::Debug for Vilsend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The backend is not `Debug`, and should not be: a client that renders
        // itself is a client that will one day render a credential.
        formatter.write_str("Vilsend { .. }")
    }
}

impl Vilsend {
    /// Starts sending `request` to its peer.
    ///
    /// The handle this returns is the only way to observe the transfer:
    /// [`TransferHandle::progress`] to look, [`TransferHandle::wait`] to block
    /// until it is over, and dropping it to cancel — see
    /// [`TransferHandle`].
    pub async fn send(&self, request: SendRequest) -> Result<TransferHandle, VilsendError> {
        self.backend.send(request).await
    }

    /// Starts receiving from `request`'s peer into its destination.
    pub async fn receive(&self, request: ReceiveRequest) -> Result<TransferHandle, VilsendError> {
        self.backend.receive(request).await
    }

    /// The authentication surface.
    ///
    /// An object rather than methods on `Vilsend`, so that auth can grow
    /// without widening the object that also owns `send` and `receive`.
    pub async fn auth(&self) -> AuthFacade {
        AuthFacade::new(self.backend.auth_state())
    }

    /// A live feed of domain events.
    ///
    /// Each call returns an independent subscription that sees everything
    /// emitted after it was taken. See [`EventStream`] for the backpressure
    /// contract.
    pub fn events(&self) -> EventStream {
        self.backend.events()
    }
}

/// Builds a [`Vilsend`].
///
/// Two constructors, and the difference between them is the point:
///
/// - [`VilsendBuilder::native`] — real files, the OS keyring, a real transport.
/// - [`VilsendBuilder::in_memory`] — touches nothing external. Not "nothing
///   external in practice"; nothing external by construction, which is a claim
///   `crates/sdk/tests/no_external_touch.rs` mechanically checks.
pub struct VilsendBuilder {
    kind: Kind,
    source: MemoryFiles,
    sink: MemoryFiles,
    clock: Arc<dyn Clock>,
    events: Option<Arc<dyn EventSink>>,
    policy: Policy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Native,
    InMemory,
}

impl fmt::Debug for VilsendBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VilsendBuilder")
            .field("kind", &self.kind)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl Default for VilsendBuilder {
    /// [`VilsendBuilder::native`].
    ///
    /// The default is the real one, not the test one: a builder that defaults
    /// to an in-memory backend is a builder that will ship an in-memory
    /// backend to a user the first time someone forgets to configure it.
    fn default() -> Self {
        Self::native()
    }
}

impl VilsendBuilder {
    /// A client backed by real files, the OS keyring and a real transport.
    ///
    /// **This does not build yet.** `01-target-architecture.md` §4.1 puts the
    /// native adapters in `vilsend-runtime` and the engine in
    /// `vilsend-engine`, and neither crate exists: Phase 3 ("Resource ports")
    /// has not run. Until it has, [`build`](VilsendBuilder::build) refuses,
    /// with an error that says so, rather than handing back a client whose
    /// every call fails — see `docs/migration/reports/phase-5-report.md`.
    pub fn native() -> Self {
        Self {
            kind: Kind::Native,
            source: MemoryFiles::new(),
            sink: MemoryFiles::new(),
            clock: Arc::new(FakeClock::new()),
            events: None,
            policy: Policy::default(),
        }
    }

    /// A client that touches nothing external.
    ///
    /// No filesystem, no keychain, no network, no real clock. Both halves of a
    /// transfer run in this process over an in-memory link: `receive` registers
    /// a listener, and `send` to [`LOOPBACK_PEER`] delivers to the earliest one
    /// waiting. See [`Vilsend`]'s module documentation in `backend::memory`.
    pub fn in_memory() -> Self {
        Self {
            kind: Kind::InMemory,
            source: MemoryFiles::new(),
            sink: MemoryFiles::new(),
            clock: Arc::new(FakeClock::new()),
            events: None,
            policy: Policy::default(),
        }
    }

    /// The store `SendRequest::files` are resolved against.
    ///
    /// Cloned by value, and the clone shares: a caller that keeps one and
    /// passes another sees the same bytes.
    pub fn with_memory_source(mut self, source: MemoryFiles) -> Self {
        self.source = source;
        self
    }

    /// The store received files are written to.
    pub fn with_memory_sink(mut self, sink: MemoryFiles) -> Self {
        self.sink = sink;
        self
    }

    /// The time source throughput and ETA are measured against.
    ///
    /// [`FakeClock`] by default, so that a transfer is deterministic unless a
    /// caller asks otherwise. A native client will default to the system clock;
    /// this one may not.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// A sink to receive every event in addition to the streams from
    /// [`Vilsend::events`].
    ///
    /// The port is `vilsend_core::EventSink` (ADR-0012), so a shell that
    /// already implements it — the desktop app's `TauriEventSink` — needs no
    /// new vocabulary to be fed by the SDK.
    pub fn with_event_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.events = Some(sink);
        self
    }

    /// The policy transfers run under, unless a request overrides it.
    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`](vilsend_core::ErrorKind::Internal) when
    /// [`native`](VilsendBuilder::native) was chosen, because the crates that
    /// backend needs do not exist yet. [`ErrorKind::InvalidInput`] when the
    /// policy cannot be honoured.
    ///
    /// [`vilsend_core::ErrorKind::InvalidInput`]: vilsend_core::ErrorKind::InvalidInput
    pub fn build(self) -> Result<Vilsend, VilsendError> {
        self.policy.validate()?;

        let backend: Box<dyn Backend> = match self.kind {
            Kind::InMemory => Box::new(MemoryBackend::new(
                self.source,
                self.sink,
                self.clock,
                EventBus::new(self.events),
                self.policy,
            )),

            // Not a placeholder that will be filled in "later in this file".
            // It is the honest state of the world: the native adapters are
            // Phase 3's `vilsend-runtime` and the engine is Phase 3's
            // `vilsend-engine`, and building a client that pretends to be
            // native would turn a missing crate into a runtime surprise three
            // call sites later.
            Kind::Native => {
                return Err(VilsendError::Internal(
                    "the native backend is not available in this build: it needs the \
                     file, credential and transport adapters from `vilsend-runtime` \
                     and the engine from `vilsend-engine` (Phase 3 of \
                     docs/migration/05-migration-plan.md). Use \
                     `VilsendBuilder::in_memory()` for now."
                        .into(),
                ))
            }
        };

        Ok(Vilsend { backend })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_builder_is_the_native_one() {
        // A builder that defaulted to the in-memory backend would ship one to a
        // user the first time someone forgot to configure it.
        assert_eq!(VilsendBuilder::default().kind, Kind::Native);
        assert_eq!(VilsendBuilder::native().kind, Kind::Native);
        assert_eq!(VilsendBuilder::in_memory().kind, Kind::InMemory);
    }

    #[test]
    fn the_native_builder_refuses_to_build_and_says_why() {
        let error = VilsendBuilder::native()
            .build()
            .expect_err("there is no native backend yet");

        assert_eq!(error.kind(), ErrorKind::Internal);

        let message = error.to_string();

        assert!(message.contains("vilsend-runtime"), "{message}");
        assert!(message.contains("vilsend-engine"), "{message}");
        assert!(message.contains("in_memory()"), "{message}");
    }

    #[test]
    fn every_builder_setter_is_chainable_and_keeps_what_it_was_given() {
        let policy = Policy {
            chunk_size: 512,
            ..Policy::default()
        };

        let builder = VilsendBuilder::in_memory()
            .with_memory_source(MemoryFiles::new())
            .with_memory_sink(MemoryFiles::new())
            .with_clock(Arc::new(FakeClock::new()))
            .with_policy(policy.clone());

        assert_eq!(builder.policy, policy);
    }

    #[test]
    fn a_policy_that_cannot_be_honoured_fails_the_build() {
        let error = VilsendBuilder::in_memory()
            .with_policy(Policy {
                chunk_size: 0,
                ..Policy::default()
            })
            .build()
            .expect_err("a zero chunk size cannot be honoured");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn the_in_memory_builder_builds() {
        assert!(VilsendBuilder::in_memory().build().is_ok());
    }

    #[test]
    fn a_client_renders_opaque() {
        // A backend that rendered itself is a backend that will one day render
        // a credential.
        let client = VilsendBuilder::in_memory().build().expect("builds");

        assert_eq!(format!("{client:?}"), "Vilsend { .. }");
    }

    #[test]
    fn the_loopback_peer_is_the_name_the_memory_backend_answers_to() {
        assert_eq!(LOOPBACK_PEER, "memory");
    }
}
