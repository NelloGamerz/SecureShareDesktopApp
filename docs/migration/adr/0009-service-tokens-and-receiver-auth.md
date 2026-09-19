# ADR-0009 — Service tokens and receiver session authorization

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 9 (receiver), 7 (CI tokens)
- **Supersedes:** —

## Context

Two related authorization problems, both currently unsolved.

### Problem 1 — the receiver accepts any credential

The receiver rejects a chunk only when the `Authorization` header is **absent**;
it does not validate the value. It binds `0.0.0.0:7878`.

Any process that can reach the port can open a session and write files. The
payload is E2E-encrypted, so confidentiality holds — but endpoint abuse,
resource exhaustion, and session interference do not require reading the payload.

This is flagged **High** in the repo's own `docs/SECURITY.md:5-18` and is the
first item in `docs/IMPROVEMENT_ROADMAP.md`'s P0 list.

**The LAN transport (Phase 10) makes this strictly worse** by putting the
receiver on every network the device joins. Hence the ordering constraint:
Phase 9 before Phase 10.

### Problem 2 — CI has no way to authenticate

A headless CI job cannot complete a browser flow. It needs a long-lived, scoped,
revocable credential.

## Options considered

### For the receiver

**A. Validate the Bearer value against the transfer key.**
Rejected. The transfer key is derived from an unauthenticated ECDH; it proves
nothing about authorization.

**B. Require a control-plane-minted, transfer-scoped session token.**
**Chosen.**

**C. Bind only to the tunnel interface.**
Necessary but insufficient — it is a mitigation, not an authorization model, and
it does not help the LAN path at all.

**D. mTLS between peers.**
Rejected for v1. It requires a PKI, and the device signing key (ADR-0007)
already provides peer authentication. Revisit if a mutual-TLS-capable control
plane appears.

### For CI tokens

**A. Clerk API Keys** — `ak_` prefix, GA since 2026-04-06, verified via
`/api_keys/verify` or `acceptsToken: 'api_key'`. **Requires the Pro plan**
($100/mo) plus per-operation fees.

**B. Backend-minted service tokens** — an opaque `vls_…` token, stored hashed in
the backend DB, verified by the backend with no Clerk involvement.

## Decision

### Receiver authorization

A `SessionToken` minted by the control plane, containing:

```rust
pub struct SessionToken {
    pub transfer_id: TransferId,
    pub sender_device: DeviceId,
    pub expires_at: UnixTime,
    /// HMAC over the above, keyed by a value the RECEIVER obtains from the
    /// control plane — not a value the sender can mint.
    pub mac: [u8; 32],
}
```

Requirements:

1. Validated **before any filesystem work**.
2. **Bound** to `transfer_id` + `sender_device` + expiry, so a leaked token
   cannot be replayed against a different transfer.
3. **Short TTL** (≤ 15 min), single-use per session.
4. **Receiver binds to a configurable interface**, not `0.0.0.0` by default.
5. Add receiver resource limits: max concurrent sessions, per-session byte cap,
   header/path length limits, request rate limit.

**Interim (task 9.3), because the backend is the long pole:** a per-receiver
random secret held in the OS keychain, exchanged out-of-band via the existing
`START_TRANSFER` payload. Ship this to unblock; migrate to the minted token when
the backend lands. **This is materially better than today's check** (which
validates nothing) even though it is not the end state.

### CI / SDK tokens

**Recommendation: option B — backend-minted service tokens.**

Rationale: you already own the backend. A service token is a table, a hash
column, and a middleware. It avoids a plan upgrade, avoids depending on a
recently-GA'd feature for a load-bearing capability, and gives you rotation and
scoping semantics you control.

Choose Clerk API Keys instead **only** if avoiding backend work is worth more
than the subscription and the dependency.

**Either way, the backend must authorize on scopes.** A long-lived bearer
credential without scoping is a full account takeover waiting to happen
(**R-27**).

Also note: **disabling** a Clerk API key makes it fail verification **without
revoking it**. Do not treat "disabled" as "revoked" in an audit story.

## Consequences

**Positive**

- The receiver gains a real authorization model, which is a prerequisite for the
  LAN transport and a P0 fix in its own right.
- Layered security becomes three distinct properties: **payload confidentiality**
  (E2E crypto), **endpoint authorization** (session token), **peer
  authentication** (ADR-0007). Today these collapse into one weak check.
- Service tokens with backend-controlled rotation avoid a vendor dependency.

**Negative**

- **Requires backend work on both counts** (**Q-06**, the critical path).
- The interim (9.3) is a second mechanism to remove later. Track it explicitly
  so it does not become permanent.
- v1 clients must keep working — the v1 path uses the interim token, not the
  minted one.

**Neutral**

- Choosing backend-minted service tokens means more backend code. That is the
  trade: control and cost, versus convenience.

## Related

- [`../02-transport-layer.md`](../02-transport-layer.md) §7.6
- [`../03-authentication.md`](../03-authentication.md) §3.4
- ADR-0007 (peer authentication)
- Q-05, Q-06 in [`../07-risks-and-open-questions.md`](../07-risks-and-open-questions.md)
