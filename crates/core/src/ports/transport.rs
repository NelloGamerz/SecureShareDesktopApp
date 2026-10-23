//! The transport port (ADR-0004).
//!
//! Today there is exactly one way bytes move: an HTTP `POST` to
//! `{endpoint}/transfer/chunk`, where the endpoint is a Cloudflare tunnel
//! hostname the control plane supplied. There is no transport concept, no
//! capability model, and no way to tell whether a *specific peer* is reachable —
//! "is the tunnel up?" is answered by scraping cloudflared's stderr, which is a
//! process-level signal.
//!
//! This module is the vocabulary that fixes that. It is deliberately *only*
//! vocabulary: types and one trait. The implementations live in
//! `vilsend-transport`, because a transport needs a socket and this crate is
//! held to `serde`, `thiserror`, `async-trait`, `bytes`, `futures-core` and
//! `tracing` alone (ADR-0002 rule 1), asserted by the `cargo tree` lint in CI.
//!
//! # The invariant
//!
//! > **A transport moves bytes and knows nothing about what they mean.**
//!
//! A [`Transport`] implementation may not know what a chunk, a file or a
//! transfer is; may not encrypt, decrypt, hash or verify; may not emit progress;
//! and may not decide retry policy. That is the whole reason the port is
//! byte-oriented rather than `send_chunk(...)`: a chunk-aware transport cannot
//! carry a handshake, and it bakes the protocol into every implementation.
//!
//! # What is deliberately *not* here
//!
//! * **A protocol version.** [`Transport::connect`] hands back a raw
//!   [`DuplexStream`]; the handshake runs *over* it, in the session layer. A
//!   transport that cannot deliver the first frame cannot deliver anything,
//!   which is why "connect returned `Ok`" is a hint and "the first frame
//!   arrived" is the truth.
//! * **The v1 chunk-POST path.** The frozen v1 wire puts the framing in its
//!   HTTP headers (`File-Id`, `Chunk-Index`, `Relative-Path`, …), so a
//!   byte-oriented stream cannot express it and a transport that could would
//!   have learned exactly what the invariant forbids. v1 is a *session*, not a
//!   transport — see `crate::ports::transport` § "Framing" below.
//!
//! [`TransportKind`] is the port's own type and is **not yet** the one
//! `vilsend_sdk::TransportKind` names. Unifying them is the SDK's own stated
//! first Phase 8 task ("Moving the type down to the port, unchanged, is the
//! first thing Phase 8 should do", `crates/sdk/src/progress.rs`), but replacing
//! a type in the SDK's frozen public surface is a semver break that would fail
//! the `public-api` and `semver-checks` CI jobs. It is deferred to its own
//! change with a deliberate version bump rather than smuggled into a refactor.

use async_trait::async_trait;
use bytes::Bytes;
use std::time::Duration;

use crate::{DeviceId, ErrorKind, PeerRef};

/// Which kind of route carried, or would carry, a transfer.
///
/// **Defined here rather than in the SDK**, which is where it currently lives
/// (`vilsend_sdk::TransportKind`). The port owns it because it is the port's
/// vocabulary; see the module docs for why the move is deferred.
///
/// `Memory` is a member even though it dials nothing: it is a genuine
/// in-process transport kind — the SDK's `in_memory` backend is built on one —
/// and including it is what makes the eventual move of the SDK's type down here
/// lossless rather than a renumbering.
///
/// `Lan`, `P2p`, `Relay`, `Bluetooth` and `Usb` are named but not yet produced,
/// which is the same lease `ErrorKind` takes in `crate::error`: naming the
/// vocabulary now is what stops a `match` from becoming the extension point
/// later.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// Both ends in one process, over an in-memory link.
    Memory,
    /// A peer on the same network, found by discovery.
    Lan,
    /// A peer reached through the cloudflared tunnel — today's only path.
    Tunnel,
    /// A peer reached directly, with NAT traversal.
    P2p,
    /// A VilSend-operated relay, when no direct path exists.
    Relay,
    /// Reserved.
    Bluetooth,
    /// Reserved.
    Usb,
}

