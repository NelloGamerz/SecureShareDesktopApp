//! Choosing a transport, by capability rather than by name.
//!
//! The rule this module exists to keep: **the selector must not know about any
//! particular transport.** It reads [`Capabilities`], scores them, and picks.
//! `docs/migration/02-transport-layer.md` §6 puts it as an instruction to the
//! next contributor — *"you must not add a branch to the selector; if you feel
//! the need to, the capability model is missing a dimension"* — and the
//! ordering that falls out, LAN > P2P > tunnel > relay, is a *consequence* of
//! the scores rather than a `match` statement.
//!
//! # What is deliberately not here yet
//!
//! **Racing.** When two candidates are close, connecting to both and keeping the
//! first whose handshake completes is faster, and costs bandwidth and, on a
//! laptop, battery. It is deferred with the failover work, because racing is
//! only worth its complexity once there is a second transport to race, and
//! because a race is meaningless without the "first frame delivered"
//! confirmation that the session layer provides.
//!
//! **The selection state machine.** `docs/migration/02-transport-layer.md` §5.3
//! models `Idle → Probing → Ranking → Racing → Connecting → Connected`. With
//! one candidate and no racing, most of those states are unreachable and the
//! machine would be untestable vocabulary. It arrives with the loop that drives
//! it.
//!
//! **The stall detector and mid-transfer failover.** Deferred with racing: both
//! need the health monitor, and the health monitor needs something to monitor.

use std::sync::Arc;
use std::time::Duration;

use vilsend_core::{
    Capabilities, DuplexStream, Framing, Peer, ProbeOutcome, SessionId, SessionToken, Transport,
    TransportKind, UnavailableReason,
};

use crate::registry::Registry;

/// What the caller will and will not accept.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Refuse any transport whose bytes traverse a third party.
    ///
    /// The downgrade-attack defence. With more than one route an attacker can
    /// suppress the direct one and force the relayed one, which is a real
    /// availability and metadata attack even under end-to-end encryption — so a
    /// caller that will not accept the downgrade must be able to say so, and
    /// the session must then *fail loudly* rather than fall back quietly.
    pub require_direct: bool,
    /// Use this transport and no other. A user override, or a compliance rule.
    pub force: Option<TransportKind>,
    /// Never use these, whatever they score.
    pub deny: Vec<TransportKind>,
    /// The score worth attempting at all.
    ///
    /// Not the same as the document's "minimum score to accept without trying
    /// anything else": because candidates are attempted in descending score
    /// order and the first success wins, the document's phrasing would describe
    /// behaviour that already happens. As a floor it has a distinct job —
    /// refusing to attempt a route that is hopeless — which is the difference
    /// between failing now and failing after a timeout.
    pub accept_threshold: i32,
    /// How long one probe may take.
    ///
    /// Per probe, not the document's total concurrent budget. A total budget
    /// only differs from a per-probe one when probes overlap, and overlapping
    /// them is part of the racing machinery that is deferred.
    pub probe_budget: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            require_direct: false,
            force: None,
            deny: Vec::new(),
            accept_threshold: 0,
            probe_budget: Duration::from_millis(150),
        }
    }
}

/// Why no transport could be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    /// Nothing was reachable, or nothing was registered.
    #[error("no route to the peer")]
    NoRoute,

    /// The policy refused every candidate that could have worked.
    ///
    /// Separate from [`SelectError::NoRoute`] because the remedy differs: a
    /// `NoRoute` may resolve on its own, whereas a policy refusal is deliberate
    /// and reporting "no route" would hide a rule the user or their
    /// organisation set.
    #[error("policy refused every available transport")]
    RefusedByPolicy,
}

/// What the selector chose.
pub struct Selected {
    /// The transport that accepted the session.
    pub kind: TransportKind,
    /// Its raw stream. No handshake has happened — see the port's docs.
    pub stream: DuplexStream,
}

/// Written by hand rather than derived: a `Selected` is the value a caller holds
/// for the life of a transfer, and printing the whole stream alongside the kind
/// would fill a log with a session's internals. The kind is the part worth
/// seeing.
impl std::fmt::Debug for Selected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Selected")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// A score, or `None` for "excluded".
///
/// `Option` rather than the document's `i32::MIN` sentinel. A sentinel is one
/// arithmetic slip away from a transport scoring its way out of a veto, and the
/// vetoes here are security decisions — `require_direct` is the downgrade
/// defence, and it must not be possible to out-score it.
///
/// This is the single place a candidate is judged, so it is also where the
/// cheap vetoes live. A candidate that comes back `Some` is one that is both
/// admissible and reachable.
pub fn score(
    kind: TransportKind,
    capabilities: &Capabilities,
    probe: &ProbeOutcome,
    policy: &Policy,
) -> Option<i32> {
    if policy.deny.contains(&kind) {
        return None;
    }

    if let Some(forced) = policy.force {
        if forced != kind {
            return None;
        }
    }

    if policy.require_direct && !capabilities.end_to_end_direct {
        return None;
    }

    /*
     * The dimension that replaced the branch. A transport that cannot carry a
     * length-delimited byte stream cannot carry a session, so it is excluded
     * here rather than special-cased later.
     */
    if capabilities.framing != Framing::ByteStream {
        return None;
    }

    if matches!(probe, ProbeOutcome::Unavailable { .. }) {
        return None;
    }

    let mut score = 0;

    // Directness dominates, because it is the property that matters most: a
    // direct path is roughly an order of magnitude faster in practice, and it
    // is the one that keeps the payload off a third party.
    if capabilities.end_to_end_direct {
        score += 60;
    }

    if capabilities.works_offline {
        score += 15;
    }

    // Latency and throughput break ties rather than deciding, so that a fast
    // relay cannot out-rank a slow direct path.
    if let ProbeOutcome::Ready {
        rtt,
        estimated_throughput,
    } = probe
    {
        score += 40 - rtt.as_millis().min(40) as i32;
        score += (estimated_throughput / (16 * 1024 * 1024)).min(30) as i32;
    }

    // Setup cost is a penalty, so a transport that takes seconds to establish
    // loses to one that is instant when everything else is equal.
    score -= capabilities.setup_latency_ms.min(200) as i32 / 20;

    Some(score)
}

