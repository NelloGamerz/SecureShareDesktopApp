# Phase 5 — `vilsend-sdk` facade · Report

**Phase:** `5` — tasks 5.1, 5.2, 5.4, 5.5, 5.6. **5.3 is not implemented** and
cannot be — see §1 and §8.
**Branch:** `phase-5-sdk-facade` (from `phase-4a-protocol-crypto-v2` @ `f857e60`)
**Commits:** seven — `f96b62a` … `4c3059e`
**Plan:** [`05-migration-plan.md`](../05-migration-plan.md) § "Phase 5"
**Read:** `README.md`, `05` § Phase 5 and the phase overview, `04` §1–§4,
`01` §4 and §6, ADR-0003, ADR-0005, ADR-0011, and the Phase 1, 2 and 4a reports
including their "Decisions needed".
**Diff:** 26 files, 5,874 insertions, 1 deletion — of which 5,477 lines are the
new `crates/sdk`, and 1,157 of those are tests or a generated snapshot.

---

## 1. Summary

Phase 5 creates `vilsend-sdk`, the stable public API, with four verbs
(`send`, `receive`, `auth`, `events`), a working `in_memory()` backend, a
written-down semver policy, and the two CI gates that keep the surface from
moving by accident.

**Everything in the phase that does not need the transfer engine is done.
Task 5.3 — "refactor the Tauri commands to call `vilsend-sdk`" — is not, and
the reason is the same one Phase 4a's report gave: Phases 3 and 4 have not been
implemented, and Phase 5 is blocked by Phase 3.**

Concretely, and this is the thing to read before anything else:

`05-migration-plan.md`'s dependency graph is `P3 → P5`. `crates/` holds `core`,
`sdk` and `desktop`. There is no `vilsend-engine`, no `vilsend-runtime`, and no
`ChunkSource`, `ChunkSink`, `CredentialStore`, `TransferStore`, `Transport` or
`AuthProvider` port anywhere in the repository. `transfer/writer.rs` still takes
a `tauri::AppHandle`; `UploadManager` still holds a `LocalTransferFileService`
that needs a SQLite pool built from a Tauri app handle.

So the desktop crate cannot become a consumer of the SDK, because there is
nothing behind the SDK's `native()` for it to consume. The consequences:

| | |
|---|---|
| `VilsendBuilder::native()` **refuses to build** | With an error naming `vilsend-runtime` and `vilsend-engine`. A client that built and then failed at every call would turn a missing crate into a runtime surprise three call sites later. |
| `examples/send.rs` completes an **in-memory** transfer | Real chunking, real integrity, real handles, real outcomes — but not over a receiver process. |
| The desktop's commands are **unchanged** | Which is also why the golden payloads still pass: nothing was touched. |

**A second thing to read:** §8 item 2. `#[non_exhaustive]` was added to
`vilsend_core::TransferStatus`, which is a change in a crate Phase 5 does not
own. It is justified below, but it is a deviation and it is the only one.

---

## 2. Per-task status

| Task | Commit | Status | Notes |
|---|---|---|---|
| 5.1 create `crates/sdk` | `f96b62a` | ✅ done | The four verbs, the backend seam, the in-memory engine |
| 5.1b the send→receive integration test | `4c3059e` | ✅ done | Not a plan task; the acceptance criterion. See §11.3 |
| 5.2 `native()` / `in_memory()` | `bf8dd26` | ⚠️ half | `in_memory()` works and its contract is enforced. `native()` is declared and refuses. §8 item 1 |
| 5.3 desktop calls the SDK, no direct `vilsend-engine` dependency | — | ❌ **not implemented** | Blocked by Phase 3. The lint the brief asks for *is* in CI (§5.4) |
| 5.4 `#[non_exhaustive]` + semver policy | `cc4d38f` | ✅ done | Plus `d16e57e`, which closed a gap the check found in `vilsend-core` |
| 5.5 `cargo-semver-checks` in CI | `febfcb8` | ✅ done | Plus the public-API snapshot. Both verified locally |
| 5.6 README example as a doctest | `b5b4bff` | ✅ done | Plus `examples/send.rs` and `smoke-checklist.md` |