impl TransportKind {
    /// Whether bytes on this transport never traverse a third party.
    ///
    /// A *declared* property, not a measured one — it is what
    /// [`Capabilities::end_to_end_direct`] is checked against, and the
    /// transport contract suite is where a transport that lies is caught.
    pub fn is_direct(self) -> bool {
        matches!(self, Self::Memory | Self::Lan | Self::P2p)
    }
}

/// How a transport puts bytes on the wire.
///
/// This is the dimension that keeps [`crate::ports::transport::TransportKind`]
/// out of the selection logic. `docs/migration/02-transport-layer.md` §6 states
/// the rule this exists to satisfy: *"if you feel the need to add a branch to
/// the selector, the capability model is missing a dimension — add the
/// dimension instead."*
///
/// `HttpRequestResponse` is declared but not yet produced by anything. It is
/// here so that the frozen v1 chunk-POST path has an honest name if it ever
/// needs one, and so that a transport that can only do request/response is
/// *filtered* rather than special-cased. The v1 path itself is not a transport.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Framing {
    /// A length-delimited byte stream. An iroh QUIC bi-stream, or a WebSocket
    /// over the tunnel. Frames are delimited *inside* the byte stream, never by
    /// the transport's message boundaries — a QUIC stream has none.
    ByteStream,
    /// An opaque `(headers, body)` exchange. Reserved.
    HttpRequestResponse,
}

/// What a transport can do.
///
/// The selector scores on these and must not hardcode knowledge of any specific
/// transport — that is what makes a new transport one new module plus one line
/// in the factory list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// How this transport frames bytes. See [`Framing`].
    pub framing: Framing,
    /// No internet required.
    pub works_offline: bool,
    /// The bytes never traverse a third party.
    pub end_to_end_direct: bool,
    /// A relayed path exists as a fallback *within* this transport.
    pub has_relay_fallback: bool,
    /// This transport performs NAT traversal.
    pub needs_nat_traversal: bool,
    /// Resume from a byte offset is supported.
    pub resumable: bool,
    /// The transport can move arbitrary streams, not just request/response.
    pub streaming: bool,
    /// Order-of-magnitude hint, bytes/sec on a good day. Scoring only.
    pub nominal_throughput: u64,
    /// Typical setup latency, in ms. Used to decide whether racing is worth it.
    pub setup_latency_ms: u32,
    /// The largest single frame this transport will carry.
    ///
    /// Declared rather than assumed because the two transports disagree: a
    /// WebSocket over the tunnel and a QUIC stream have different practical
    /// limits, and the session layer configures its length-delimited codec from
    /// this value. Guessing produces a framing that works on one transport and
    /// silently truncates on the other.
    pub max_frame_bytes: u32,
}

/// What a bounded probe found.
///
/// A probe is a **hint**. It must not perform a handshake, so it can report
/// `Ready` and the subsequent [`Transport::connect`] can still fail — that case
/// is first-class, not an error, which is why the selector treats "the first
/// frame arrived" rather than "connect returned `Ok`" as the truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Ready right now.
    Ready {
        rtt: Duration,
        estimated_throughput: u64,
    },
    /// Could work, but needs a handshake first.
    Possible,
    /// Definitively unavailable.
    Unavailable { reason: UnavailableReason },
}

/// Why a transport is not available.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    /// The peer is absent, or nothing is listening.
    PeerAbsent,
    /// The capability is switched off locally.
    FeatureOff,
    /// There is no network to carry it.
    NoNetwork,
    /// The peer speaks a framing this transport cannot carry.
    Unsupported,
    /// The peer answered, and refused. Carries the reason to show.
    Refused(String),
}

