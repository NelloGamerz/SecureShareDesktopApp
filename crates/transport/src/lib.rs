//! Transport implementations for VilSend.
//!
//! The port itself is in `vilsend-core` ([`vilsend_core::Transport`]); this
//! crate holds the things that satisfy it. The split is ADR-0002's: the domain
//! owns the interface and depends on nothing, and an adapter that needs a socket
//! depends on the domain.
//!
//! # What is here today
//!
//! [`mock::MockTransport`], which joins two halves of a
//! [`vilsend_core::DuplexStream`] through a channel and touches no network, plus
//! [`TransportSelector`] and the [`Registry`] that feeds it.
//!
//! # What is not, and why
//!
//! **`TunnelTransport`.** The port requires a length-delimited byte stream, and
//! the tunnel is request/response. The way through is a WebSocket upgrade at
//! `GET /v2/session`, which the receiver does not serve yet — that endpoint and
//! the session framing above it are the next stage, and the tunnel transport
//! arrives with them rather than before them.
//!
//! **`P2pTransport`.** Behind a `p2p` feature when it lands, over `iroh`.
//!
//! Registering either is one line in [`Registry::defaults`]. That is the whole
//! point of the port, and `tests/new_transport.rs` is the test that keeps it
//! true.

pub mod mock;
pub mod registry;
pub mod selector;

pub use mock::MockTransport;
pub use registry::Registry;
pub use selector::{score, Policy, SelectError, Selected, TransportSelector};
