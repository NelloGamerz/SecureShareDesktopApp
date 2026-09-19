# ADR-0011 — Contract test suites and in-memory adapters

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 2 (harness), 3, 8 (contract suites)
- **Supersedes:** —

## Context

**There are zero automated tests.** No `src-tauri/tests/`, no `examples/`, no
`benches/`, no `#[test]`, no `#[cfg(test)]` anywhere in `src-tauri/src`.
The repo's own `docs/ENGINEERING_REVIEW.md:17` scores Testing **1/10** and lists
"no meaningful automated tests were found" as a top-10 problem.

This is not merely a quality gap — it is a **blocker for the migration**:

- Phase 3 splits `transfer/writer.rs` (819 lines, six responsibilities) into
  three modules. Nothing can tell whether that changed behaviour.
- Phase 4 changes the crypto wire format. A mistake ships silent data corruption.
- Phases 8–10 introduce transport failover — a concurrency feature, and
  concurrency bugs are the least hand-testable class of defect.
- Phase 12 adds two platforms.

Today, testing the transfer engine requires a webview, an OS keychain, a real
filesystem, a running cloudflared, and two machines. That is why there are no
tests, and it is the thing the port extraction fixes.

## Options considered

**A. Test through the Tauri IPC boundary (integration only).**
Rejected as the primary layer. Slow, needs a webview, and cannot cover failover
deterministically.

**B. Unit tests per module, no shared suites.**
Insufficient. It does not make the ports real — each implementation would be
tested against its own ad-hoc expectations.

**C. Contract suites per port, plus in-memory adapters.**
**Chosen.**

**D. Property-based testing as the primary approach.**
Adopted as a **supplement** for the protocol layer (chunker coverage, frame
round-tripping), not as the primary strategy.

## Decision

### 1. Contract suites are the core artefact

Every port gets a suite that **every implementation must pass unmodified**:

- `Transport` — 9 tests including a **capability meta-test** asserting that a
  transport declaring `end_to_end_direct: true` never contacts an external
  endpoint, and one declaring `works_offline: true` works with the network down.
- `AuthProvider` — including a test that headless providers are **provably
  incapable** of prompting.
- `CredentialStore` — including account isolation and persistence.

**A port with one implementation is an interface. A port with a contract suite
is an abstraction.** The distinction is the entire reason the refactor is safe.

**Contract tests are never weakened to make an implementation pass.** If a
transport cannot satisfy the contract, either the contract or the transport is
wrong. Escalate; do not special-case.

### 2. In-memory adapters for everything

`InMemoryChunkSource`/`Sink`, `MemoryCredentialStore`, `InMemoryTransport`,
`FaultyTransport`, `RecordingEventSink`, `FakeClock`, `MockAuthProvider`.

`FaultyTransport` deserves emphasis: it is how failover and resume are tested —
deterministically, on one machine, without flakiness.

### 3. `VilsendBuilder::in_memory()` is a contract, not a convenience

It must touch **nothing external** — no filesystem, no keychain, no network, no
real clock.

> **If a test needs a real socket or a real keychain, that is an architectural
> finding, not a testing inconvenience.**

### 4. Test hygiene rules

- **No `sleep` for synchronisation.** Use `FakeClock` or poll-with-timeout.
  Flaky tests are worse than no tests.
- No test may touch the real keychain or the real network.
- Every bug fix lands with a **failing-first** test.
- Test names state the invariant, not the method.
- No snapshot tests on the ~60 generated shadcn/ui primitives.

### 5. Enforce the architecture in CI

```bash
cargo tree -p vilsend-core --prefix none | grep -E '^(tauri|reqwest|axum|sqlx|keyring)'
```

fails the build. This turns ADR-0002 from prose into a gate.

### 6. Fix the existing bugs with tests first

Three cheap tests that can be written in Phase 1, before any architecture exists:

| Test | Bug |
|---|---|
| Every `invoke("...")` has a registered Rust command | `stop_websocket` is invoked but unregistered |
| Every emitted event has a `listen(...)`, and vice versa | `update-available` is emitted into the void |
| The `ConnectionStatus` serialisation matches the frontend's comparison | `"Connected"` vs `'connected'` |

## Consequences

**Positive**

- Phase 3's `writer.rs` split is verifiable: characterisation tests pin the
  current behaviour (including its bugs), then the refactor must preserve them.
- A new transport is provably correct before it ships.
- The core becomes testable without a webview for the first time.

**Negative**

- The contract suites are real work — budget roughly 40 tests before the first
  transport is "done".
- In-memory adapters are additional code to maintain. Worth it: they are the
  difference between a testable engine and an untestable one.

**Neutral**

- The stated target (~1000 tests, under 60 s in CI by Phase 5) is a greenfield
  decision — there is no existing suite to migrate.

## Related

- [`../06-testing-and-quality.md`](../06-testing-and-quality.md)
- [`../02-transport-layer.md`](../02-transport-layer.md) §6.1
- ADR-0001, ADR-0002, ADR-0004
