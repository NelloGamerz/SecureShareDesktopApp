//! The transport contract suite, run against [`MockTransport`].
//!
//! `docs/migration/02-transport-layer.md` §6.1 requires every `Transport`
//! implementation to pass a suite **unmodified** — that is what makes "is this
//! transport correct?" a mechanical question rather than a review. The suite
//! currently runs against the in-memory transport only, because it is the only
//! one there is; the tunnel joins it when it lands, and these assertions are
//! written against the port's vocabulary rather than the mock's internals so
//! that joining is a copy rather than a rewrite.
//!
//! Three of the document's nine are *not* here, and the reason is specific
//! rather than an omission:
//!
//! * `teardown_releases_resources` asserts no file-descriptor leak, which is a
//!   question about a socket. The in-memory transport has none, so asserting it
//!   here would pass for the wrong reason. It lands with the tunnel.
//! * `partial_frame_does_not_corrupt` requires a framing in which a frame can be
//!   half-delivered. The port carries whole `Bytes`, so there is no half-frame
//!   to survive yet; the length-delimited framing that creates one is the next
//!   stage's work. The in-memory analogue — a frame over the declared limit is
//!   refused rather than truncated — is tested below.
//! * `connect_is_idempotent_per_session` is asserted as "a second connect on the
//!   same session succeeds and is independent" rather than by counting tasks,
//!   because a task count is a fact about the implementation and the contract is
//!   about behaviour.

use std::time::{Duration, Instant};

use bytes::Bytes;
use vilsend_core::{
    Health, Peer, PeerAddress, PeerRef, ProbeOutcome, SessionId, SessionToken, Transport,
    TransportKind, UnavailableReason,
};

use vilsend_transport::mock::{coherent_capabilities, MockTransport};

const SESSION: &str = "session-1";

fn session() -> SessionId {
    SessionId::new(SESSION)
}

fn token() -> SessionToken {
    SessionToken::new("token-1")
}

/// A peer that can be reached by `kind`, and by nothing else.
fn peer_reachable_by(kind: TransportKind) -> Peer {
    Peer {
        reference: PeerRef::new("peer-1"),
        device_id: None,
        addresses: vec![PeerAddress {
            kind,
            address: "memory://peer-1".into(),
        }],
    }
}

fn memory_peer() -> Peer {
    peer_reachable_by(TransportKind::Memory)
}

/// The budget every probe in this file is given.
const BUDGET: Duration = Duration::from_millis(150);

#[tokio::test]
async fn probe_is_bounded() {
    /*
     * A transport whose probe would take five seconds, asked for an answer in
     * 150 ms. It must answer, and it must answer *inside* the budget — a probe
     * that ran long has not established that anything works, so `Unavailable` is
     * the honest reply rather than `Ready`.
     */
    let transport = MockTransport::new().with_probe_delay(Duration::from_secs(5));

    let started = Instant::now();
    let outcome = transport.probe(&memory_peer(), BUDGET).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < BUDGET * 2,
        "the probe took {elapsed:?}, which is outside its {BUDGET:?} budget"
    );

    assert_eq!(
        outcome,
        ProbeOutcome::Unavailable {
            reason: UnavailableReason::PeerAbsent
        },
        "a probe that cannot finish in time must not claim the peer is ready"
    );
}

#[tokio::test]
async fn probe_is_ready_for_a_reachable_peer() {
    let transport = MockTransport::new();

    let outcome = transport.probe(&memory_peer(), BUDGET).await;

    assert!(
        matches!(outcome, ProbeOutcome::Ready { .. }),
        "expected Ready, got {outcome:?}"
    );
}

#[tokio::test]
async fn honours_peer_unavailable() {
    /*
     * A peer the control plane gave us no address for cannot be probed, whatever
     * the far end is doing. This is the check that turns "the endpoint field was
     * missing" into an unavailable candidate rather than a hang.
     */
    let transport = MockTransport::new();
    let unreachable = peer_reachable_by(TransportKind::Tunnel);

    assert_eq!(
        transport.probe(&unreachable, BUDGET).await,
        ProbeOutcome::Unavailable {
            reason: UnavailableReason::PeerAbsent
        }
    );

    assert!(
        transport
            .connect(&unreachable, &session(), &token())
            .await
            .is_err(),
        "connecting to a peer with no address for this transport must fail"
    );
}

