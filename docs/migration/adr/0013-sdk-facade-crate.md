# ADR-0013 — `vilsend-sdk`: the facade crate and its dependency edges

- **Status:** Proposed
- **Date:** 2026-09-20
- **Phase:** 5
- **Supersedes:** —

## Context

`05-migration-plan.md` Phase 5 creates `crates/sdk` as `vilsend-sdk`, the stable
public API. `01-target-architecture.md` §4.1 already places the crate in the
workspace and lists what it may depend on:

> `vilsend-sdk` | The **stable public API**. Re-exports a curated subset; owns
> semver. | `engine`, `runtime`, `auth`, `transport`, `core` | tauri

The shared rules require a short ADR for a crate that needs a new dependency
edge. Three of the five crates in that list do not exist — Phase 3 has not run —
so this ADR records the edges the crate *does* take, why each is unavoidable,
and which of them are meant to disappear.

## Decision

### 1. The crate is `crates/sdk`, named `vilsend-sdk`, `publish = false`

`publish = false` is not a preference: ADR-0005 says "Do not publish the Rust
crate to crates.io until Phase 7 is complete", and `05-migration-plan.md`'s risk
table repeats it. A comment cannot enforce that; the manifest field can.

### 2. Dependencies, and the reason for each

| Edge | Why it is here | Should it survive? |
|---|---|---|
| `vilsend-core` (path) | `PeerRef`, `TransferId`, `TransferStatus`, `VilsendError`/`ErrorKind`, `DomainEvent`, `EventSink` — the SDK is a facade over the core's vocabulary, not a second copy of it | **Yes.** It is the edge §4.1 names. |
| `futures-core` | `Stream` for the events verb (§2.3). Trait-only; no runtime. | **Yes.** A pull API needs the trait. |
| `futures-channel` | The bounded per-subscriber event queue. `futures-util` is not needed, so it is not taken. | Until the events port moves to `core`. |
| `sha2` | Per-chunk integrity over the in-memory link — what `Outcome`'s `verified` field is a statement about | **No.** Moving into the engine once a real chunk cipher exists. |
| `serde` | Requests are data, not closures (§2.2), so they must serialise across FFI | **Yes.** |
| `futures` (dev) | An executor for the integration tests; the crate must not have one | dev-only |

**What is deliberately absent.** `tokio`, `reqwest`, `axum`, `sqlx`, `keyring`
and `tauri` are all absent, and `crates/sdk/tests/no_external_touch.rs` fails
the build if any appears — directly or transitively — or if the crate's own
source names `std::fs`, `std::net`, `std::time::Instant`, `std::path::Path`,
`std::env` or `std::process`.

### 3. `futures-channel` over an unbounded queue or a hand-rolled one

Phase 4 task 4.9 records "unbounded queues are an OOM reachable from a LAN
peer". The event fan-out is the same shape of queue, so it is bounded
(`EVENT_BUFFER = 1024`) and a subscriber that falls behind loses events and can
count how many via `EventStream::dropped_events`. A silent loss would be worse
than either alternative.

### 4. The clock is a port, defined here for now

ADR-0011 §3 forbids `in_memory()` a real clock, so throughput and ETA need an
injected time source. `05-migration-plan.md` task 3.2 puts a `Clock` port in
`vilsend-core`; task 3.2 has not run, so `vilsend_sdk::Clock` is defined in the
SDK instead. **When 3.2 lands this trait is deleted and the core one
re-exported**, which touches one import per use site — the trait is one method.

## Consequences

**Positive**

- The public surface exists and is exercisable today, with no Tauri, no socket
  and no filesystem anywhere in its tree.
- The absence of the outside world is mechanical rather than aspirational: two
  tests fail the build if it appears, one by scanning the source and the
  dependency tree, one by running a whole transfer under a clock that never
  moves.

**Negative**

- The crate carries `sha2` for integrity over a link that is a function call.
  It is the honest way to make `Outcome::verified` mean something, and it is
  the first edge to drop when a real chunk cipher arrives.
- Four Phase 3/6/8 ports (`ChunkSource`, `ChunkSink`, `CredentialStore`,
  `Transport`) are **not** declared here, so the builder's injection surface is
  smaller than `04` §2.1 sketches. That is deliberate: a stub port is a shape a
  caller writes against and that then has to be kept compatible.

**Neutral**

- `Progress` and `TransportKind` are defined in the SDK rather than in
  `vilsend-core`, which is where §4.1 puts a transport's vocabulary. Same
  reason as the clock: the phase that owns the port should define it.

## Related

- [`../05-migration-plan.md`](../05-migration-plan.md) § "Phase 5"
- [`../04-sdk-cli-mobile-build-plan.md`](../04-sdk-cli-mobile-build-plan.md) §1–§2
- ADR-0003 (error model), ADR-0005 (bindings order), ADR-0011 (in-memory
  adapters are a contract)
- [`../reports/phase-5-report.md`](../reports/phase-5-report.md)
