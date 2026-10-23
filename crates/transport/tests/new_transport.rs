//! The test that proves the abstraction is real.
//!
//! `docs/migration/02-transport-layer.md` §6 states the payoff of the port:
//! adding a transport is **one new module plus one line in the factory list**,
//! and nothing else changes. `05-migration-plan.md` Phase 8 makes that an
//! acceptance criterion — *"adding a second (mock) transport in a test requires
//! zero changes to existing files"* — because an abstraction that cannot survive
//! that is decorative.
//!
//! So this file defines a transport *from outside the crate*, which the crate
//! has never heard of, and uses it. It compiles only if the port is genuinely
//! open: there is no `match` to extend, no enum to add a case to, and nothing in
//! [`Registry`] or [`TransportSelector`] that names a transport.
//!
//! `UdpSounding` is deliberately not a plausible future transport. If it were
//! realistic, a reviewer could read its success as "the tunnel will fit" rather
//! than as "the interface is open", and the whole point is the second claim.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::mpsc;
use vilsend_core::{
    Capabilities, DuplexStream, FrameReceiver, FrameSender, Framing, Health, Peer, ProbeOutcome,
    SessionId, SessionToken, Transport, TransportError, TransportKind,
};

use vilsend_transport::{Policy, Registry, TransportSelector};

/// A transport defined entirely in this test, with no cooperation from the crate.
struct UdpSounding {
    /// When the "peer" was last heard from. A live map, so `health` has
    /// something to be wrong about.
    last_seen: Arc<Mutex<HashMap<String, Instant>>>,
}

/// The sending half: counts what it sent and remembers nothing else.
struct CountingSender {
    sent: Arc<Mutex<usize>>,
    tx: Mutex<Option<mpsc::Sender<Bytes>>>,
}

#[async_trait]
impl FrameSender for CountingSender {
    async fn send(&self, frame: Bytes) -> Result<(), TransportError> {
        *self.sent.lock().expect("sent poisoned") += 1;

        let tx = self.tx.lock().expect("sender poisoned").clone();

        match tx {
            Some(tx) => tx.send(frame).await.map_err(|_| TransportError::Closed),
            None => Err(TransportError::Closed),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.tx.lock().expect("sender poisoned").take();
        Ok(())
    }
}

struct SilentReceiver {
    rx: mpsc::Receiver<Bytes>,
}

#[async_trait]
impl FrameReceiver for SilentReceiver {
    async fn recv(&mut self) -> Result<Option<Bytes>, TransportError> {
        Ok(self.rx.recv().await)
    }
}

#[async_trait]
impl Transport for UdpSounding {
    fn kind(&self) -> TransportKind {
        // A kind the port already names — adding a *kind* is the one shared
        // edit the port cannot avoid, and it is `#[non_exhaustive]` so that it
        // is additive.
        TransportKind::Lan
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            framing: Framing::ByteStream,
            works_offline: true,
            end_to_end_direct: true,
            has_relay_fallback: false,
            needs_nat_traversal: false,
            resumable: true,
            streaming: true,
            nominal_throughput: 2_000_000_000,
            setup_latency_ms: 1,
            max_frame_bytes: 1024 * 1024,
        }
    }

    async fn probe(&self, peer: &Peer, _budget: Duration) -> ProbeOutcome {
        if peer.address(TransportKind::Lan).is_none() {
            return ProbeOutcome::Unavailable {
                reason: vilsend_core::UnavailableReason::PeerAbsent,
            };
        }

        ProbeOutcome::Ready {
            rtt: Duration::from_millis(2),
            estimated_throughput: 2_000_000_000,
        }
    }