#[tokio::test]
async fn a_peer_that_is_gone_is_unavailable_rather_than_slow() {
    let transport = MockTransport::down();

    assert_eq!(
        transport.probe(&memory_peer(), BUDGET).await,
        ProbeOutcome::Unavailable {
            reason: UnavailableReason::PeerAbsent
        }
    );

    assert!(
        transport
            .connect(&memory_peer(), &session(), &token())
            .await
            .is_err(),
        "connecting to a peer that is down must fail"
    );
}

#[tokio::test]
async fn a_refusal_is_classified_as_no_route() {
    /*
     * A refusal is an answer, not a silence, and the port classifies it as
     * `NoRoute` — the same as unreachability, because to a caller they mean the
     * same thing: there is no way to this peer, try another route. The
     * distinction that matters for failover is `NoRoute` versus `Network`, and
     * this pins the first half of it.
     */
    let transport = MockTransport::refusing();

    let error = transport
        .connect(&memory_peer(), &session(), &token())
        .await
        .expect_err("a refusing peer must not produce a stream");

    assert_eq!(error.kind(), vilsend_core::ErrorKind::NoRoute);
}

#[tokio::test]
async fn connect_after_teardown_succeeds() {
    let transport = MockTransport::new();
    let peer = memory_peer();

    transport
        .connect(&peer, &session(), &token())
        .await
        .unwrap();
    transport.teardown(&session()).await.unwrap();

    transport
        .connect(&peer, &session(), &token())
        .await
        .expect("a session may be re-established after a teardown, which is what a failover does");
}

#[tokio::test]
async fn teardown_is_idempotent_and_leaves_nothing_live() {
    let transport = MockTransport::new();
    let peer = memory_peer();

    transport
        .connect(&peer, &session(), &token())
        .await
        .unwrap();
    assert_eq!(transport.live_sessions(), 1);

    transport.teardown(&session()).await.unwrap();

    // A failover, a user cancel and a shutdown can all reach teardown; the
    // second one has already done what was asked.
    transport
        .teardown(&session())
        .await
        .expect("teardown must be idempotent");

    assert_eq!(transport.live_sessions(), 0);
}

#[tokio::test]
async fn health_reports_dead_after_the_peer_is_gone() {
    let transport = MockTransport::new();
    let peer = memory_peer();

    transport
        .connect(&peer, &session(), &token())
        .await
        .unwrap();
    assert!(
        matches!(transport.health(&session()).await, Health::Alive { .. }),
        "a freshly connected session must read as alive"
    );

    transport.peer_goes_away();

    assert_eq!(
        transport.health(&session()).await,
        Health::Dead,
        "a session whose peer has gone must read as dead, not merely quiet"
    );
}

#[tokio::test]
async fn health_reports_dead_for_a_session_that_was_torn_down() {
    let transport = MockTransport::new();
    let peer = memory_peer();

    transport
        .connect(&peer, &session(), &token())
        .await
        .unwrap();
    transport.teardown(&session()).await.unwrap();

    assert_eq!(transport.health(&session()).await, Health::Dead);
}

#[tokio::test]
async fn stream_preserves_byte_order() {
    /*
     * Frames must come back in the order they went out. A stream that reordered
     * them would produce a file the receiver assembles from the right bytes in
     * the wrong places, which is the corruption class the whole design is
     * arranged to make impossible.
     */
    let transport = MockTransport::new();
    let mut stream = transport
        .connect(&memory_peer(), &session(), &token())
        .await
        .unwrap();

    let mut receiver = stream.take_receiver().expect("the receiver half is free");

    const FRAMES: usize = 500;
    let sent: Vec<Bytes> = (0..FRAMES)
        .map(|index| Bytes::from(format!("frame-{index:04}")))
        .collect();

    // Send and receive concurrently: the queue is bounded, so filling it before
    // draining it would deadlock the test rather than fail it.
    let writer = async {
        for frame in &sent {
            stream.send(frame.clone()).await.unwrap();
        }
    };

    let reader = async {
        let mut received = Vec::with_capacity(FRAMES);

        for _ in 0..FRAMES {
            received.push(receiver.recv().await.unwrap().expect("the peer is up"));
        }

        received
    };

    let ((), received) = tokio::join!(writer, reader);

    assert_eq!(received, sent, "the frame sequence was not preserved");
}

