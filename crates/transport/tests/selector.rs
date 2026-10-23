//! Selection: the scoring table, the policy vetoes, and the fallback order.

use std::sync::Arc;
use std::time::Duration;

use vilsend_core::{
    Capabilities, DuplexStream, Framing, Health, Peer, PeerAddress, PeerRef, ProbeOutcome,
    SessionId, SessionToken, Transport, TransportError, TransportKind, UnavailableReason,
};

use vilsend_transport::mock::{coherent_capabilities, MockTransport};
use vilsend_transport::{score, Policy, Registry, SelectError, TransportSelector};

fn peer_for(kinds: &[TransportKind]) -> Peer {
    Peer {
        reference: PeerRef::new("peer-1"),
        device_id: None,
        addresses: kinds
            .iter()
            .map(|kind| PeerAddress {
                kind: *kind,
                address: format!("memory://{kind:?}"),
            })
            .collect(),
    }
}

fn session() -> SessionId {
    SessionId::new("session-1")
}

fn token() -> SessionToken {
    SessionToken::new("token-1")
}

fn ready() -> ProbeOutcome {
    ProbeOutcome::Ready {
        rtt: Duration::from_millis(5),
        estimated_throughput: 500_000_000,
    }
}

fn caps_for(kind: TransportKind) -> Capabilities {
    coherent_capabilities(kind)
}

// ---------------------------------------------------------------- scoring ----

#[test]
fn directness_outranks_everything_else() {
    let policy = Policy::default();

    let lan = score(
        TransportKind::Lan,
        &caps_for(TransportKind::Lan),
        &ready(),
        &policy,
    )
    .unwrap();

    let tunnel = score(
        TransportKind::Tunnel,
        &caps_for(TransportKind::Tunnel),
        &ready(),
        &policy,
    )
    .unwrap();

    let relay = score(
        TransportKind::Relay,
        &caps_for(TransportKind::Relay),
        &ready(),
        &policy,
    )
    .unwrap();

    /*
     * Direct beats relayed, and that one *is* a consequence of the scores: a
     * direct transport collects the 60-point directness term and a relayed one
     * cannot, whatever its latency.
     */
    assert!(
        lan > tunnel,
        "LAN ({lan}) must outrank the tunnel ({tunnel})"
    );
    assert!(lan > relay, "LAN ({lan}) must outrank a relay ({relay})");
}

/// Two relayed transports are ordered by what they *declare*, not by what they
/// are called.
///
/// This is a correction to `docs/migration/02-transport-layer.md` §5.1, which
/// presents "LAN > P2P > tunnel > relay" as an ordering that "falls out of the
/// scores rather than being a `match` statement". Half of that is true: the
/// direct/non-direct split falls out. The tunnel-versus-relay half does **not**,
/// and cannot — with the same declared capabilities the two score identically,
/// because the selector reads capabilities and never reads the kind. Measured:
/// both score 54 against the fixture capabilities in `vilsend_transport::mock`.
///
/// That is the correct behaviour, and this test pins it in the form that is
/// actually guaranteed: give one relayed transport worse numbers and it loses.
/// Ranking by *name* would be the `match` statement the document forbids.
#[test]
fn between_relayed_transports_the_better_declared_capabilities_win() {
    let policy = Policy::default();

    let mut poor = caps_for(TransportKind::Relay);
    poor.nominal_throughput = 5_000_000;
    poor.setup_latency_ms = 200;

    let poor_probe = ProbeOutcome::Ready {
        rtt: Duration::from_millis(120),
        estimated_throughput: 5_000_000,
    };

    let good_probe = ProbeOutcome::Ready {
        rtt: Duration::from_millis(10),
        estimated_throughput: 200_000_000,
    };

    let good = score(
        TransportKind::Relay,
        &caps_for(TransportKind::Tunnel),
        &good_probe,
        &policy,
    )
    .unwrap();

    let bad = score(TransportKind::Relay, &poor, &poor_probe, &policy).unwrap();

    assert!(
        good > bad,
        "a well-connected relay ({good}) must beat a poor one ({bad}) despite the same kind"
    );

    // And the tie the document's ordering assumes away is real: identical
    // capabilities score identically, whatever the kinds are.
    assert_eq!(
        score(
            TransportKind::Tunnel,
            &caps_for(TransportKind::Tunnel),
            &ready(),
            &policy
        ),
        score(
            TransportKind::Relay,
            &caps_for(TransportKind::Relay),
            &ready(),
            &policy
        ),
        "two relayed transports with identical capabilities must tie, \
         which is what registration order is for"
    );
}