    async fn connect(
        &self,
        peer: &Peer,
        session: &SessionId,
        _token: &SessionToken,
    ) -> Result<DuplexStream, TransportError> {
        if peer.address(TransportKind::Lan).is_none() {
            return Err(TransportError::Unreachable);
        }

        self.last_seen
            .lock()
            .expect("last_seen poisoned")
            .insert(session.as_str().to_owned(), Instant::now());

        let sent = Arc::new(Mutex::new(0));

        let (outbound_tx, outbound_rx) = mpsc::channel::<Bytes>(4);
        let (inbound_tx, inbound_rx) = mpsc::channel::<Bytes>(4);

        // The far end acknowledges each frame by echoing it, so the suite can
        // read something back.
        tokio::spawn(async move {
            let mut outbound_rx = outbound_rx;

            while let Some(frame) = outbound_rx.recv().await {
                if inbound_tx.send(frame).await.is_err() {
                    break;
                }
            }
        });

        Ok(DuplexStream::new(
            Box::new(CountingSender {
                sent,
                tx: Mutex::new(Some(outbound_tx)),
            }),
            Box::new(SilentReceiver { rx: inbound_rx }),
        ))
    }

    async fn health(&self, session: &SessionId) -> Health {
        let seen = self
            .last_seen
            .lock()
            .expect("last_seen poisoned")
            .get(session.as_str())
            .copied();

        match seen {
            Some(at) => Health::Alive { rtt: at.elapsed() },
            None => Health::Dead,
        }
    }

    async fn teardown(&self, session: &SessionId) -> Result<(), TransportError> {
        self.last_seen
            .lock()
            .expect("last_seen poisoned")
            .remove(session.as_str());

        Ok(())
    }
}

fn peer_with_lan_address() -> Peer {
    Peer {
        reference: vilsend_core::PeerRef::new("peer-1"),
        device_id: None,
        addresses: vec![vilsend_core::PeerAddress {
            kind: TransportKind::Lan,
            address: "192.0.2.10:7879".into(),
        }],
    }
}

/// The acceptance criterion itself: a transport the crate has never seen is
/// registered with one line and is selected, used and torn down.
#[tokio::test]
async fn a_new_transport_needs_one_registration_and_nothing_else() {
    let transport = Arc::new(UdpSounding {
        last_seen: Arc::new(Mutex::new(HashMap::new())),
    });

    // The one line.
    let registry = Registry::new().with(transport.clone());

    let selector = TransportSelector::new(registry);

    let selected = selector
        .select(
            &peer_with_lan_address(),
            &SessionId::new("session-1"),
            &SessionToken::new("token-1"),
            &Policy::default(),
        )
        .await
        .expect("a registered, reachable transport must be selectable");

    assert_eq!(selected.kind, TransportKind::Lan);

    // It is usable, not merely selectable.
    let mut stream = selected.stream;
    let mut receiver = stream.take_receiver().expect("the receiver half is free");

    stream.send(Bytes::from_static(b"ping")).await.unwrap();

    assert_eq!(
        receiver.recv().await.unwrap(),
        Some(Bytes::from_static(b"ping"))
    );

    assert!(matches!(
        transport.health(&SessionId::new("session-1")).await,
        Health::Alive { .. }
    ));

    transport
        .teardown(&SessionId::new("session-1"))
        .await
        .unwrap();

    assert_eq!(
        transport.health(&SessionId::new("session-1")).await,
        Health::Dead
    );
}

/// A transport defined outside the crate can also be *refused* by policy,
/// which is the other half of "the selector reads capabilities, not names".
#[tokio::test]
async fn an_outside_transport_is_subject_to_the_same_policy() {
    let transport = Arc::new(UdpSounding {
        last_seen: Arc::new(Mutex::new(HashMap::new())),
    });

    let selector = TransportSelector::new(Registry::new().with(transport));

    let policy = Policy {
        deny: vec![TransportKind::Lan],
        ..Policy::default()
    };

    let error = selector
        .select(
            &peer_with_lan_address(),
            &SessionId::new("session-1"),
            &SessionToken::new("token-1"),
            &policy,
        )
        .await
        .expect_err("a denied transport must not be selected");

    assert_eq!(error, vilsend_transport::SelectError::RefusedByPolicy);
}
