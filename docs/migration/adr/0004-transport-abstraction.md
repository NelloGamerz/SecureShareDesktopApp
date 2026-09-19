# ADR-0004 — Model the transfer transport as a port

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 8
- **Supersedes:** —

## Context

There is exactly one way bytes move: `POST {endpoint}/transfer/chunk`, where
`endpoint` is an opaque string supplied by the control plane
(`transfer/http_client.rs:30-36`, `:75-140`). There is no transport concept, no
capability model, and no failover.

`UploadState` carries a `network_type: ConnectionType` field
(`transfer/state.rs:11-27`) that is **never used to make a decision** — a vestige
of an abstraction that was never built.

The goal is to add LAN (and later P2P, relay, Bluetooth) **without modifying
existing code**.

## Options considered

**A. Add a `match transport_kind` inside the upload path.**
Rejected. Every new transport edits shared code — the exact failure mode the
requirement forbids. Deleting `ConnectionType` and pretending is worse.

**B. A `Transport` trait that exposes `send_chunk(...)`.**
Rejected — **this is the trap.** A chunk-aware transport cannot carry a
handshake, cannot be used for a raw stream, and bakes the protocol into every
implementation. It is the current design wearing a trait.

**C. A byte-oriented `Transport` port with a `DuplexStream` waist.**
**Chosen.**

**D. Adopt `iroh` as the transport layer wholesale.**
Deferred, not rejected. `iroh` (QUIC, ~90–95% hole-punch success, relay
fallback, `iroh-blobs` for content-addressed transfer) is a strong candidate for
the *P2P* transport. But adopting it as the *only* transport would (a) discard
the tunnel path, breaking the installed base, and (b) replace the protocol layer
this design deliberately keeps. Revisit at Phase 10 as a `P2pTransport`.

## Decision

Define in `vilsend-core`:

- `Transport` — `kind()`, `capabilities()`, `probe()`, `connect()`, `health()`,
  `teardown()`.
- `DuplexStream` — `send(Bytes)`, `recv() -> Option<Bytes>`, `close()`.
- `Capabilities` — `works_offline`, `end_to_end_direct`, `has_relay_fallback`,
  `needs_nat_traversal`, `resumable`, `streaming`, `nominal_throughput`,
  `setup_latency_ms`.

**The invariant:** a transport moves bytes and knows nothing about what they
mean. It may not know what a chunk, manifest, transfer, or file is; it may not
encrypt or emit progress; it may not decide retry policy.

**Selection** is a `TransportSelector` scoring on **capabilities**, not a
hardcoded priority list. LAN > P2P > tunnel > relay **falls out of the scores**.

**Ordering constraint:** Phase 8 ships the port with the *existing tunnel only*.
Proving the abstraction with a transport you already have, before adding one you
do not, is what makes a failure attributable.

## Consequences

**Positive**

- A new transport is one module + one feature flag + one line in the shell's
  factory list. `TransportKind` gains a variant (the one unavoidable shared
  edit; the enum is `#[non_exhaustive]`).
- Failover with resume becomes expressible: the transfer key lives in the
  **session**, not the transport, so a new transport resumes rather than restarts.
- The contract test suite makes "is this transport correct?" a mechanical
  question.

**Negative**

- One more indirection on the hot path. Negligible: the cost is a virtual call
  per stream, not per byte.
- The `DuplexStream` abstraction is a poor fit for HTTP's request/response model
  and needs a framing layer over it. Accepted — the framing layer is needed
  anyway (Phase 4's `MANIFEST` exchange, and the v2 protocol generally).

**Neutral**

- `probe()` is a **hint**, not a guarantee. A probe can say Ready and the
  subsequent connect can fail. The selector must treat that as a first-class
  case, not an error.

## Related

- [`../02-transport-layer.md`](../02-transport-layer.md)
- ADR-0007 (peer authentication), ADR-0010 (protocol versioning)