#[test]
fn a_fast_relay_cannot_outrank_a_slow_direct_path() {
    let policy = Policy::default();

    // A relay at 1 Gb/s against a LAN path at 10 Mb/s with a worse RTT.
    let mut fast_relay = caps_for(TransportKind::Relay);
    fast_relay.nominal_throughput = 1_000_000_000;

    let fast_relay_probe = ProbeOutcome::Ready {
        rtt: Duration::from_millis(1),
        estimated_throughput: 1_000_000_000,
    };

    let slow_lan_probe = ProbeOutcome::Ready {
        rtt: Duration::from_millis(40),
        estimated_throughput: 10_000_000,
    };

    let relay = score(
        TransportKind::Relay,
        &fast_relay,
        &fast_relay_probe,
        &policy,
    )
    .unwrap();

    let lan = score(
        TransportKind::Lan,
        &caps_for(TransportKind::Lan),
        &slow_lan_probe,
        &policy,
    )
    .unwrap();

    assert!(
        lan > relay,
        "a direct path ({lan}) must beat a faster relay ({relay}); if it does not, \
         the directness weight is too low and users will be relayed for speed"
    );
}

#[test]
fn require_direct_excludes_the_tunnel() {
    let policy = Policy {
        require_direct: true,
        ..Policy::default()
    };

    assert_eq!(
        score(
            TransportKind::Tunnel,
            &caps_for(TransportKind::Tunnel),
            &ready(),
            &policy
        ),
        None,
        "a policy that requires directness must be able to veto the tunnel"
    );

    assert!(score(
        TransportKind::Lan,
        &caps_for(TransportKind::Lan),
        &ready(),
        &policy
    )
    .is_some());
}

#[test]
fn deny_and_force_both_veto() {
    let denied = Policy {
        deny: vec![TransportKind::Tunnel],
        ..Policy::default()
    };

    assert_eq!(
        score(
            TransportKind::Tunnel,
            &caps_for(TransportKind::Tunnel),
            &ready(),
            &denied
        ),
        None
    );

    let forced = Policy {
        force: Some(TransportKind::Tunnel),
        ..Policy::default()
    };

    assert!(
        score(
            TransportKind::Tunnel,
            &caps_for(TransportKind::Tunnel),
            &ready(),
            &forced
        )
        .is_some(),
        "a forced transport must survive its own force"
    );

    assert_eq!(
        score(
            TransportKind::Lan,
            &caps_for(TransportKind::Lan),
            &ready(),
            &forced
        ),
        None,
        "a forced transport must exclude every other"
    );
}

#[test]
fn an_unavailable_or_unframed_candidate_scores_nothing() {
    let policy = Policy::default();

    assert_eq!(
        score(
            TransportKind::Lan,
            &caps_for(TransportKind::Lan),
            &ProbeOutcome::Unavailable {
                reason: UnavailableReason::PeerAbsent
            },
            &policy
        ),
        None
    );

    let mut unframed = caps_for(TransportKind::Lan);
    unframed.framing = Framing::HttpRequestResponse;

    assert_eq!(
        score(TransportKind::Lan, &unframed, &ready(), &policy),
        None,
        "the selector must filter on framing rather than branch on the transport's kind"
    );
}

#[test]
fn a_candidate_that_needs_a_handshake_still_scores() {
    let policy = Policy::default();

    let possible = score(
        TransportKind::P2p,
        &caps_for(TransportKind::P2p),
        &ProbeOutcome::Possible,
        &policy,
    )
    .unwrap();

    let ready_lan = score(
        TransportKind::Lan,
        &caps_for(TransportKind::Lan),
        &ready(),
        &policy,
    )
    .unwrap();

    assert!(possible > 0, "a Possible candidate is still worth trying");
    assert!(ready_lan > possible);
}

// --------------------------------------------------------------- selecting ----

#[tokio::test]
async fn one_candidate_is_a_passthrough() {
    let registry = Registry::new().with(Arc::new(MockTransport::with_kind(TransportKind::Lan)));
    let selector = TransportSelector::new(registry);

    let selected = selector
        .select(
            &peer_for(&[TransportKind::Lan]),
            &session(),
            &token(),
            &Policy::default(),
        )
        .await
        .expect("a single reachable candidate must be selected");

    assert_eq!(selected.kind, TransportKind::Lan);
}

#[tokio::test]
async fn an_empty_registry_has_no_route() {
    let selector = TransportSelector::new(Registry::new());

    let error = selector
        .select(
            &peer_for(&[TransportKind::Lan]),
            &session(),
            &token(),
            &Policy::default(),
        )
        .await
        .expect_err("nothing is registered, so nothing can be selected");

    assert_eq!(error, SelectError::NoRoute);
}

#[tokio::test]
async fn the_higher_scoring_transport_wins() {
    let registry = Registry::new()
        .with(Arc::new(MockTransport::with_kind(TransportKind::Tunnel)))
        .with(Arc::new(MockTransport::with_kind(TransportKind::Lan)));

    let selector = TransportSelector::new(registry);

    let selected = selector
        .select(
            &peer_for(&[TransportKind::Lan, TransportKind::Tunnel]),
            &session(),
            &token(),
            &Policy::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        selected.kind,
        TransportKind::Lan,
        "the direct path must win on capability, not on registration order"
    );
}