/// The liveness of an established session.
///
/// Three states rather than a boolean because "alive" and "moving" are
/// different questions. The failure this exists to catch is the one nothing
/// reports today: the process is alive, the socket is open, and no bytes are
/// moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// The pipe is live.
    Alive { rtt: Duration },
    /// The pipe is live but has not made progress for this long.
    Degraded { since: Duration },
    /// The pipe is gone.
    Dead,
}

/// One way of addressing a peer.
///
/// Keyed by the transport that understands it, so the selector never has to
/// look inside an address it cannot interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAddress {
    /// The transport that can reach `address`.
    pub kind: TransportKind,
    /// The transport's own spelling of the address: a hostname, an endpoint id,
    /// a `host:port`. Opaque to everything but that transport.
    pub address: String,
}

/// The other end of a transfer, and every way we know to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// How this side refers to the peer.
    pub reference: PeerRef,
    /// The peer's installation id, once the control plane has told us.
    pub device_id: Option<DeviceId>,
    /// Every address the control plane supplied. A transport reads its own.
    ///
    /// A list rather than one field because the whole point is that there is
    /// more than one way to reach the same peer, and which ones exist is
    /// discovered per transfer.
    pub addresses: Vec<PeerAddress>,
}

impl Peer {
    /// The address for one transport, if the control plane supplied one.
    ///
    /// First match wins: a later duplicate of the same kind is not merged, so
    /// the ordering of [`Peer::addresses`] is the preference order.
    pub fn address(&self, kind: TransportKind) -> Option<&str> {
        self.addresses
            .iter()
            .find(|candidate| candidate.kind == kind)
            .map(|candidate| candidate.address.as_str())
    }
}

/// Identifies one transfer's session on both sides of it.
///
/// Distinct from a [`crate::TransferId`] on purpose: a failover reconnects the
/// *same* session over a *different* transport, and it is the session id that
/// lets the receiver recognise the reconnect, find the key it already derived,
/// and answer with the chunks it already holds rather than restarting.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<String> for SessionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SessionId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// The credential a receiver validates before it accepts a session.
///
/// Opaque today: the shell fills it with the bearer the control plane supplied
/// for the transfer, which is what the v1 wire already carries. It is a
/// distinct type rather than a `String` so that the day it becomes an
/// HMAC-bound, expiring token (ADR-0009) the port's shape does not change and
/// neither does any transport.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The token as the wire carries it.
    ///
    /// Named `expose` rather than `as_str` so that a call site reads as a
    /// deliberate disclosure of a secret rather than a routine borrow.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether this token carries nothing.
    ///
    /// A receiver must reject an empty token rather than accept it, and this is
    /// where a caller asks. See the note on
    /// [`crate::ports::transport::SessionToken`] in the stage 1a report: the v1
    /// receiver today checks only that the `Authorization` header is *present*.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Hand-written so a token cannot reach a log through a derived `Debug`.
impl std::fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            formatter.write_str("SessionToken(<empty>)")
        } else {
            formatter.write_str("SessionToken(<redacted>)")
        }
    }
}

impl From<String> for SessionToken {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SessionToken {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A transport-level failure.
///
/// Every variant has its own [`ErrorKind`], because a shell branches on the
/// kind and never parses a message (ADR-0003). The mapping is asserted by a
/// test rather than left to review.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The peer could not be reached at all.
    #[error("peer unreachable")]
    Unreachable,

    /// The operation did not finish in time.
    #[error("timed out after {0:?}")]
    Timeout(Duration),

    /// The peer is reachable but refused the connection.
    #[error("connection refused")]
    Refused,

    /// The pipe was live and is not any more.
    #[error("stream closed")]
    Closed,

    /// The peer offers a framing this transport cannot carry.
    ///
    /// Distinct from [`TransportError::Protocol`]: this is a *capability*
    /// mismatch, decided before any bytes moved, and the caller's remedy is to
    /// pick a different transport rather than to retry.
    #[error("transport cannot carry {0:?}")]
    Unsupported(Framing),

