# 06 — Testing and Quality

> Status: **proposal.** Current state: **zero automated tests exist.**
> No `src-tauri/tests/`, no `examples/`, no `benches/`, no `#[test]`, no
> `#[cfg(test)]` anywhere in `src-tauri/src`. Verified by
> `grep -rn "#\[cfg(test)\]" src-tauri/src --include=*.rs` → zero hits.
> The repo's own `docs/ENGINEERING_REVIEW.md:17` scores Testing **1/10**.

---

## 1. Why this document is load-bearing

Every other document in this set proposes a refactor. **None of it is safe
without a test harness.** Specifically:

- Phase 3 splits `transfer/writer.rs` (819 lines) into three modules. There is
  currently **no way to tell whether the split changed behaviour.**
- Phase 4 changes the crypto wire format. Without crypto test vectors, a
  mistake is a silent data-corruption bug shipped to users.
- Phase 8–10 introduce transport failover. Failover is a **concurrency** feature;
  concurrency bugs are the least testable-by-hand class of defect there is.
- Phase 11–12 add three new platforms. Without contract tests, "does the new
  binding work?" is answered by a human clicking through an app.

So: **the harness is not a phase, it is a prerequisite woven through the plan.**
Phase 2 stands up the skeleton; each subsequent phase adds to it.

---

## 2. The test pyramid, adapted to this codebase

```
                    ┌─────────────────────────┐
                    │  E2E (few, slow, real)  │  ~10 tests
                    │  Real app, real net     │
                    ├─────────────────────────┤
                    │ Integration (moderate)  │  ~150 tests
                    │ Two engines in-process  │
                    ├─────────────────────────┤
                    │ Contract (small, vital) │  ~40 tests
                    │ Every impl must pass    │
                    ├─────────────────────────┤
                    │ Unit (many, fast)       │  ~800 tests
                    │ Pure logic, in-memory   │
                    └─────────────────────────┘
```

The **contract layer** is the unusual one and the most valuable here: it is what
makes the ports real rather than decorative.

**Target at end of Phase 5:** ~1000 tests, whole suite under 60 s in CI.
There is no existing suite to migrate, so the target is a greenfield decision.

---

## 3. Contract tests — the core of the strategy

A port with one implementation is an interface. A port with a contract suite is
an **abstraction**. The distinction is the whole reason the refactor is safe.

### 3.1 Transport contract

Every `Transport` implementation must pass this suite **unmodified**. Its
existence is what lets Phase 10 add a LAN transport without re-verifying the
tunnel.

```rust
// crates/transport/tests/contract.rs
//
// Usage in a new transport's test file:
//   transport_contract!(|| LanTransport::new(test_config()), peer_fixture());

#[tokio::test] async fn probe_completes_within_budget() { }
#[tokio::test] async fn probe_is_unavailable_for_absent_peer() { }
#[tokio::test] async fn connect_is_idempotent_per_session() { }
#[tokio::test] async fn connect_after_teardown_succeeds() { }
#[tokio::test] async fn teardown_releases_resources() { }        // fd count, listeners
#[tokio::test] async fn health_reports_dead_after_peer_disappears() { }
#[tokio::test] async fn stream_preserves_byte_order() { }        // 10k random frames
#[tokio::test] async fn partial_frame_does_not_corrupt() { }     // kill mid-frame, reconnect
#[tokio::test] async fn capabilities_match_declared_behaviour() { }  // meta-test, see below
#[tokio::test] async fn respects_peer_unavailability() { }
```

**`capabilities_match_declared_behaviour` is the one that earns its keep.** It
asserts that a transport declaring `end_to_end_direct: true` never contacts an
external endpoint, and that one declaring `works_offline: true` still works with
the network interface down. This is the single most likely lie a new transport
implementation will tell, and it is the one that turns into a privacy incident.

### 3.2 AuthProvider contract

```rust
// crates/auth/tests/contract.rs
#[tokio::test] async fn token_refreshes_before_expiry() { }
#[tokio::test] async fn concurrent_token_calls_refresh_exactly_once() { }
#[tokio::test] async fn definitive_failure_clears_the_session() { }   // 4xx → signed out
#[tokio::test] async fn transient_failure_keeps_the_session() { }     // 5xx → stay signed in
#[tokio::test] async fn logout_clears_all_stored_credentials() { }
#[tokio::test] async fn never_logs_token_material() { }               // see §6.4
#[tokio::test] async fn headless_provider_never_opens_a_browser() { }
```

