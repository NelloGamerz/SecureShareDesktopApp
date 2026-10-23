//! The list of transports this build knows about.
//!
//! This is the file a new transport edits, and the *only* one: the checklist in
//! `docs/migration/02-transport-layer.md` §6 is "one new module, one feature
//! flag, one line here". Nothing downstream branches on which transports exist,
//! so adding one cannot require a change to the selector, the session layer or
//! the engine.
//!
//! The port already carries the one unavoidable shared edit — the
//! `TransportKind` variant — and it is `#[non_exhaustive]` so that adding one
//! is additive rather than breaking.

use std::sync::Arc;

use vilsend_core::{Transport, TransportKind};

/// The transports a build has installed.
#[derive(Clone, Default)]
pub struct Registry {
    transports: Vec<Arc<dyn Transport>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The transports a default build installs.
    ///
    /// **Empty, and that is correct today.** There is no v2 transport yet: the
    /// tunnel reaches the `ByteStream` framing this port requires by upgrading
    /// to a WebSocket at `GET /v2/session`, and that endpoint does not exist.
    /// Until it does, a transfer runs on the frozen v1 session — see
    /// `LegacyV1Session` in the desktop shell — and the selector has nothing to
    /// choose between.
    ///
    /// The alternative would be to register a transport that no peer can speak
    /// to, so that the list looks populated. An empty list that means "nothing
    /// is ready" is more honest than a full one that means "nothing works".
    pub fn defaults() -> Self {
        Self::new()
    }

    /// Add a transport.
    ///
    /// Order is preference order *within equal scores*: the selector ranks by
    /// capability score and uses this order only to break ties, so registering a
    /// transport first does not make it win.
    #[must_use]
    pub fn with(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transports.push(transport);
        self
    }

    /// The registered transports.
    pub fn candidates(&self) -> &[Arc<dyn Transport>] {
        &self.transports
    }

    /// Whether anything is registered.
    pub fn is_empty(&self) -> bool {
        self.transports.is_empty()
    }

    /// The kinds registered, in registration order.
    ///
    /// For tests and for a diagnostic log. Deliberately not used by the
    /// selector, which reads capabilities rather than names.
    pub fn kinds(&self) -> Vec<TransportKind> {
        self.transports
            .iter()
            .map(|transport| transport.kind())
            .collect()
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Registry")
            .field("kinds", &self.kinds())
            .finish()
    }
}