    /// The peer said something that does not parse as the protocol.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The underlying socket or stream failed.
    #[error("io error: {0}")]
    Io(String),
}

impl TransportError {
    /// The stable classification of this error.
    ///
    /// `Unreachable` and `Refused` are [`ErrorKind::NoRoute`] because both mean
    /// the same thing to a caller — there is no way to this peer, try another
    /// route — whereas every mid-stream failure is [`ErrorKind::Network`],
    /// because there *was* a route and it broke. That distinction is the one the
    /// failover policy acts on, so it must not be flattened.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Unreachable | Self::Refused => ErrorKind::NoRoute,
            Self::Timeout(_) | Self::Closed | Self::Io(_) => ErrorKind::Network,
            Self::Unsupported(_) => ErrorKind::InvalidInput,
            Self::Protocol(_) => ErrorKind::Serialization,
        }
    }
}

/*
 * There is deliberately **no** `From<TransportError> for VilsendError`.
 *
 * It was written, and it broke the SDK: `vilsend_sdk::VilsendError` is this
 * crate's `VilsendError`, re-exported (`crates/sdk/src/lib.rs:46`), so a
 * conversion from `TransportError` gave every `?` that already targeted
 * `VilsendError` a *second* candidate source. Rust then cannot infer the error
 * type, and `crates/sdk/examples/send.rs` stopped compiling with
 * `E0283: multiple impls satisfying VilsendError: From<_>` — in a file this
 * change never touched.
 *
 * That is the general hazard of adding a conversion to a shared boundary type:
 * it is not additive for anyone writing `?`. The classification is therefore
 * reached through [`TransportError::kind`], which is explicit, has no inference
 * cost, and is what ADR-0003 actually requires — a shell branches on the kind
 * and never parses a message. A caller that wants a `VilsendError` says which
 * variant it wants, which is one line and is not ambiguous.
 */

/// The send half of a [`DuplexStream`].
///
/// Takes `&self` rather than `&mut self` because the sender is shared: the
/// transfer scheduler runs several workers concurrently, and a transport whose
/// writer needed exclusive access would put a lock across every chunk. An
/// implementation serialises internally — the natural shape is one writer task
/// draining a bounded channel, which is also what gives `send` its backpressure.
#[async_trait]
pub trait FrameSender: Send + Sync + 'static {
    /// Put one frame on the wire.
    async fn send(&self, frame: Bytes) -> Result<(), TransportError>;

    /// Close the stream.
    ///
    /// Idempotent: closing twice is not an error, because a failover and a
    /// user-initiated cancel can race to close the same session.
    async fn close(&self) -> Result<(), TransportError>;
}

/// The receive half of a [`DuplexStream`].
///
/// Takes `&mut self`, and is owned rather than shared, because a session has
/// exactly one reader. Sharing it would put an `Arc<Mutex<_>>` on the inbound
/// path for no benefit.
///
/// `Ok(None)` means the peer closed the stream cleanly, which is different from
/// `Err(TransportError::Closed)` — the first is an ending, the second is a
/// break.
#[async_trait]
pub trait FrameReceiver: Send + 'static {
    /// Take the next frame, or `None` when the peer closed the stream.
    async fn recv(&mut self) -> Result<Option<Bytes>, TransportError>;
}

/// A byte-oriented duplex stream, and the narrow waist of the whole design.
///
/// Everything above this line is protocol — framing, chunking, resume,
/// encryption. Everything below is a transport. Neither knows about the other,
/// which is what lets a new transport be added without touching the engine.
///
/// A concrete struct rather than a trait, so `Box<dyn Transport>` stays
/// object-safe and there is one virtual call per frame rather than per byte.
pub struct DuplexStream {
    sender: Box<dyn FrameSender>,
    /*
     * `Option` because a stream has one reader and taking it is how that is
     * enforced. Behind a `std::sync::Mutex` rather than a tokio one because the
     * critical section is a move with no await in it — taking the receiver is a
     * cold path, once per session.
     */
    receiver: std::sync::Mutex<Option<Box<dyn FrameReceiver>>>,
}