That last one is important: `ByoAuthProvider` and `ApiKeyProvider` must be
**provably incapable** of prompting. A test that asserts no browser-open call
occurs is cheap and prevents an SDK from hijacking a host application's UI.

### 3.3 CredentialStore contract

```rust
#[tokio::test] async fn set_then_get_round_trips() { }
#[tokio::test] async fn get_missing_returns_none() { }
#[tokio::test] async fn delete_is_idempotent() { }
#[tokio::test] async fn list_returns_all_keys_for_an_account() { }
#[tokio::test] async fn accounts_are_isolated_from_each_other() { }
#[tokio::test] async fn values_survive_a_new_store_instance() { }  // persistence
```

### 3.4 How a contract suite is wired

Keep it a macro, not a trait with default methods — test discovery must be
per-implementation so a failure names the transport.

```rust
// crates/transport/tests/common/mod.rs
#[macro_export]
macro_rules! transport_contract {
    ($name:ident, $factory:expr, $peer:expr) => {
        mod $name {
            use super::*;
            #[tokio::test] async fn probe_completes_within_budget() { /* ... */ }
            // ... all cases
        }
    };
}

// crates/transport/tests/tunnel.rs
transport_contract!(tunnel, || TunnelTransport::new(cfg()), peer_fixture());
```

**How to know the suite is complete:** when Phase 10's `LanTransport` passes it
without a single test needing a special case, the abstraction is correct.

---

## 4. Test doubles

The single biggest enabler. Today, testing the transfer engine requires: a
webview, an OS keychain, a real filesystem, a running cloudflared, and two
machines. That is why there are no tests.

### 4.1 In-memory adapters

| Test double | Replaces | Where |
|---|---|---|
| `InMemoryChunkSource` / `InMemoryChunkSink` | `std::fs` | `vilsend-runtime` (test feature) |
| `MemoryCredentialStore` | OS keychain | `vilsend-runtime` |
| `InMemoryTransport` | a real socket | `vilsend-transport` |
| `FaultyTransport` | — | wraps another transport, injects: latency, drops, stalls, mid-stream resets |
| `RecordingEventSink` | Tauri `emit` | `vilsend-core` |
| `FakeClock` | `tokio::time` | `vilsend-runtime` |
| `MockAuthProvider` | Clerk | `vilsend-auth` |
| `SqliteTransferStore` (temp file) | — | real impl, cheap enough to use directly |

`FaultyTransport` deserves emphasis. It is how you test the failover and resume
logic from [`02-transport-layer.md`](./02-transport-layer.md) §5.4 —
deterministically, without two machines and without flakiness.

```rust
let faulty = FaultyTransport::new(TunnelTransport::new(cfg()))
    .fail_after(Duration::from_secs(2))         // drop the stream mid-transfer
    .with_latency(Duration::from_millis(50));

let vilsend = VilsendBuilder::in_memory()
    .with_transport(Arc::new(faulty))
    .build()?;

// Assert failover happened and the transfer completed without data loss.
```

### 4.2 The `in_memory()` builder is a contract

`VilsendBuilder::in_memory()` must touch **nothing external** — no filesystem,
no keychain, no network, no clock. It is the test entry point.

> **If a test needs a real socket or a real keychain, that is a signal the port
> boundary leaked.** Treat it as an architectural finding, not a testing
> inconvenience. Write it down.

---

## 5. What to test, by layer

| Layer | Test type | Examples | Priority |
|---|---|---|---|
| `vilsend-crypto` | Unit + **fixed vectors** | Encrypt/decrypt round trip; AAD mismatch fails; nonce length enforced; **frozen ciphertext vectors for wire-format regression** | **P0** |
| `vilsend-protocol` | Unit + property | Chunker covers all bytes exactly once; frame encode/decode round trip; malformed frames fail closed; resume bitmap arithmetic | **P0** |
| `vilsend-core` | Unit | `DomainEvent` → wire name (exhaustive match); `ErrorKind` mapping; percentage/ETA arithmetic | **P0** |
| `vilsend-runtime` | Contract + integration | `CredentialStore`, `TransferStore` contracts; path safety | **P0** |
| `vilsend-transport` | Contract | §3.1 | **P0** |
| `vilsend-auth` | Contract + unit | §3.2; PKCE vectors; redirect matching | **P0** |
| `vilsend-engine` | Integration | Failover, resume, concurrency, backpressure, cancellation | **P0** |
| `vilsend-sdk` | Doctest + API snapshot | README example compiles; public API diff | P1 |
| `vilsend-cli` | Integration | Exit codes, `--json` snapshots, end-to-end send | P1 |
| Desktop shell | Smoke | Tauri commands map to SDK calls; events fire | P1 |
| Frontend | Component + unit | Axios interceptor attaches the header; auth context transitions | P2 |

