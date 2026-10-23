//! An in-memory transport: two halves of one [`DuplexStream`], joined by a
//! channel, with no socket anywhere.
//!
//! It exists for two reasons.
//!
//! **It is the contract suite's subject.** The nine tests in
//! `docs/migration/02-transport-layer.md` §6.1 describe what *any* transport
//! must do. Running them against a real socket first would mean debugging the
//! abstraction and the socket at once, which is exactly what
//! `05-migration-plan.md` Phase 8 warns against. The in-memory transport makes
//! the suite runnable today, and the same suite is what the tunnel must pass
//! unmodified when it arrives.
//!
//! **It is the proof that a new transport is one registration.** Registering it
//! requires no edit to the selector, the registry's shape or the port — see
//! `tests/new_transport.rs`, which is the test that would fail if that stopped
//! being true.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::mpsc;
use vilsend_core::{
    Capabilities, DuplexStream, FrameReceiver, FrameSender, Framing, Health, Peer, ProbeOutcome,
    SessionId, SessionToken, Transport, TransportError, TransportKind, UnavailableReason,
};

/// How many frames may be queued in either direction before `send` waits.
///
/// Bounded on purpose, and it is the whole reason `send` can be `&self` without
/// a lock: backpressure comes from awaiting a full queue rather than from
/// excluding other senders. An unbounded channel would let a stalled peer grow
/// the sender's memory without limit, which on a 4 MiB chunk is the difference
/// between a slow transfer and a dead app.
const QUEUE_DEPTH: usize = 8;

/// An in-memory transport.
///
/// Cloneable, and every clone shares one peer: the knobs below describe the
/// *far end*, not the transport object, which is how a test takes the peer down
/// while a stream is still open.
#[derive(Clone)]
pub struct MockTransport {
    inner: Arc<Inner>,
}

