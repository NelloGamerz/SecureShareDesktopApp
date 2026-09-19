# ADR-0010 — Protocol versioning and backward compatibility

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 4
- **Supersedes:** —

## Context

Version `1.0.4` is shipped to real users. Those clients speak a specific wire
protocol:

- Control WebSocket: `START_TRANSFER` with
  `{transfer_id, receiver_public_key, endpoint, chunk_size, concurrency, max_retries}`
  (`websocket/server_command.rs:3-29`).
- Data plane: `POST /transfer/start`, `POST /transfer/chunk`,
  `GET /transfer/public-key`, with headers `Transfer-Id`, `File-Id`,
  `Chunk-Index`, `Total-Chunks`, `Relative-Path`, `Chunk-Nonce`,
  `Encryption: AES-256-GCM` (`transfer/http_client.rs:75-140`).

Phase 4 needs to add AAD and a whole-file integrity check, and Phase 9 needs to
add a device signature and a session token. **All of these change the wire.**

There is no protocol version field anywhere in the current code.

## Options considered

**A. Change the protocol in place.**
Rejected. It breaks every installed client on the day the server updates. There
is no forced-upgrade mechanism beyond the auto-updater, and the updater is
disabled for Microsoft Store builds.

**B. Version the whole API surface and require v2 for everything.**
Rejected. It creates a hard cutover with no transition window.

**C. Additive v2 endpoints alongside a frozen v1 surface.**
**Chosen.**

**D. A version negotiation field in every request.**
Rejected as the primary mechanism — it makes every handler branch on version.
Capability-based detection is cleaner.

## Decision

1. **Freeze v1.** The three `/transfer/*` endpoints keep their **exact** request
   and response schema, unchanged, indefinitely. They become a thin adapter that
   translates v1 headers into the core's `ChunkSpec` and calls the same code
   path as v2.

2. **Add v2 under new paths** — `/v2/session`, `/v2/stream`. An old receiver
   returns **404** rather than misparsing, which makes detection unambiguous.

3. **Detect by capability, not by version string.** Probe `/v2/session`; fall
   back on 404. Version strings drift; endpoints do not.

4. **Additive-only changes to `START_TRANSFER`.** New fields must be `Option`
   and ignored by old clients. `transport_hints: Option<Vec<TransportKind>>` is
   the first such field.

5. **A `PROTOCOL_VERSION: u16` constant**, independent of the crate version. The
   wire contract and the code version evolve at different rates.

6. **Refuse to downgrade silently.** If a peer offers only a transport the policy
   forbids, fail with `NoRoute` rather than quietly relaying.

### Compatibility matrix — the contract

| Sender ↓ / Receiver → | v1 client | v2 client |
|---|---|---|
| **v1 client** | Works (today) | Works — v2 serves `/transfer/*` unchanged |
| **v2 client** | **Must work** — tunnel only, no resume, no signature | Full feature set |

Building the v2 receiver to **also** serve the v1 endpoints — rather than
replacing them — is what makes this matrix diagonal-safe, and it is cheap.

### Enforcement scoping

Signature verification (ADR-0007) and minted session tokens (ADR-0009) are
**v2-only**. v1 clients continue over the tunnel with the interim token. This is
what keeps the compatibility floor from becoming the security hole: v1 is
protected, just less strongly.

## Consequences

**Positive**

- No forced upgrade, no cutover day, no broken installed base.
- New capabilities are opt-in by capability detection.
- The v1 adapter is thin — a header translation, not a second implementation.

**Negative**

- Two code paths exist for one protocol. The v1 path is frozen and must be
  **tested in CI against a pinned v1 binary** ([`06`](../06-testing-and-quality.md) §6.2),
  or it will rot silently.
- The v1 path is permanently weaker on security. That is a deliberate,
  documented trade for compatibility, and it must be retired on a schedule
  rather than left forever.

**Neutral**

- `PROTOCOL_VERSION` and the crate version are separate on purpose. Confusing
  them is how compatibility bugs happen.

## Related

- [`../02-transport-layer.md`](../02-transport-layer.md) §8
- ADR-0004 (transport abstraction), ADR-0007 (peer auth), ADR-0009 (session tokens)
- A-03 in [`../07-risks-and-open-questions.md`](../07-risks-and-open-questions.md)