### 5.1 The tests that would have caught existing bugs

Worth writing first, because they are pure regression value with zero design
risk:

| Test | Bug it catches |
|---|---|
| Assert every frontend `invoke("...")` has a registered Rust command | **B1** — `stop_websocket` is invoked but not registered |
| Assert every emitted event has a `listen(...)` somewhere, and vice versa | **B6** — `update-available` is emitted into the void |
| Assert `ConnectionStatus` serialisation matches the frontend's comparison | **B4** — `"Connected"` vs `'connected'` |
| Assert receiver rejects a wrong-but-present `Authorization` | The High finding in `docs/SECURITY.md` |
| Assert a replayed chunk at a different index is rejected | The AEAD has no AAD |
| Assert a truncated file fails verification | There is no whole-file checksum |

**The first three are cheap and can be written in Phase 1**, before any
architecture exists, as a plain script over the source tree. That script is
worth writing on day one.

**Phase 1 note:** the first of the three was written —
`scripts/check-ipc-contract.mjs`, run as `npm run check:ipc`. It reads the
`generate_handler!` list out of `src-tauri/src/lib.rs` and scans `src/` for
`invoke("<name>")` call sites, rather than the generated manifest §7.4
describes; it strips comments first, and it checks command *names* only —
argument names and types are not yet covered. It found two instances of the
B1 defect class: `stop_websocket` (the known one) and `local_transfer_exists`
(not previously documented). The other two script checks — emitted events vs
`listen` calls, and `ConnectionStatus` serialisation — are still unwritten;
the second would have caught the confirmed `server-event` gap.
Summarised in `reports/phase-1-report.md` §7.

---

## 6. CI changes

### 6.1 The pipeline split

`.github/workflows/release.yml` is currently 1799 lines with ~890 lines
commented out. Split it:

```
.github/workflows/
├── ci.yml                  # PR gate — the important one
├── release-desktop.yml
├── release-cli.yml
├── release-bindings.yml
└── release-store.yml
```

### 6.2 `ci.yml` — the PR gate

```yaml
jobs:
  rust:
    steps:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test --workspace --all-features
      - cargo test -p vilsend-sdk --doc

  architecture:
    steps:
      # The load-bearing assertion: the domain must not depend on adapters.
      - run: |
          if cargo tree -p vilsend-core --prefix none \
             | grep -E '^(tauri|reqwest|axum|sqlx|keyring|hyper)'; then
            echo "::error::vilsend-core has an outbound adapter dependency"
            exit 1
          fi
      - run: |
          if cargo tree -p vilsend-transport --prefix none | grep -E '^(tauri|axum|sqlx)'; then
            echo "::error::vilsend-transport must stay Tauri-free"
            exit 1
          fi
      - run: cargo deny check bans licenses

  api-surface:
    steps:
      - cargo semver-checks check-release -p vilsend-sdk   # breaking-change gate
      - cargo public-api dump -p vilsend-sdk > /tmp/now.txt
      - diff /tmp/now.txt crates/sdk/public-api.txt        # reviewable diff

  frontend:
    steps:
      - npm ci
      - npm run typecheck
      - npm run lint
      - npm test                                            # new; there is none today
      - node scripts/check-ipc-contract.mjs                 # §5.1 checks

  wire-compat:
    steps:
      # Build a pinned v1 client, run it against the current receiver.
      - cargo test -p vilsend-engine --test compat_v1
```

### 6.3 Quality gates that do not exist today

| Gate | Why | Phase |
|---|---|---|
| `cargo fmt --check` | Currently unformatted in places | 2 |
| `cargo clippy -D warnings` | ~27 warnings today; zero-tolerance from here | 2 |
| Architecture dependency lint | Turns [`01`](./01-target-architecture.md) §4.1 from prose into a build failure | 2 |
| `cargo-semver-checks` on `vilsend-sdk` | Catches accidental breaking changes | 5 |
| Public API snapshot | Makes API changes visible in review | 5 |
| IPC contract check | Catches B1-class bugs | 1 |
| Wire-compat test vs a pinned v1 binary | Protects the installed base | 4 |
| `cargo deny` (licenses, advisories, duplicate deps) | Supply chain | 3 |
| Frontend unit tests | None exist | 3 |