struct Inner {
    kind: TransportKind,
    capabilities: Capabilities,
    /// What the next probe reports, before the availability checks below.
    probe_outcome: Mutex<ProbeOutcome>,
    /// How long a probe pretends to take. Used to test that a budget is obeyed.
    probe_delay: Mutex<Duration>,
    /// The peer is reachable and refuses. Distinct from being down: a refusal is
    /// an answer, and the port classifies the two the same way on purpose.
    refuses: AtomicBool,
    /// Whether the peer is up at all.
    up: AtomicBool,
    /// Sessions with a live stream.
    sessions: Mutex<HashSet<String>>,
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTransport {
    /// A direct, in-memory link — the `Memory` kind.
    pub fn new() -> Self {
        Self::with_kind(TransportKind::Memory)
    }

    /// A transport of any kind, with capabilities that are coherent for it.
    ///
    /// Built from [`coherent_capabilities`] rather than typed out per call site
    /// so that a test cannot accidentally describe a tunnel as end-to-end
    /// direct and then assert against its own mistake.
    pub fn with_kind(kind: TransportKind) -> Self {
        Self {
            inner: Arc::new(Inner {
                kind,
                capabilities: coherent_capabilities(kind),
                probe_outcome: Mutex::new(ProbeOutcome::Ready {
                    rtt: Duration::from_millis(1),
                    estimated_throughput: 1_000_000_000,
                }),
                probe_delay: Mutex::new(Duration::ZERO),
                refuses: AtomicBool::new(false),
                up: AtomicBool::new(true),
                sessions: Mutex::new(HashSet::new()),
            }),
        }
    }

    /// A peer that answers and refuses every session.
    pub fn refusing() -> Self {
        let transport = Self::new();
        transport.inner.refuses.store(true, Ordering::Relaxed);
        transport
    }

    /// A peer that is not there at all.
    pub fn down() -> Self {
        let transport = Self::new();
        transport.inner.up.store(false, Ordering::Relaxed);
        transport
    }

    /// Make probes take this long.
    #[must_use]
    pub fn with_probe_delay(self, delay: Duration) -> Self {
        *self.inner.probe_delay.lock().expect("probe delay poisoned") = delay;
        self
    }

    /// Report `Possible` instead of `Ready`, as a transport that needs a
    /// handshake before it can promise anything.
    #[must_use]
    pub fn needing_a_handshake(self) -> Self {
        *self
            .inner
            .probe_outcome
            .lock()
            .expect("probe outcome poisoned") = ProbeOutcome::Possible;
        self
    }

    /// Take the peer away, as a network drop or a peer shutdown would.
    ///
    /// Distinct from [`Transport::teardown`], which releases *this* side's
    /// resources. This is the far end disappearing, which is the case
    /// `health` and the stall detector exist for.
    pub fn peer_goes_away(&self) {
        self.inner.up.store(false, Ordering::Relaxed);
    }

    /// How many sessions this transport believes are live.
    pub fn live_sessions(&self) -> usize {
        self.inner.sessions.lock().expect("sessions poisoned").len()
    }
}

/// Capabilities that are coherent for a kind of transport.
///
/// This is the table the contract suite's meta-test checks a transport against.
/// It exists so that declaring `end_to_end_direct: true` on a transport that
/// traverses a relay is a *test failure* rather than a comment nobody reads —
/// the most likely lie a new implementation tells, per
/// `docs/migration/02-transport-layer.md` §6.1.
pub fn coherent_capabilities(kind: TransportKind) -> Capabilities {
    let direct = kind.is_direct();

    Capabilities {
        framing: Framing::ByteStream,
        works_offline: direct,
        end_to_end_direct: direct,
        has_relay_fallback: matches!(kind, TransportKind::P2p),
        needs_nat_traversal: matches!(kind, TransportKind::P2p),
        resumable: true,
        streaming: true,
        nominal_throughput: if direct { 1_000_000_000 } else { 40_000_000 },
        setup_latency_ms: if direct { 1 } else { 200 },
        max_frame_bytes: 8 * 1024 * 1024,
    }
}

/// The sending half. Holds the channel to the echoing peer.
struct MockSender {
    /// `Option` so that `close` can drop it, which is what ends the peer's
    /// receive loop and therefore the far half of the stream.
    tx: Mutex<Option<mpsc::Sender<Bytes>>>,
    max_frame_bytes: u32,
}

#[async_trait]
impl FrameSender for MockSender {
    async fn send(&self, frame: Bytes) -> Result<(), TransportError> {
        /*
         * The declared limit is enforced here rather than assumed, because a
         * transport whose capabilities say one thing and whose behaviour does
         * another is the failure the meta-test is looking for. Refusing is the
         * honest outcome: silently truncating would corrupt a transfer in a way
         * that looks like the sender's bug.
         */
        if frame.len() > self.max_frame_bytes as usize {
            return Err(TransportError::Protocol(format!(
                "frame of {} bytes exceeds the declared limit of {}",
                frame.len(),
                self.max_frame_bytes
            )));
        }

        /*
         * Cloned out of the lock and the guard dropped before the await, so the
         * lock is never held across a suspension point. Cloning a sender is a
         * refcount bump.
         */
        let tx = self
            .tx
            .lock()
            .expect("sender lock poisoned")
            .clone()
            .ok_or(TransportError::Closed)?;

        tx.send(frame).await.map_err(|_| TransportError::Closed)
    }

    async fn close(&self) -> Result<(), TransportError> {
        // Idempotent: dropping the already-taken sender is a no-op.
        self.tx.lock().expect("sender lock poisoned").take();

        Ok(())
    }
}

/// The receiving half.
struct MockReceiver {
    rx: mpsc::Receiver<Bytes>,
}

#[async_trait]
impl FrameReceiver for MockReceiver {
    async fn recv(&mut self) -> Result<Option<Bytes>, TransportError> {
        // `None` is the peer closing cleanly, which is an ending rather than a
        // break — the port distinguishes the two and so must this.
        Ok(self.rx.recv().await)
    }
}

#[async_trait]
impl Transport for MockTransport {
    fn kind(&self) -> TransportKind {
        self.inner.kind
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities
    }