The commit order is not the task order: 5.6 was committed before 5.4b because
5.4b exists only because 5.4's own check found something.

---

## 3. Acceptance criteria

### 3.1 ✅ `cargo test -p vilsend-sdk --doc` passes

**Pass.** One doctest, and it is the README. `lib.rs` opens with
`#![doc = include_str!("../README.md")]`, so the example a reader sees on
crates.io is the example the test runner compiles and executes. There is no
second copy to drift.

```
running 1 test
test crates\sdk\src\lib.rs - (line 16) ... ok
test result: ok. 1 passed; 0 failed
```

The example runs a full send→receive cycle, drains nothing, asserts the bytes
and the auth state, and uses `futures::executor::block_on` — no `tokio::`
appears in it, or in the crate.

### 3.2 ✅ An `in_memory()` integration test does a full send→receive cycle

**Pass.** `crates/sdk/tests/in_memory_cycle.rs`, 15 tests, over the public API
only — it is an integration test, so it *cannot* see a private module.

It covers: a multi-file multi-chunk round trip byte for byte; a zero-byte file
(the chunker's `max(1)`); a destination prefix; `NoRoute` for a send with no
listener and for a second send to a used listener; a listener cancelled before
a send; a listener dropped before a send (the `Drop`-cancels contract); a
receive cancelled *while the transfer runs*; `NotFound` for a missing source
file, and that it costs no listener; `InvalidInput` for an empty file list; the
event stream; a host `EventSink`; and a terminal `Progress` sample.

### 3.3 ⚠️ A ~20-line example completes a real transfer

**Pass, with the word "real" qualified.** `crates/sdk/examples/send.rs` is 22
lines of code and runs:

```
$ cargo run -p vilsend-sdk --example send
sent 12 bytes in 1 chunk(s)
received: hello, world
```

It completes a real transfer over the in-memory backend. It does **not** go over
a network, and the file's own doc comment says so in the first sentence rather
than leaving a reader to find out at the first `native()` call. Running it
against a real receiver is manual step §10.3 and is blocked by Phase 3.

### 3.4 ⚠️ The desktop app has no direct dependency on `vilsend-engine`

**Passes, vacuously.** `vilsend-engine` does not exist, so no crate can depend
on it. The lint the brief asks for is in CI anyway
(`.github/workflows/ci.yml`, "The desktop crate must not depend on the engine")
in the negated form, with a "guards the guard" step — and its comment says
plainly that it passes vacuously today. A lint that says nothing about today and
everything about the day Phase 3 lands is worth having; a lint that pretends to
be doing work is not.

### 3.5 ✅ The Tauri command/event payloads are unchanged

**Pass, by not touching them.** `git diff f857e60 HEAD -- crates/desktop/src/`
is **empty**. The only change to `crates/desktop` on this branch is four lines
of `Cargo.toml` adding `publish = false`.

The golden fixtures (`crates/core/tests/golden_payloads.rs`, regenerated by
`crates/desktop/src/golden.rs`) pass unchanged in both feature configurations.

### 3.6 ✅ The remaining brief constraints

| Constraint | Where it is enforced |
|---|---|
| Four verbs only | `Vilsend` has exactly `send`, `receive`, `auth`, `events`. No fifth was needed, so none was added. |
| `in_memory()` touches nothing external, and a test fails if it does | `crates/sdk/tests/no_external_touch.rs` — §5.2 |
| Every public enum `#[non_exhaustive]` | `crates/sdk/tests/public_api.rs`, and for re-exported enums, `vilsend-core`'s source too |
| No public type exposes `tauri`, `axum`, `sqlx`, or a lifetime | Same file, plus the dependency-tree scan |
| `cargo-semver-checks` and a public-API snapshot in CI | §5.4 |
| `crates/sdk/README.md` with the semver policy, example as a doctest | §3.1 |
| `publish = false` on every crate | `vilsend-core`, `vilsend-sdk` had it; **`vilsend` (the desktop app) did not** and now does (`d16e57e`) |
| Nothing published | Nothing was |

---

## 4. What the SDK actually is

```
crates/sdk/src/
├── lib.rs          Vilsend, VilsendBuilder, LOOPBACK_PEER, the re-export block
├── request.rs      SendRequest, ReceiveRequest, Policy
├── handle.rs       TransferHandle, Outcome, Sent/ReceivedOutcome, ReceivedFile
├── progress.rs     Progress, TransportKind, DegradedReason
├── events.rs       EventStream, and the bounded fan-out behind it
├── auth.rs         AuthFacade, AuthState
├── clock.rs        Clock, FakeClock
├── files.rs        MemoryFiles
├── ids.rs          FileRef, Destination
├── state.rs        the shared state behind one handle
└── backend/
    ├── mod.rs      the Backend seam
    └── memory.rs   the in-memory engine
```

**How a transfer happens in memory.** There is no wire: the two halves are two
objects in one process, and the transport is a method call that hands a chunk's
bytes and its SHA-256 to the other side. `receive` registers a listener and
allocates its id (it has to — `TransferHandle::id` is synchronous, and a handle
must answer before anything arrives). `send` claims the earliest listener on
that peer, first in first served, and adopts its id, so both handles name the
same transfer. Chunks are cut, digested, handed over, counted and
acknowledged; the receiving side assembles each file in memory and writes it to
the sink only once every chunk has arrived — the shape the shipped receiver's
`.part`-then-merge has, minus the filesystem.

A `send` with no listener is `ErrorKind::NoRoute`, which is the first real
producer that error kind has had.

---

## 5. Design decisions

Each of these is a place the plan or the ADRs left something open.

### 5.1 `native()` fails at `build()`, rather than building a client that cannot work

`build()` already returns `Result`, and "there is no native backend in this
build" is a fact about configuration, not a runtime condition. Handing back a
`Vilsend` whose every call fails would move the surprise three call sites later
and make it look like a transfer problem. A test pins the message, including
that it names `vilsend-runtime`, `vilsend-engine` and `in_memory()`.

The kind is `ErrorKind::Internal`. ADR-0003 warns against climbing into
`Internal` instead of naming a real condition; this is not a condition, it is a
missing crate, and the alternative kinds (`NotConnected`, `InvalidInput`) would
be worse lies.

### 5.2 `in_memory()`'s contract is enforced two ways, because one is not enough

ADR-0011 §3: "no filesystem, no keychain, no network, no real clock."

* **Statistically** — the crate's own source, comments stripped, may not name
  `std::fs`, `std::net`, `std::time::SystemTime`, `std::time::Instant`,
  `std::path::Path`, `std::path::PathBuf`, `std::env` or `std::process`; and
  `cargo tree -p vilsend-sdk` may not contain `tauri`, `reqwest`, `axum`,
  `sqlx`, `keyring`, `tokio`, `hyper`, `rustls`, `ureq` or `curl`. The scan
  cannot see a `SystemTime` reached through a dependency.
* **Behaviourally** — a whole transfer runs under a `FakeClock` that never
  moves and must report the throughput that clock implies, zero. One
  `Instant::now()` in the measured path makes it non-zero. The test cannot see a
  stray `PathBuf` that nothing calls yet.

The comment stripper is itself tested, and it handles the two things that would
make it a false-negative machine: Rust's nested block comments, and a `//`
inside a string literal.

### 5.3 The fault injection: `Clock` is a port, and it is defined in the SDK

Since `in_memory()` may not read the real clock, throughput and ETA need an
injected one. `05-migration-plan.md` task 3.2 puts a `Clock` port in
`vilsend-core`; task 3.2 has not run, so `vilsend_sdk::Clock` exists instead.
ADR-0013 records it as a thing to **delete** when 3.2 lands — it is one method,
so that is one import per use site.

### 5.4 `Policy` precedence, which `04` §2.2 leaves undefined

`SendRequest` carries both `chunk_size`/`concurrency`/`max_retries` **and** a
whole `policy`, without saying which wins. The rule chosen, documented in
`Policy::merged_with` and pinned by tests: **a field named on the request beats
the request's `policy`, which beats the builder's policy.** The narrower
statement wins.

### 5.5 `Policy` is the one public struct that is not `#[non_exhaustive]`

The brief and `04` §2.5 ask for `#[non_exhaustive]` on public **enums**. A
configuration struct is a different thing: marking it non-exhaustive forbids
`..Policy::default()`, which is the whole reason its `Default` impl is useful.
Adding a field to it is therefore a breaking change — the right trade for a
config value, and written into the README rather than left to be discovered from
a compile error.

### 5.6 Events are `vilsend_core::DomainEvent`, on a bounded queue

One event vocabulary in the product (ADR-0012); the SDK does not get a second
one. The `Item` is the same enum the desktop shell's `EventSink` carries, and
`VilsendBuilder::with_event_sink` feeds a host sink and the streams at the same
time, so push and pull are not alternatives.

The per-subscriber queue is **bounded** at 1,024, with the count of what was
dropped visible on the stream. Phase 4 task 4.9 records that "unbounded queues
are an OOM reachable from a LAN peer"; a fan-out with an unbounded queue would
put the same reachable OOM in the SDK. The test asserts the invariant that
matters and that does not encode the channel's exact capacity: *every event
emitted is either delivered or counted as dropped*.

### 5.7 The engine yields between chunks, and that is load-bearing

`std::task::yield_now` does not exist (it is `std::thread::yield_now`, a
different thing), the SDK has no runtime to borrow one from, and
`futures` is a dev-dependency. So `state.rs` has a twelve-line `yield_once`
built on `std::future::poll_fn`.

Without it a `send` future never returns `Pending`, so a `cancel` polled beside
it could not run until the transfer was already over — and `cancel` would be
untestable and its contract a fiction. With it, the cancellation test in §3.2
is deterministic rather than a race.

`std::future::poll_fn` is also what backs the handle's wait/wake. The SDK never
reaches for `tokio::sync::Notify`; `01` §6.1 requires the public signatures to
be runtime-agnostic, and hardcoding a runtime in the guts while the `async fn`s
look neutral would not be.

---

## 6. Limits and gaps

1. **`pause` and `resume` move the reported status and nothing else.** There is
   no state in which the in-memory link is blocked. The reason is structural:
   pausing would mean a sender parked until a future the caller has no way to
   poll *together with it* ran, which is a deadlock with extra steps. Recorded
   in the backend's module docs rather than implemented halfway.

2. **A transfer in flight can only be cancelled from the receiving handle.**
   The sending handle does not exist until `send` returns, and `send` returns
   when the transfer is over. This is a consequence of eager execution, not an
   oversight — and an earlier draft of this code *did* have a
   `sender.is_cancel_requested()` branch in the engine loop, which was
   unreachable. It was deleted rather than shipped: rule 10.

3. **`Policy::concurrency` and `max_retries` are validated and ignored.** The
   in-memory link never fails and the chunks go over in order; a retry here
   would be a retry of a function call. They become real with the `Transport`
   port in Phase 8.

4. **`Progress` and `vilsend_core::TransferProgress` are not bridged.** `04`
   §2.3's `Progress` is the SDK's richer view; the core one is the frozen wire
   payload. Nothing consumes both today, so the bridge is a mapping function
   nobody needs yet — but §5.3's desktop-consumption task is exactly what will
   need it, and the temptation will be to change the wire payload instead. It
   must not.

5. **`TransportKind` is defined in the SDK**, not in `vilsend-core` where
   `01` §4.1 puts a transport's vocabulary, for the same reason as `Clock`.
   ADR-0013 records the move-down.

6. **`auth()` is the shape of a verb and nothing more.** `04` §2.2 gives
   `Vilsend` four verbs and one of them is `auth()`. Phase 6 owns
   authentication; inventing a token lifecycle here would give Phase 6 two
   things to reconcile instead of one thing to build. `AuthFacade::state()`
   returns `SignedOut` for the in-memory backend, structurally — a session needs
   a credential store and a network, and `in_memory()` may touch neither — and a
   test pins it.

7. **The public-API snapshot contains `serde_core`'s generated impls.** `-ss`
   drops blanket and auto-trait impls but not derived ones (dropping those would
   hide a lost `Debug`). A serde major bump may therefore produce a snapshot
   diff that is not an API change for the SDK. Regenerating is one command, and
   the alternative — `-sss` — would hide a type moving between modules.

8. **`cargo doc` is not wired**, and `#![warn(missing_docs)]` is on, so the
   crate is fully documented by construction; but nothing builds the docs in CI,
   so a broken intra-doc link would not fail. Same gap Phase 4a recorded.

---

## 7. Defects found

### D1 — `cargo-semver-checks` silently skips `publish = false` workspace members

**This is the one that matters**, because it is a CI gate that looks green while
checking nothing.

`vilsend-sdk` is `publish = false` (ADR-0005). cargo-semver-checks treats an
unpublished workspace member that was *not explicitly selected* as out of scope:
it prints `Skipping vilsend-sdk v0.1.0 (current)` and **exits 0 having run zero
checks**. Selecting the package with `-p vilsend-sdk` is what stops that.

Verified both ways on this machine:

```
$ cargo semver-checks check-release --manifest-path crates/sdk/Cargo.toml --baseline-rev 75f6ac5
     Cloning 75f6ac5
$ echo $?
0
```

```
$ cargo semver-checks check-release -p vilsend-sdk --manifest-path crates/sdk/Cargo.toml --baseline-rev 75f6ac5
...
    Checking vilsend-sdk v0.1.0 -> v0.1.0 (no change; assume minor)
     Checked [   0.017s] 196 checks: 196 pass, 58 skip
     Summary no semver update required
```

The CI job therefore passes `-p vilsend-sdk` **and** asserts that a summary line
was printed, so a skip fails the build. A skipped step and a passing step look
identical in a log; this is the only thing that makes them distinguishable.

### D2 — `04` §2.2's `SendRequest` carries the same three knobs twice

`chunk_size`, `concurrency` and `max_retries` sit beside a whole
`policy: Option<Policy>` with no stated precedence. §5.4 chose one and documented
it. The doc should either drop the three fields or say which wins.

### D3 — `04` §2.1's builder lists ports that do not exist

`with_auth`, `with_transport`, `with_store`, `with_credentials`, `api_base` and
`protocol_version` are all Phase 3/6/8 seams. The doc reads as if the builder can
be configured today. It cannot, and none of them was stubbed: a stub port is a
shape a caller writes against and that then has to be kept compatible.

### D4 — `04` §2.3 and `vilsend_core::progress` are two progress types with no bridge

See §6.4. Not a bug yet; a trap for whoever wires the desktop to the SDK.

### D5 — `05`'s dependency graph says `P3 → P5`, and the docs elsewhere disagree about when the SDK was due

`04` §1 says "the desktop shell should be refactored to call `vilsend-sdk` **in
Phase 4**". `01` §6.3 and ADR-0005 say the Rust crate is Phase **4** and the CLI
is Phase **5**. `05` says the SDK is Phase 5 and the CLI is Phase 7. Three
different plans; the repository built to `05`. Recorded, not resolved.

### D6 — Phase 4a's D1 is still open

The transfer-key hash logged at `INFO` in `manager.rs:111-116` and
`writer.rs:522-527`. Six lines, no wire impact, no test depends on it. Still not
Phase 5's (rule 2), still unfixed, still worth someone's ten minutes.

### D7 — the `smoke-checklist.md` Phase 2 asked for now exists

Phase 2's report §8 decision 7 asked for it, and this phase's brief names it
again as a manual step. It is now at `docs/migration/smoke-checklist.md`, with a
section per phase rather than a single flat list, and every step says what
failure looks like.

---

## 8. Decisions needed

1. **Phase 5 is blocked by Phase 3, and task 5.3 cannot be done.** This is the
   headline. `01` §4.1 puts the native adapters in `vilsend-runtime` and the
   engine in `vilsend-engine`; neither exists. Concretely:
   * **5.3 cannot be implemented as written.** "Refactor the Tauri commands to
     call `vilsend-sdk`" requires the engine to be behind the SDK, which
     requires Phase 3's ports (3.1–3.7) and the `crates/runtime`/`crates/engine`
     split.
   * **`native()` cannot be built**, so `04` §1's "eat your own dogfood" has not
     happened, and the plan's Phase 5 acceptance "only on `vilsend-sdk`" is not
     a statement anyone can make yet.
   * **`examples/send.rs` runs in memory**, and the manual step "run it against
     a real receiver" has nothing to run against.
   * Options: (a) implement Phase 3, then finish 5.3; (b) treat this as a replan
     and fold 5.3 into Phase 3's sub-phase that extracts the engine; (c) accept
     the SDK as surface-only until Phase 3 and move on to Phase 6, which is
     blocked by Phase 2 and *is* runnable. **This is the blocking one.**

2. **`#[non_exhaustive]` was added to `vilsend_core::TransferStatus`**
   (`d16e57e`). `vilsend-sdk` re-exports it — `Progress::status` is that type —
   so a caller matching on it exhaustively would break the day a variant is
   added, whatever the SDK's own enums do. Marking it changes nothing on the
   wire and the golden fixtures still pin the eight variants exactly. But it is
   a change in a crate Phase 5 does not own. Keep it, or revert and let Phase 6+
   decide? `ConnectionStatus` was deliberately left alone because the SDK does
   not re-export it.

3. **`publish = false` was added to the `vilsend` (desktop) crate.** The brief
   says "on every crate"; the application did not have it. Harmless, but it
   changes a file the phase otherwise does not touch.

4. **The SDK's `Clock` port duplicates Phase 3 task 3.2's.** ADR-0013 says the
   SDK's is deleted when 3.2 lands. If 3.2 is being replanned (item 1), it may be
   cheaper to define `Clock` in `vilsend-core` now and have the SDK re-export it.
   Which?

5. **`04` §2.3's `Progress` and `vilsend_core::TransferProgress`** (§6.4, D4).
   The desktop-consumption task needs a mapping function. Confirm the rule now —
   *the wire payload never changes to suit the SDK* — or it will be decided by
   accident later.

6. **Three docs disagree about which phase builds the SDK** (D5: `04` §1 and
   ADR-0005 say Phase 4, `05` says Phase 5). Should `04` and ADR-0005 be
   corrected to match `05`, or is `05` the one that moved?

7. **`04` §2.2's duplicated policy fields** (D2) — drop the three fields, or
   keep them and write the precedence into the doc? The implementation documents
   it either way; the question is what the *doc* should say.

8. **Phase 1's, 2's and 4a's open items are still open** and unchanged. Phase 4a
   §8 items 1–6, Phase 2 §8 items 1–4 and 6–8, Phase 1's D2/D3/D4/D6.

---

## 9. Doc/code discrepancies

Where the documents and the code disagree, the code wins and the document is
wrong. None is corrected here — rule 2 and §8.

| Doc | Says | Code / reality |
|---|---|---|
| `05` Phase 5 | `P3 → P5` | True, and Phase 3 has not run — §8 item 1 |
| `04` §1, `01` §6.3, ADR-0005 | the Rust SDK crate is Phase 4 | `05` puts it in Phase 5, and that is what was built (D5) |
| `01` §4.1 | the SDK re-exports `engine`, `runtime`, `auth`, `transport`, `core` | Three of the five do not exist; the SDK depends on `core` and defines two small ports of its own |
| `01` §4.1 | `TransportKind` and the ports belong in `vilsend-core` | `TransportKind` and `Clock` are in the SDK, pending their phases |
| `04` §2.1 | the builder takes `with_auth`, `with_transport`, `with_store`, `with_credentials`, `api_base`, `protocol_version` | None exists; none was stubbed (D3) |
| `04` §2.2 | `SendRequest` has both explicit knobs and a `policy`, unranked | Precedence is now defined and tested (§5.4) |
| `04` §2.5 | "`#[non_exhaustive]` on all public enums" | Enforced for the SDK's own and its re-exports; `Policy` is the one documented exception (§5.5) |
| `06` §5, `06` §6.2 | `vilsend-crypto`, `cargo test -p vilsend-engine` | Unchanged from Phase 4a's report — same discrepancy |
| `05` Phase 5 acceptance | "The desktop app has no direct dependency on `vilsend-engine` — only on `vilsend-sdk`" | Not achievable today; the lint is in place for when it is |

---

## 10. Manual steps for the human

1. **Decide §8 item 1** — the Phase 3 sequencing. Nothing else in the phase list
   is blocked on it, but Phase 5's remaining task is.
2. **Run the smoke checklist**: `docs/migration/smoke-checklist.md` §5.1
   (`cargo run -p vilsend-sdk --example send`), §5.2
   (`cargo test -p vilsend-sdk --doc`), §5.5 (the public-API gates).
3. **Run `examples/send.rs` against a real receiver — blocked.** The step is
   written down in the checklist §5.3 so it is not quietly dropped, and it needs
   Phase 3 first.
4. **Confirm §8 items 2–7**, all of which are small and none of which changes
   behaviour.
5. **Optionally verify CI on this branch.** The gates are green locally; the
   `public-api` job is new and has never run on a runner. Locally:

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace
   cargo test --workspace --all-features
   cargo test -p vilsend-sdk --doc
   cargo public-api --manifest-path crates/sdk/Cargo.toml -ss \
     | diff -u crates/sdk/public-api.txt -
   npm run typecheck && npm run build && npm run check:ipc
   ```

---

## 11. Test and gate evidence

### 11.1 Counts

Baseline at `f857e60`: **70** tests in the default configuration, **125** with
`--all-features`.

At `4c3059e`:

| Configuration | Tests | Result |
|---|---|---|
| `cargo test --workspace` | **178** | all pass |
| `cargo test --workspace --all-features` | **233** | all pass |
| `cargo test -p vilsend-sdk --doc` | 1 | pass |

The 108 new default-configuration tests are: 82 unit tests in `crates/sdk/src`,
15 in `in_memory_cycle.rs`, 5 in `no_external_touch.rs`, 5 in `public_api.rs`,
1 doctest. With `--all-features` the desktop's v2 suites add 55 more.

### 11.2 Gates, all green

* `cargo fmt --all --check`
* `cargo clippy --workspace --all-targets -- -D warnings`
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`
* `cargo test --workspace` and `--all-features`
* `cargo test -p vilsend-sdk --doc`
* `cargo build --workspace` — the desktop app builds
* `npm run typecheck`, `npm run build`, `npm run check:ipc`
  (28 invoke sites, 33 registered commands — unchanged)
* the three architecture lints, negated form:

  ```sh
  ! cargo tree -p vilsend-core --prefix none | grep -E '^(tauri|reqwest|axum|sqlx|keyring)( |$)'
  ! cargo tree -p vilsend      --prefix none | grep -E '^vilsend-engine( |$)'
  ! cargo tree -p vilsend-sdk  --prefix none | grep -E '^(tauri|reqwest|axum|sqlx|keyring|tokio)( |$)'
  ```

  All pass. `cargo tree -p vilsend-sdk --prefix none` is `vilsend-core`,
  `futures-channel`, `futures-core`, `serde`, `sha2` and their transitive
  dependencies — nothing that reaches a shell, a socket or a database.

* `cargo-semver-checks`, both ways (D1): **196 checks: 196 pass, 58 skip**.
* `cargo public-api --manifest-path crates/sdk/Cargo.toml -ss`, regenerated
  twice with no diff, and diffed against the committed snapshot: **no diff**.

### 11.3 Two process failures, recorded rather than smoothed over

1. **`crates/sdk/tests/in_memory_cycle.rs` was written with 5.1 and never
   committed.** It compiled and passed as an untracked file, so every gate was
   green while the phase's primary acceptance test was not in the repository.
   The per-commit `cargo test --workspace` cannot catch this — it tests the
   working tree, not the commit. Fixed in `4c3059e`.

2. **The 5.2 commit was not `cargo fmt`-clean.** The file was formatted by a
   later `cargo fmt --all`, and the formatting fix was not staged with it. The
   branch was rebased with `--exec 'cargo fmt --all; git add -u; git commit
   --amend'` and **every commit is now verified fmt-clean** by checking out each
   one into its own worktree and running `cargo fmt --all --check` there. All
   seven pass. The hashes in this report are the post-rebase ones.

Both are the same shape: a gate that runs against the working tree cannot tell
you what is in the commit. Worth a per-commit gate of its own if this phase's
process is repeated.

---

## 12. Files

### Added

| File | Lines | What |
|---|---|---|
| `crates/sdk/src/backend/memory.rs` | 922 | The in-memory engine: the link, the assembly, chunking, publishing |
| `crates/sdk/src/state.rs` | 668 | The shared state behind a handle; the wait/wake and pause signals |
| `crates/sdk/tests/in_memory_cycle.rs` | 541 | The acceptance test — 15 tests over the public API |
| `crates/sdk/src/lib.rs` | 356 | `Vilsend`, `VilsendBuilder`, the re-exports, `LOOPBACK_PEER` |
| `crates/sdk/tests/no_external_touch.rs` | 344 | The `in_memory()` contract, checked statically and behaviourally |
| `crates/sdk/src/request.rs` | 340 | `SendRequest`, `ReceiveRequest`, `Policy` and its precedence |
| `crates/sdk/public-api.txt` | 333 | The generated snapshot |
| `crates/sdk/src/handle.rs` | 315 | `TransferHandle` and the outcome types |
| `crates/sdk/src/events.rs` | 313 | `EventStream` and the bounded fan-out |
| `crates/sdk/tests/public_api.rs` | 272 | The three public-surface invariants |
| `crates/sdk/src/progress.rs` | 190 | `Progress`, `TransportKind`, `DegradedReason` |
| `crates/sdk/src/files.rs` | 190 | `MemoryFiles` |
| `crates/sdk/src/ids.rs` | 165 | `FileRef`, `Destination` |
| `crates/sdk/src/clock.rs` | 161 | `Clock`, `FakeClock` |
| `crates/sdk/README.md` | 135 | Crate docs, the compiled example, the semver policy |
| `docs/migration/smoke-checklist.md` | 124 | The manual steps, per phase |
| `docs/migration/adr/0013-sdk-facade-crate.md` | 96 | The crate and its dependency edges |
| `crates/sdk/src/auth.rs` | 89 | `AuthFacade`, `AuthState` |
| `crates/sdk/examples/send.rs` | 57 | 22 lines of code, past the docs |
| `crates/sdk/src/backend/mod.rs` | 51 | The `Backend` seam |
| `crates/sdk/Cargo.toml` | 35 | Five dependencies, each justified in ADR-0013 |

### Changed

| File | What |
|---|---|
| `.github/workflows/ci.yml` | The `public-api` job; the engine lint; the "guards the guard" step |
| `crates/core/src/status.rs` | `#[non_exhaustive]` on `TransferStatus` (10 lines) |
| `crates/desktop/Cargo.toml` | `publish = false` (4 lines) |
| `Cargo.toml` | `crates/sdk` added to the workspace members |

### Not touched

`crates/desktop/src/**` — not one line. `git diff f857e60 HEAD -- crates/desktop/src/`
is empty, which is why the golden payloads and the IPC contract check are
unchanged rather than merely passing. The frontend is untouched. The v1 wire
format is untouched.