### 6.4 Secret-redaction lint

Token material must never appear in logs. Enforce it:

```rust
#[test]
fn tracing_events_never_contain_token_material() {
    // Install a capturing subscriber, run a full auth + transfer flow,
    // assert no captured field contains the literal token string.
}
```

This is worth its weight because the current codebase already logs token
existence and length (`src/lib/api.ts:14-44`), so the pattern is established and
would otherwise be copied.

---

## 7. Frontend testing

Currently: `npm run typecheck` and `npm run lint` only. No test runner.

**Recommendation: Vitest + React Testing Library.** Vite is already the
bundler, so Vitest is the low-friction choice.

Priorities, in order:

1. **The axios interceptor** (`src/lib/api.ts:14-44`) — this is the single most
   security-relevant frontend code path. Test: token attached when present,
   `X-Device-Id` always attached, no `Authorization` header when the token is
   null, no token logged on failure.
2. **`auth-context.tsx` state machine** — bootstrap, sign-in, timeout, error,
   logout. Mock the Tauri IPC boundary.
3. **`ensureTunnelHostname()`** (`src/features/tunnel/tunnel-hostname.ts:14-41`)
   — keychain hit, API fallback, write-back.
4. **`api/tauri.ts`** — assert every command name it invokes exists in a
   committed manifest of Rust commands (generated in CI from
   `generate_handler!`). **This is the B1 fix.**

**Do not** write component snapshots for the ~60 shadcn/ui primitives. They
come from a generator; snapshotting them is pure noise.

---

## 8. End-to-end testing

Slow, few, and worth it. These are the tests that answer "does it actually work?"

| Test | What it proves |
|---|---|
| Two `Vilsend` instances in one process, real loopback sockets, `in_memory()` stores: complete a 100 MB transfer | The core works without Tauri |
| Same, but kill the transport mid-transfer | Failover + resume work |
| Same, but with `FakeClock` and forced retries | Backoff is correct |
| Launch the built desktop app, sign in, send a file | The shell works |
| `vilsend send` from a shell, assert the file arrives | The CLI works |
| **v1 client ↔ v2 receiver**, and **v2 client ↔ v1 receiver** | Wire compatibility |
| macOS arm64: cloudflared starts | Catches the Phase 1.13 bug |

Run the first four on every PR. The last three nightly.

**Tooling:** start with plain `cargo test` integration tests. Add Playwright or
WebdriverIO for the desktop-UI E2E **only** when the shell itself needs
verification — the `tauri-driver` story is workable but not free, and the core
is where the risk is.

---

## 9. Test hygiene rules

| Rule | Rationale |
|---|---|
| **No `sleep` for synchronisation.** Use `FakeClock` or poll-with-timeout. | Flaky tests are worse than no tests |
| **No test may touch the real keychain or the real network.** | They cannot run in CI, and they leak state between runs |
| **Every bug fix lands with a failing-first test.** | Otherwise the bug returns |
| **Contract tests are never weakened to make an implementation pass.** | If a transport cannot satisfy the contract, the contract or the transport is wrong — escalate, do not special-case |
| **Test names state the invariant, not the method.** `rejects_replayed_chunk_at_different_index`, not `test_receive_2`. | A failing test should explain itself |
| **No snapshot tests on generated UI primitives.** | Noise, not signal |

---

## 10. Definition of "green" for a phase

A phase is not done until:

- [ ] `cargo test --workspace` passes, with the test count **higher than before**.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- [ ] The architecture lint passes.
- [ ] Any new port has a contract suite and **≥ 2 implementations**.
- [ ] The desktop app still completes a real transfer (manual checklist).
- [ ] No phase introduces a test that is `#[ignore]`d without an issue reference.

---

## See also

- [`02-transport-layer.md`](./02-transport-layer.md) §6.1 — the transport contract suite
- [`05-migration-plan.md`](./05-migration-plan.md) — which phase adds which test
- [`01-target-architecture.md`](./01-target-architecture.md) §4.2 — the architecture lint
- [`adr/0011-testing-and-contract-suites.md`](./adr/0011-testing-and-contract-suites.md)