    async fn probe(&self, peer: &Peer, budget: Duration) -> ProbeOutcome {
        /*
         * A peer we have no address for cannot be probed, whatever the far end
         * is doing. This is the check that makes a missing address
         * `Unavailable` rather than a hang, which is what the selector relies on
         * when the control plane supplies only one of a peer's addresses.
         */
        if peer.address(self.inner.kind).is_none() {
            return ProbeOutcome::Unavailable {
                reason: UnavailableReason::PeerAbsent,
            };
        }

        if !self.inner.up.load(Ordering::Relaxed) {
            return ProbeOutcome::Unavailable {
                reason: UnavailableReason::PeerAbsent,
            };
        }

        let delay = *self.inner.probe_delay.lock().expect("probe delay poisoned");

        /*
         * The port's contract is that a probe returns *inside* its budget. A
         * real transport gets there with a socket timeout; here the delay is
         * simulated, so the budget is compared directly. Answering
         * `Unavailable` on timeout rather than `Ready` is deliberate — a probe
         * that ran out of time has not established that anything works.
         */
        if delay >= budget {
            return ProbeOutcome::Unavailable {
                reason: UnavailableReason::PeerAbsent,
            };
        }

        if delay > Duration::ZERO {
            tokio::time::sleep(delay).await;
        }

        self.inner
            .probe_outcome
            .lock()
            .expect("probe outcome poisoned")
            .clone()
    }

    async fn connect(
        &self,
        peer: &Peer,
        session: &SessionId,
        _token: &SessionToken,
    ) -> Result<DuplexStream, TransportError> {
        if peer.address(self.inner.kind).is_none() {
            return Err(TransportError::Unreachable);
        }

        if !self.inner.up.load(Ordering::Relaxed) {
            return Err(TransportError::Unreachable);
        }

        if self.inner.refuses.load(Ordering::Relaxed) {
            return Err(TransportError::Refused);
        }

        let (outbound_tx, outbound_rx) = mpsc::channel::<Bytes>(QUEUE_DEPTH);
        let (inbound_tx, inbound_rx) = mpsc::channel::<Bytes>(QUEUE_DEPTH);

        /*
         * The far end: everything it receives comes back, so the suite can
         * assert that a frame sequence survives the trip unchanged and in
         * order. It ends when the sender half is dropped or closed, which is
         * what turns a `close` on this side into an `Ok(None)` on the other.
         */
        tokio::spawn(async move {
            let mut outbound_rx = outbound_rx;

            while let Some(frame) = outbound_rx.recv().await {
                if inbound_tx.send(frame).await.is_err() {
                    break;
                }
            }
        });

        self.inner
            .sessions
            .lock()
            .expect("sessions poisoned")
            .insert(session.as_str().to_owned());

        Ok(DuplexStream::new(
            Box::new(MockSender {
                tx: Mutex::new(Some(outbound_tx)),
                max_frame_bytes: self.inner.capabilities.max_frame_bytes,
            }),
            Box::new(MockReceiver { rx: inbound_rx }),
        ))
    }

    async fn health(&self, session: &SessionId) -> Health {
        if !self.inner.up.load(Ordering::Relaxed) {
            return Health::Dead;
        }

        let known = self
            .inner
            .sessions
            .lock()
            .expect("sessions poisoned")
            .contains(session.as_str());

        if known {
            Health::Alive {
                rtt: Duration::from_millis(1),
            }
        } else {
            Health::Dead
        }
    }

    async fn teardown(&self, session: &SessionId) -> Result<(), TransportError> {
        // Idempotent: a failover, a cancel and a shutdown can all arrive here,
        // and removing a session that is already gone has already done what was
        // asked.
        self.inner
            .sessions
            .lock()
            .expect("sessions poisoned")
            .remove(session.as_str());

        Ok(())
    }
}