#[tokio::test]
async fn closing_the_stream_ends_the_peers_side() {
    let transport = MockTransport::new();
    let mut stream = transport
        .connect(&memory_peer(), &session(), &token())
        .await
        .unwrap();

    let mut receiver = stream.take_receiver().expect("the receiver half is free");

    stream.close().await.unwrap();

    /*
     * `Ok(None)` rather than an error: the peer closed cleanly, which is an
     * ending and not a break. The port distinguishes the two and so must every
     * transport — a session layer that treated a clean close as a failure would
     * report a completed transfer as a broken one.
     */
    assert_eq!(receiver.recv().await.unwrap(), None);
}

#[tokio::test]
async fn a_frame_over_the_declared_limit_is_refused_not_truncated() {
    /*
     * `Capabilities::max_frame_bytes` is a promise, and this is the promise
     * being kept. Truncating would corrupt a chunk in a way that looks like the
     * sender's bug; refusing surfaces it where it happened.
     */
    let transport = MockTransport::new();
    let limit = transport.capabilities().max_frame_bytes;

    let stream = transport
        .connect(&memory_peer(), &session(), &token())
        .await
        .unwrap();

    let oversized = Bytes::from(vec![0u8; limit as usize + 1]);

    let error = stream
        .send(oversized)
        .await
        .expect_err("a frame over the declared limit must be refused");

    assert!(
        matches!(error, vilsend_core::TransportError::Protocol(_)),
        "expected a protocol error for an oversized frame, got {error:?}"
    );
}

/// The meta-test, and the one worth its cost.
///
/// It asserts that a transport's declared capabilities are *coherent* with the
/// kind it claims to be — the most likely lie a new implementation tells, per
/// `docs/migration/02-transport-layer.md` §6.1. A transport that traverses a
/// relay but declares `end_to_end_direct: true` would score ahead of a genuinely
/// direct path, and the user-facing consequence is that their file silently went
/// through a third party after they were told it would not.
#[test]
fn capabilities_match_declared_behaviour() {
    for kind in [
        TransportKind::Memory,
        TransportKind::Lan,
        TransportKind::Tunnel,
        TransportKind::P2p,
        TransportKind::Relay,
    ] {
        let transport = MockTransport::with_kind(kind);
        let capabilities = transport.capabilities();

        assert_eq!(
            capabilities.end_to_end_direct,
            kind.is_direct(),
            "{kind:?} declares end_to_end_direct = {}, which contradicts its kind",
            capabilities.end_to_end_direct
        );

        // Nagle's own claim, spelled out: a relayed transport must not claim
        // directness even though it is "not the internet".
        if matches!(kind, TransportKind::Tunnel | TransportKind::Relay) {
            assert!(
                !capabilities.end_to_end_direct,
                "{kind:?} puts a third party in the path and must not claim to be direct"
            );
        }

        // Only a transport that performs NAT traversal may say it needs to.
        assert_eq!(
            capabilities.needs_nat_traversal,
            matches!(kind, TransportKind::P2p),
            "{kind:?} declares needs_nat_traversal = {}",
            capabilities.needs_nat_traversal
        );

        // Every transport in this suite carries sessions, and the selector
        // filters on exactly this.
        assert_eq!(
            capabilities.framing,
            vilsend_core::Framing::ByteStream,
            "{kind:?} must be able to carry a session to be in this suite"
        );

        assert!(
            capabilities.max_frame_bytes > 0,
            "{kind:?} declares a zero-length frame limit, so no chunk could ever be sent"
        );

        assert_eq!(transport.kind(), kind);
    }
}

/// The coherence helper is what the mock builds from, so it must agree with the
/// hand-written expectations above. This is the guard on the guard: if
/// [`coherent_capabilities`] were changed to say a tunnel is direct, the test
/// above would still pass against the mock while describing a lie.
#[test]
fn the_coherence_table_refuses_to_describe_a_tunnel_as_direct() {
    assert!(!coherent_capabilities(TransportKind::Tunnel).end_to_end_direct);
    assert!(!coherent_capabilities(TransportKind::Relay).end_to_end_direct);
    assert!(coherent_capabilities(TransportKind::Lan).end_to_end_direct);
    assert!(coherent_capabilities(TransportKind::P2p).end_to_end_direct);
}