impl DuplexStream {
    /// Build a stream from its two halves.
    pub fn new(sender: Box<dyn FrameSender>, receiver: Box<dyn FrameReceiver>) -> Self {
        Self {
            sender,
            receiver: std::sync::Mutex::new(Some(receiver)),
        }
    }

    /// Put one frame on the wire.
    pub async fn send(&self, frame: Bytes) -> Result<(), TransportError> {
        self.sender.send(frame).await
    }

    /// Take ownership of the receiving half.
    ///
    /// Exactly once. A second call is [`TransportError::Protocol`], not a
    /// panic: two readers on one stream would split the frame sequence between
    /// them, which corrupts a transfer in a way that looks like data loss, so
    /// the mistake has to be refused where it is made.
    pub fn take_receiver(&mut self) -> Result<Box<dyn FrameReceiver>, TransportError> {
        self.receiver
            .lock()
            .expect("duplex receiver lock poisoned")
            .take()
            .ok_or_else(|| {
                TransportError::Protocol("the receiver half has already been taken".into())
            })
    }

    /// Close the stream.
    pub async fn close(&self) -> Result<(), TransportError> {
        self.sender.close().await
    }
}

impl std::fmt::Debug for DuplexStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DuplexStream")
            .field(
                "receiver_taken",
                &self.receiver.lock().map(|r| r.is_none()).unwrap_or(false),
            )
            .finish_non_exhaustive()
    }
}