/// Picks a transport from a [`Registry`].
#[derive(Debug, Clone)]
pub struct TransportSelector {
    registry: Registry,
}

impl TransportSelector {
    /// Build a selector over a registry.
    pub fn new(registry: Registry) -> Self {
        Self { registry }
    }

    /// The registry this selector reads.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Probe every candidate, then connect to the best one that works.
    ///
    /// Probes run concurrently — they are independent, and a slow probe blocking
    /// a fast one would make the budget meaningless. Connections do *not*: with
    /// nothing racing, attempting a second candidate after a first failure is a
    /// fallback, and the first success wins.
    pub async fn select(
        &self,
        peer: &Peer,
        session: &SessionId,
        token: &SessionToken,
        policy: &Policy,
    ) -> Result<Selected, SelectError> {
        if self.registry.is_empty() {
            return Err(SelectError::NoRoute);
        }

        /*
         * The policy vetoes that cost nothing are applied before any probe is
         * spent, so a denied transport is never dialled. The same vetoes are
         * re-applied inside `score`, which is what keeps them from drifting
         * apart: this filter is an optimisation, and `score` is the authority.
         */
        let candidates: Vec<Arc<dyn Transport>> = self
            .registry
            .candidates()
            .iter()
            .filter(|transport| {
                let capabilities = transport.capabilities();

                score(
                    transport.kind(),
                    &capabilities,
                    &ProbeOutcome::Possible,
                    policy,
                )
                .is_some()
            })
            .cloned()
            .collect();

        if candidates.is_empty() {
            return Err(SelectError::RefusedByPolicy);
        }

        let probed = probe_all(&candidates, peer, policy.probe_budget).await;

        /*
         * The probe outcome decides, so this second `score` is the one that
         * counts. `sort_by` is stable, so transports that score identically keep
         * the registry's registration order — which is the only thing
         * registration order is for.
         */
        let mut ranked: Vec<(i32, Arc<dyn Transport>)> = probed
            .into_iter()
            .filter_map(|(transport, outcome)| {
                score(
                    transport.kind(),
                    &transport.capabilities(),
                    &outcome,
                    policy,
                )
                .map(|score| (score, transport))
            })
            .collect();

        ranked.sort_by(|left, right| right.0.cmp(&left.0));

        for (candidate_score, transport) in ranked {
            if candidate_score < policy.accept_threshold {
                continue;
            }

            match transport.connect(peer, session, token).await {
                Ok(stream) => {
                    return Ok(Selected {
                        kind: transport.kind(),
                        stream,
                    })
                }

                /*
                 * A candidate that probed `Ready` and then failed to connect is
                 * the case the port calls first-class rather than an error: a
                 * probe is a hint and the fallback is the answer. Logged, and
                 * the next candidate is tried.
                 */
                Err(error) => tracing::warn!(
                    transport = ?transport.kind(),
                    session = %session,
                    error = %error,
                    "a transport that probed as available could not be connected; \
                     falling back to the next candidate"
                ),
            }
        }

        Err(SelectError::NoRoute)
    }
}

/// Probe every candidate concurrently, each within its own budget.
///
/// A free function rather than a method because it reads nothing from the
/// selector, which keeps the borrow of the transport list obvious.
async fn probe_all(
    candidates: &[Arc<dyn Transport>],
    peer: &Peer,
    budget: Duration,
) -> Vec<(Arc<dyn Transport>, ProbeOutcome)> {
    let mut set = tokio::task::JoinSet::new();

    for transport in candidates {
        let transport = Arc::clone(transport);
        let peer = peer.clone();

        set.spawn(async move {
            /*
             * A probe that overruns its budget is `Unavailable`, not `Ready`: it
             * has not established that anything works. The port's contract says
             * a probe returns inside its budget, so this is the belt to that
             * braces — and it is what makes the budget mean something even if a
             * transport forgets.
             */
            let outcome = match tokio::time::timeout(budget, transport.probe(&peer, budget)).await {
                Ok(outcome) => outcome,
                Err(_) => ProbeOutcome::Unavailable {
                    reason: UnavailableReason::PeerAbsent,
                },
            };

            (transport, outcome)
        });
    }

    let mut probed = Vec::with_capacity(candidates.len());

    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(result) => probed.push(result),
            // A panicking probe is a bug in that one transport, not a reason to
            // fail the selection: it simply contributes no candidate, and the
            // rest of the selection proceeds.
            Err(error) => tracing::error!(%error, "a transport's probe panicked"),
        }
    }

    probed
}