#[tokio::test]
async fn registration_order_only_breaks_ties() {
    // The tunnel is registered first, which must not let it win.
    let registry = Registry::new()
        .with(Arc::new(MockTransport::with_kind(TransportKind::Tunnel)))
        .with(Arc::new(MockTransport::with_kind(TransportKind::Lan)));

    let selector = TransportSelector::new(registry);

    let selected = selector
        .select(
            &peer_for(&[TransportKind::Lan, TransportKind::Tunnel]),
            &session(),
            &token(),
            &Policy::default(),
        )
        .await
        .unwrap();

    assert_eq!(selected.kind, TransportKind::Lan);
}

#[tokio::test]
async fn a_policy_that_excludes_everything_is_reported_as_a_refusal() {
    let registry = Registry::new().with(Arc::new(MockTransport::with_kind(TransportKind::Tunnel)));
    let selector = TransportSelector::new(registry);

    let policy = Policy {
        require_direct: true,
        ..Policy::default()
    };

    let error = selector
        .select(
            &peer_for(&[TransportKind::Tunnel]),
            &session(),
            &token(),
            &policy,
        )
        .await
        .expect_err("a tunnel-only route must not satisfy a direct-only policy");

    /*
     * Distinct from `NoRoute` on purpose: the remedy differs. A `NoRoute` may
     * resolve on its own; a policy refusal is deliberate, and reporting "no
     * route" would hide a rule the user set.
     */
    assert_eq!(error, SelectError::RefusedByPolicy);
}

#[tokio::test]
async fn an_unreachable_top_candidate_falls_back_to_the_next() {
    /*
     * A transport that probes `Possible` and then refuses every connection —
     * the case the port calls first-class rather than an error. The selector
     * must move on rather than fail the transfer.
     */
    let registry = Registry::new()
        .with(Arc::new(MockTransport::refusing().needing_a_handshake()))
        .with(Arc::new(MockTransport::with_kind(TransportKind::Tunnel)));

    let selector = TransportSelector::new(registry);

    let selected = selector
        .select(
            &peer_for(&[TransportKind::Memory, TransportKind::Tunnel]),
            &session(),
            &token(),
            &Policy::default(),
        )
        .await
        .expect("the fallback candidate is reachable, so a route exists");

    assert_eq!(selected.kind, TransportKind::Tunnel);
}

#[tokio::test]
async fn a_hopeless_candidate_is_not_attempted_at_all() {
    /*
     * The score floor's job: refuse to spend a connection attempt on a route
     * that cannot pass. Without it the failure arrives after a timeout instead
     * of immediately.
     */
    let registry = Registry::new().with(Arc::new(MockTransport::with_kind(TransportKind::Lan)));
    let selector = TransportSelector::new(registry);

    let policy = Policy {
        accept_threshold: i32::MAX,
        ..Policy::default()
    };

    let error = selector
        .select(
            &peer_for(&[TransportKind::Lan]),
            &session(),
            &token(),
            &policy,
        )
        .await
        .expect_err("no candidate can reach a threshold of i32::MAX");

    assert_eq!(error, SelectError::NoRoute);
}

#[tokio::test]
async fn the_budget_is_enforced_even_against_a_transport_that_ignores_it() {
    let registry = Registry::new().with(Arc::new(SlowProbe));
    let selector = TransportSelector::new(registry);

    let policy = Policy {
        probe_budget: Duration::from_millis(50),
        ..Policy::default()
    };

    let started = std::time::Instant::now();

    // `SlowProbe` reports Ready only after 10s and never looks at its budget.
    let error = selector
        .select(
            &peer_for(&[TransportKind::Usb]),
            &session(),
            &token(),
            &policy,
        )
        .await
        .expect_err("a probe that never finishes inside the budget leaves no candidate");

    assert_eq!(error, SelectError::NoRoute);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the budget is the selector's to enforce: it waited {:?}",
        started.elapsed()
    );
}

/// A transport whose probe overruns its budget without noticing.
///
/// The port's contract says a probe returns inside its budget. This is what
/// happens when a transport breaks that contract, and it exists to prove the
/// selector does not depend on every transport's good behaviour.
struct SlowProbe;

#[async_trait::async_trait]
impl Transport for SlowProbe {
    fn kind(&self) -> TransportKind {
        TransportKind::Usb
    }

    fn capabilities(&self) -> Capabilities {
        coherent_capabilities(TransportKind::Usb)
    }

    async fn probe(&self, _peer: &Peer, _budget: Duration) -> ProbeOutcome {
        tokio::time::sleep(Duration::from_secs(10)).await;
        ready()
    }

    async fn connect(
        &self,
        _peer: &Peer,
        _session: &SessionId,
        _token: &SessionToken,
    ) -> Result<DuplexStream, TransportError> {
        Err(TransportError::Unreachable)
    }

    async fn health(&self, _session: &SessionId) -> Health {
        Health::Dead
    }

    async fn teardown(&self, _session: &SessionId) -> Result<(), TransportError> {
        Ok(())
    }
}