/// One way of moving bytes to a peer.
///
/// Object-safe, so the selector holds `Arc<dyn Transport>` and a new transport
/// is registered rather than branched on.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Which kind this is.
    fn kind(&self) -> TransportKind;

    /// What this transport can do. See [`Capabilities`].
    fn capabilities(&self) -> Capabilities;

    /// A fast availability probe.
    ///
    /// **Must complete within `budget` and must not perform a full handshake.**
    /// Called concurrently for every candidate, so a slow probe on one transport
    /// delays the decision for all of them.
    async fn probe(&self, peer: &Peer, budget: Duration) -> ProbeOutcome;

    /// Establish the channel.
    ///
    /// Safe to call concurrently with another transport's `connect` while
    /// racing; a loser must be cleanly abandonable through [`Transport::teardown`].
    ///
    /// Returns the **raw** stream. No handshake has happened, and none may: the
    /// session layer owns the handshake, because a transport must not know what
    /// a protocol version is.
    async fn connect(
        &self,
        peer: &Peer,
        session: &SessionId,
        token: &SessionToken,
    ) -> Result<DuplexStream, TransportError>;

    /// A cheap liveness check, used by the health monitor.
    async fn health(&self, session: &SessionId) -> Health;

    /// Release everything held for a session: a listener, a socket, a lease.
    ///
    /// Idempotent, because a failover, a cancel and a shutdown can all reach it.
    async fn teardown(&self, session: &SessionId) -> Result<(), TransportError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /*
     * The port's tests are driven by hand rather than by an async runtime.
     *
     * This is not a workaround, it is the point: `vilsend-core` may depend only
     * on `serde`, `thiserror`, `async-trait` and `bytes` (ADR-0002 rule 1), so a
     * `#[tokio::test]` here would mean widening the one edge the architecture
     * lint exists to keep narrow — and the SDK, which embeds this crate, is
     * meant to be runtime-agnostic.
     *
     * `std::task::Wake` is what makes this possible without `unsafe`: a no-op
     * waker is a type and an `Arc`, not a hand-built `RawWaker`.
     *
     * The fakes below never yield, so a single poll resolves them. A future that
     * returned `Pending` would hand back nothing, which is why that case panics
     * with a message saying so rather than looping.
     */
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        /// Does nothing, which is correct: the futures under test never park.
        struct Noop;

        impl Wake for Noop {
            fn wake(self: Arc<Self>) {}
        }

        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);

        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("the future under test did not resolve on the first poll"),
        }
    }

    /// A sender that records what it was given, so the stream's forwarding can
    /// be asserted without a socket.
    #[derive(Clone, Default)]
    struct RecordingSender {
        sent: Arc<Mutex<Vec<Bytes>>>,
        closed: Arc<Mutex<bool>>,
    }

    #[async_trait]
    impl FrameSender for RecordingSender {
        async fn send(&self, frame: Bytes) -> Result<(), TransportError> {
            self.sent.lock().unwrap().push(frame);
            Ok(())
        }

        async fn close(&self) -> Result<(), TransportError> {
            *self.closed.lock().unwrap() = true;
            Ok(())
        }
    }

    struct ScriptedReceiver(Vec<Bytes>);

    #[async_trait]
    impl FrameReceiver for ScriptedReceiver {
        async fn recv(&mut self) -> Result<Option<Bytes>, TransportError> {
            Ok(self.0.pop())
        }
    }

    fn peer_with(addresses: Vec<PeerAddress>) -> Peer {
        Peer {
            reference: PeerRef::new("p-1"),
            device_id: Some(DeviceId::new("d-1")),
            addresses,
        }
    }

    #[test]
    fn a_peers_address_is_found_by_the_transport_that_understands_it() {
        let peer = peer_with(vec![
            PeerAddress {
                kind: TransportKind::Tunnel,
                address: "example.trycloudflare.com".into(),
            },
            PeerAddress {
                kind: TransportKind::P2p,
                address: "endpoint-id".into(),
            },
        ]);

        assert_eq!(
            peer.address(TransportKind::Tunnel),
            Some("example.trycloudflare.com")
        );
        assert_eq!(peer.address(TransportKind::P2p), Some("endpoint-id"));
        // A kind the control plane said nothing about is absent, not empty.
        assert_eq!(peer.address(TransportKind::Lan), None);
        assert_eq!(peer.address(TransportKind::Relay), None);
    }

    #[test]
    fn the_first_address_of_a_kind_wins() {
        let peer = peer_with(vec![
            PeerAddress {
                kind: TransportKind::Tunnel,
                address: "first".into(),
            },
            PeerAddress {
                kind: TransportKind::Tunnel,
                address: "second".into(),
            },
        ]);

        assert_eq!(peer.address(TransportKind::Tunnel), Some("first"));
    }

    /// The classification is the only thing a shell branches on, so every
    /// variant must have one and the mapping must be deliberate. The
    /// `NoRoute`/`Network` split is load-bearing: failover acts on it, so a
    /// mid-stream break must not be flattened into "unreachable".
    #[test]
    fn every_transport_error_maps_to_its_own_kind() {
        let cases: [(TransportError, ErrorKind); 7] = [
            (TransportError::Unreachable, ErrorKind::NoRoute),
            (TransportError::Refused, ErrorKind::NoRoute),
            (
                TransportError::Timeout(Duration::from_secs(1)),
                ErrorKind::Network,
            ),
            (TransportError::Closed, ErrorKind::Network),
            (TransportError::Io("reset".into()), ErrorKind::Network),
            (
                TransportError::Unsupported(Framing::ByteStream),
                ErrorKind::InvalidInput,
            ),
            (
                TransportError::Protocol("bad frame".into()),
                ErrorKind::Serialization,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.kind(), expected, "wrong kind for {error:?}");
        }
    }

    /// There is no `From<TransportError> for VilsendError`, and this is the
    /// guard on that decision rather than a test of it.
    ///
    /// Adding one broke `crates/sdk/examples/send.rs` with `E0283`, because
    /// `vilsend_sdk::VilsendError` is this crate's type re-exported, so a second
    /// conversion source made `?` unable to infer. A trait-coherence check
    /// cannot be written without the impl existing to check, so what is pinned
    /// is the thing that is actually required: every variant classifies, via
    /// `kind()`, with no inference involved.
    #[test]
    fn every_transport_failure_can_be_classified_without_a_conversion() {
        let cases = [
            TransportError::Unreachable,
            TransportError::Refused,
            TransportError::Timeout(Duration::from_secs(2)),
            TransportError::Closed,
            TransportError::Io("reset".into()),
            TransportError::Unsupported(Framing::HttpRequestResponse),
            TransportError::Protocol("bad frame".into()),
        ];

        let kinds: Vec<crate::ErrorKind> = cases.iter().map(TransportError::kind).collect();

        assert_eq!(
            kinds,
            vec![
                crate::ErrorKind::NoRoute,
                crate::ErrorKind::NoRoute,
                crate::ErrorKind::Network,
                crate::ErrorKind::Network,
                crate::ErrorKind::Network,
                crate::ErrorKind::InvalidInput,
                crate::ErrorKind::Serialization,
            ]
        );
    }

    #[test]
    fn a_stream_forwards_a_frame_to_its_sender() {
        let sender = RecordingSender::default();
        let recorded = sender.clone();

        let stream = DuplexStream::new(Box::new(sender), Box::new(ScriptedReceiver(vec![])));

        block_on(stream.send(Bytes::from_static(b"one"))).unwrap();

        assert_eq!(recorded.sent.lock().unwrap().len(), 1);
        assert_eq!(&recorded.sent.lock().unwrap()[0][..], b"one");
    }

    #[test]
    fn closing_a_stream_closes_its_sender() {
        let sender = RecordingSender::default();
        let closed = sender.closed.clone();

        let stream = DuplexStream::new(Box::new(sender), Box::new(ScriptedReceiver(vec![])));

        block_on(stream.close()).unwrap();

        assert!(*closed.lock().unwrap());
    }

    #[test]
    fn a_stream_hands_its_receiver_over_exactly_once() {
        let mut stream = DuplexStream::new(
            Box::new(RecordingSender::default()),
            Box::new(ScriptedReceiver(vec![Bytes::from_static(b"frame")])),
        );

        let mut receiver = stream.take_receiver().expect("the first take succeeds");

        assert_eq!(
            block_on(receiver.recv()).unwrap(),
            Some(Bytes::from_static(b"frame"))
        );

        // A second reader would split the frame sequence with the first, which
        // corrupts a transfer in a way that looks like data loss — so it is
        // refused where the mistake is made, rather than at the far end.
        let second = stream.take_receiver();

        assert!(
            matches!(second, Err(TransportError::Protocol(_))),
            "a second take must be refused, not panicked or silently allowed"
        );
    }

    /// A token must not be readable out of a log line. The derived `Debug` on a
    /// newtype over `String` would print it, so it is written by hand.
    #[test]
    fn a_session_token_is_redacted_in_debug_output() {
        let token = SessionToken::new("super-secret-bearer");

        let rendered = format!("{token:?}");

        assert!(!rendered.contains("super-secret-bearer"));
        assert_eq!(rendered, "SessionToken(<redacted>)");

        assert_eq!(
            format!("{:?}", SessionToken::new("")),
            "SessionToken(<empty>)"
        );

        // And the value is still there when a caller deliberately asks.
        assert_eq!(token.expose(), "super-secret-bearer");
        assert!(!token.is_empty());
        assert!(SessionToken::new("").is_empty());
    }

    #[test]
    fn a_session_id_round_trips_as_its_text() {
        let id = SessionId::new("t-1");

        assert_eq!(id.as_str(), "t-1");
        assert_eq!(id.to_string(), "t-1");
        assert_eq!(SessionId::from("t-1"), id);
        assert_eq!(SessionId::from(String::from("t-1")), id);
    }

    #[test]
    fn directness_is_declared_per_kind() {
        assert!(TransportKind::Lan.is_direct());
        assert!(TransportKind::P2p.is_direct());
        assert!(TransportKind::Memory.is_direct());

        // The tunnel and a relay both put a third party in the path.
        assert!(!TransportKind::Tunnel.is_direct());
        assert!(!TransportKind::Relay.is_direct());
    }
}
