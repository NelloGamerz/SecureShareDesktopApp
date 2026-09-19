# ADR-0001 — Extract a platform-agnostic core from the Tauri app

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 2
- **Supersedes:** —

## Context

The repository is a single Rust crate (`src-tauri/`) that is simultaneously the
business logic and the Tauri shell. There is no Cargo workspace.

The mandate is to ship the same logic as a desktop app, an SDK, a CLI, and a
mobile app. That is not achievable from a crate that depends on Tauri.

**But the coupling is narrower than the mandate implies.** Verified counts of
`tauri::` references per file show the transfer engine touches Tauri in exactly
three places:

- `transfer/events.rs:2` — `app.emit(name, progress)` (event emission)
- `transfer/manager.rs:20,31` — an `AppHandle` field used only for emission
- `transfer/writer.rs:12-13,39,68` — `AppHandle` for store access, download-dir
  resolution, secure storage, and emission

Everything else — `crypto`, `chunker`, `merger`, `scheduler`, `upload`,
`scanner`, `http_client`, `progress`, `retry`, `state`, `errors`, `constants`,
the whole `websocket/` module, the whole `utils/` module — is already Tauri-free.

## Options considered

**A. Rewrite from scratch as a workspace.**
Rejected. The crypto is sound, the PKCE flow is sound, the WebSocket client is
sound. A rewrite discards working code and re-introduces known-bad bugs.
There are also **zero tests**, so a rewrite has no safety net.

**B. Keep one crate; add `#[cfg]` gates for the shell.**
Rejected. `cfg` gates do not create a dependency boundary — a shell type can
still be referenced from domain code, and nothing fails until you try to build
the CLI.

**C. Incremental strangler extraction into a Cargo workspace.**
**Chosen.**

## Decision

Create a virtual Cargo workspace. Extract a `vilsend-core` crate containing
domain types, the error model, and **all port traits**, with a hard dependency
rule: `vilsend-core` may not depend on `tauri`, `reqwest`, `axum`, `sqlx`, or
`keyring`. Enforce it in CI with `cargo tree`, not by convention.

Proceed in the order: workspace → core + `EventSink` port (**walking
skeleton**) → resource ports → protocol hardening → SDK facade. Each step is
independently mergeable, and the desktop app ships at every step.

## Consequences

**Positive**

- The core becomes unit-testable without a webview. Today it is not testable at
  all, which is why there are zero tests.
- SDK, CLI, and mobile become possible without forking logic.
- The dependency lint turns an architectural principle into a build failure.

**Negative**

- A workspace adds ceremony: `Cargo.lock` moves, CI cache paths change, and the
  release workflow's hardcoded `src-tauri/` paths must all be updated in one
  commit (**R-06**).
- More crates means more places to declare dependencies.

**Neutral**

- The extraction happens regardless of the SDK/CLI/mobile mandate, because it is
  also the prerequisite for testing the receiver-auth fix (Phase 9) and the
  protocol hardening (Phase 4).

## Related

- [`../01-target-architecture.md`](../01-target-architecture.md) §4
- ADR-0003 (error model), ADR-0012 (events), ADR-0011 (testing)
