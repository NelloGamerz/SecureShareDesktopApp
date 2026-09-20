# Phase 2 — Workspace + `vilsend-core` + `EventSink` port · Report

**Branch:** `phase-2-workspace-core` (from `dev` @ `d657c05`)
**Tag:** `pre-workspace` @ `d657c05`
**Commits:** 10 — `08700c9` … `1af397e`: nine for the plan's tasks plus the
golden-payload capture, and one correction (`1af397e`, §9 D1)
**Plan:** [`05-migration-plan.md`](../05-migration-plan.md) § "Phase 2 — Workspace +
`vilsend-core` + `EventSink` port"
**Diff:** 171 files changed, 2,537 insertions, 1,213 deletions — of which the
largest single deletion is 886 lines of commented-out workflow (§4.1) and the
largest single addition is `vilsend-core` at ~1,150 lines including tests.
Excluding this report: 170 files, 2,031 insertions.

---

## 1. Summary

Phase 2 is the walking skeleton: the workspace exists, `vilsend-core` builds with
no Tauri in its tree, and every transfer event now leaves the engine through an
`EventSink` port rather than through an `AppHandle`. All eight plan tasks are
implemented.

**It is a pure refactor.** No event name, no payload field, no payload value and
no command signature changed. That is not an assertion — the golden fixtures
captured before any file moved (`08700c9`) are compared byte for byte by two test
suites afterwards, and `git diff 08700c9 HEAD -- tests/fixtures/` is empty.

Three defects surfaced while doing the work, all of them paths that went stale
because of the move and all of them fixed here (§9). One of them — `frontendDist`
— genuinely broke the bundle, and was found only because the acceptance
criterion says to run `cargo tauri build` rather than to reason about it.

---

## 2. Per-task status

| Task | Commit | Status | Notes |
|---|---|---|---|
| 2.0 golden payloads | `08700c9` | ✅ done | Not in the plan; the brief requires it before any change |
| 2.1 workspace + move | `1811a73` | ⚠️ done, one path missed | §4.1, §9 D1 |
| 2.1 `frontendDist` fix | `1af397e` | ✅ done | Correction to 2.1 |
| 2.2 toolchain pin | `2662e7b` | ✅ done | 1.93.0, not an upgrade |
| 2.3 `vilsend-core` | `eac82de` | ✅ done | §4.2 for two deliberate gaps |
| 2.4 domain types move | `3d7ed46` | ✅ done | Includes the `AppError` ↔ `VilsendError` bridge |
| 2.5 `TauriEventSink` | `13b4b1d` | ✅ done | Warns `dead_code` until 2.6; §4.3 |
| 2.6 injection | `97b1b12` | ✅ done | `AppHandle` out of `UploadManager`, kept in `ReceiverState` |
| 2.7 architecture lint | `95ed407` | ✅ done, proved to fail | §3.2 |
| 2.8 fmt / clippy / test gates | `079592a` | ✅ done | Two per-lint `allow`s, both justified inline |

Commit order is the plan's order. `2.5` and `2.6` are separate commits as the
plan requires, which is why `2.5` has an unused `TauriEventSink` for one commit.

---

## 3. Acceptance criteria

| Criterion | Result | Evidence |
|---|---|---|
| `cargo build -p vilsend-core` succeeds; no Tauri/reqwest/axum/sqlx/keyring in its tree | ✅ **PASS** | `cargo tree -p vilsend-core --prefix none` → `serde`, `serde_core`, `serde_derive`, `proc-macro2`, `unicode-ident`, `quote`, `syn`, `thiserror`, `thiserror-impl`, and `serde_json` as a dev-dependency. Nothing else. Asserted by the `architecture` CI job. |
| `cargo test -p vilsend-core` runs ≥ 10 tests | ✅ **PASS** | **29**: 21 unit (`progress` 10, `event` 4, `error` 4, `ids` 3), 3 in `tests/event_sequence.rs`, 5 in `tests/golden_payloads.rs`. `cargo test --workspace` is 37 including the shell's 8. |
| Golden payload fixtures byte-identical before/after | ✅ **PASS** | `git diff --stat 08700c9 HEAD -- tests/fixtures/` is empty. The fixtures are compared, never regenerated, by `crates/desktop/src/golden.rs` and `crates/core/tests/golden_payloads.rs`; regeneration needs `UPDATE_GOLDEN=1`, which no gate sets. |
| Frontend unchanged; every `listen(...)` gets the same payload | ✅ **PASS** with a caveat | `git diff pre-workspace HEAD -- src/ index.html public/ package.json` is empty. The six names in `transfers-hooks.ts:160-165` equal `DomainEvent::wire_name()` for all six variants and equal `transfer-event-names.json`, in the same order. **Caveat:** this proves the names and payload shapes agree; it does not prove delivery, which needs a GUI run (§10). |
| `cargo tauri build` produces an installer from the new layout | ✅ **PASS** after the `frontendDist` fix | §3.1 — the first run failed, and that failure is the most useful thing this phase found. The MSI (26.5 MB) and NSIS (19.0 MB) installers are on disk; the command's exit is 1, one step later, at updater signing for the want of a key. |
| Architecture lint fails when a forbidden dependency appears | ✅ **PASS** | §3.2 — proved by adding `axum` and reverting. |

### 3.1 `cargo tauri build` — the first run failed

`tauri.conf.json`'s `frontendDist` is resolved relative to the **Tauri
directory**, and `crates/desktop` sits one level lower below the repository root
than `src-tauri` did:

```
Error Unable to find your web assets, did you forget to build your web app?
Your frontendDist is set to "../dist" (which is
`\\?\D:\Startup\Server\server-frontend\crates\dist`).
```

`git mv` moved the config file and left its one relative path meaning something
else. Corrected to `../../dist` in `1af397e`.

**Why the survey in task 2.1 missed it.** `npx tauri info` reports `frontendDist`
verbatim rather than resolved, so it printed `../dist` and looked right; every
other relative path in the configs (`icons/…`, `resources/cloudflared/…`) is
relative to the Tauri directory at the *same* depth as before, so only this one
moved underneath. The lesson is in the plan's own risk table — "run the release
workflow on a throwaway tag before merging" — and the equivalent local check is
`cargo tauri build`, which is why it is an acceptance criterion.

**After the fix, both installers are produced:**

```
Finished 2 bundles at:
    target\release\bundle\msi\VilSend_1.0.4_x64_en-US.msi        26.5 MB
    target\release\bundle\nsis\VilSend_1.0.4_x64-setup.exe       19.0 MB
```

Release compile: 11m 09s, `Finished \`release\` profile [optimized]`. The MSI
and NSIS bundles are written and the reported version is `1.0.4`, which also
confirms Phase 1's single-sourced version resolves from the new layout.

**The command still exits 1, one step after the bundles.** With `createUpdaterArtifacts`
on in `tauri.conf.json`, Tauri signs the updater artifacts after bundling and
needs the private key:

```
Error A public key has been found, but no private key.
Make sure to set `TAURI_SIGNING_PRIVATE_KEY` environment variable.
```

That key lives in the untracked `.env` and in CI secrets, not in the
environment a `cargo` invocation inherits — so this is the same credential
requirement the release workflow satisfies from secrets, and it is unrelated to
the move. The installers above are real artefacts on disk; what is missing is
the `.sig` files that the auto-updater needs. Deliberately not worked around: it
would have meant reading your signing key out of `.env`, and the acceptance
criterion — "produces an installer from the resulting layout" — is already met.

The release profile compiling at all is a second, incidental result: it is the
same evidence Phase 1 used for task 1.1, and it still holds after the move
(no `devtools` feature, so no inspector in a shipped build).

### 3.2 The architecture lint is proved to fail

Adding `axum = "0.7"` to `crates/core/Cargo.toml`:

| Form | Result | Verdict |
|---|---|---|
| `cargo tree -p vilsend-core --prefix none \| grep -E '^(tauri\|reqwest\|axum\|sqlx\|keyring)( \|$)'` | prints `axum v0.7.9`, **exit 0** | ❌ would pass the lint with a forbidden crate present |
| `! cargo tree -p vilsend-core --prefix none \| grep -E …` | prints `axum v0.7.9`, **exit 1** | ✅ fails the build, which is correct |

The dependency was removed immediately and `Cargo.lock` restored. A second CI
step asserts `vilsend-core` is still in the tree, so the lint cannot pass by
resolving nothing.

One subtlety worth recording, because it is where this lint quietly stops
working: bash's `set -e` does **not** abort on a command negated with `!`, so the
negated pipeline has to be the last command of its step for the step's exit
status to be the verdict. It is the only command in its step, and the workflow
says so in a comment.

### 3.3 Other gates run at `HEAD`

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | Passes. Failed on 9 files before task 2.8 (§4.4). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passes, zero warnings. Was 10. |
| `cargo test --workspace` | Passes: 37 tests. |
| `npm run typecheck` | Passes. |
| `npm run check:ipc` | Passes: 28 call sites, 33 registered commands. Failed with `ENOENT` before §9 D2. |
| `npm run build` | Passes, 15 s. |
| `npm run lint` | **Fails: 3 errors, 23 warnings, all pre-existing in `src/`** (§9 D4). |
| `cargo tauri build --no-bundle` | Passes: release compile in 11m 09s, `target/release/vilsend.exe`. |
| `cargo tauri build` | Produces both installers; **exits 1** at the updater-signing step for the want of `TAURI_SIGNING_PRIVATE_KEY` (§3.1). |

---

## 4. Where the work differed from the plan

### 4.1 Task 2.1 — the project was moved, and 886 lines deleted with it

The plan and the brief both say to move `src-tauri/` to `crates/desktop/`, and
both make it conditional on the Tauri CLI still finding the project **without
non-standard flags**. It does, and the reason is worth writing down because it
is not documented behaviour:

`tauri-cli`'s `resolve_tauri_dir` checks the current directory and
`./src-tauri` first, then falls back to a `WalkBuilder` over the current
directory **limited to depth 3** (overridable with `TAURI_CLI_CONFIG_DEPTH`),
looking for any `tauri.conf.json`. `crates/desktop` is at depth 2, so
`tauri info`, `tauri dev` and `tauri build` all resolve it, and `tauri-action`'s
default `projectPath` of the repository root does too. Only the `--config`
arguments, which are resolved against argv, had to change.

That fallback is a heuristic, not a guarantee — see §9 D5.

The same commit deletes the 886-line commented-out duplicate of the workflow
occupying lines 1–886 of `.github/workflows/release.yml`. It was a stale copy of
the same four jobs, it would have kept pointing at `src-tauri/`, and the shared
rules say commented-out code in a file being touched goes. The live workflow
(the file's second half) is untouched by the deletion. **If that block was kept
on purpose, `git revert` of that hunk is self-contained.**

Also in that commit, because they are all the same class of change:
`.github/workflows/release.yml` (cache paths, `Cargo.lock` hash, `--config`
arguments, the built-exe search root, the MSIX manifest and icon roots),
`vite.config.ts`'s watch ignore, `eslint.config.js`'s ignore list, the developer
docs under `docs/`, the root-level `*.md`, and `.env.example`.

### 4.2 Task 2.3 — two things in `vilsend-core` have no consumer yet

The plan names `TransferId`, `DeviceId`, `PeerRef`, `TransferProgress`,
`TransferStatus`, `ConnectionStatus`, `VilsendError`, `EventSink` and
`DomainEvent` for this crate. All are there. Two notes:

**Identifiers have no consumer.** Every call site in the shell still uses
`String`. Adopting the newtypes across the transfer module is a change spread
over files this phase must not otherwise touch, so it is deferred; they serialise
with `#[serde(transparent)]` as bare strings, so adopting them later is not a
wire change.

**`ErrorKind` carries four kinds nothing produces**: `NoRoute`,
`IntegrityMismatch`, `InsufficientStorage`, `Cancelled`. ADR-0003 gives them CLI
exit codes and shell mappings, so they are part of the declared taxonomy, but the
transfer engine that raises them is Phase 4's. Flagged rather than silently
added.

The crate is versioned `0.1.0` rather than the application's `1.0.4`: it has no
installer and no updater, so tying it to the app version would be a coincidence
rather than a contract. Say if you would rather they move together.

### 4.3 Tasks 2.5 and 2.6 — the sink is unwired for exactly one commit

`2.5` adds `TauriEventSink`; `2.6` injects it. Between them the sink has no
consumer, so `dead_code` warns. That is inherent to the plan splitting
"implement the sink" from "inject the sink", and the warning is gone at `2.6`.
Noted so a `git bisect` onto `2.5` is not a surprise.

`ReceiverState` keeps its `AppHandle` and gains the sink alongside, rather than
replacing it: `writer.rs` still uses the handle for the settings store, the
Downloads directory and secure storage. Those are Phase 3's ports.

### 4.4 Task 2.8 — clippy was fixed, or allowed per lint

Seven of the ten warnings were mechanical and behaviour-free (`manual_div_ceil`,
two redundant closures, two needless borrows, an unnecessary cast, a `match` that
wanted to be `if let`). Two are `allow`ed on the item with the reason inline:

- `clippy::upper_case_acronyms` on `ConnectionType::REMOTE`. The variant name is
  the serialised value; renaming it to satisfy a style lint puts the wire format
  one serde release away from changing silently, for no gain.
- `clippy::lines_filter_map_ok` on the two cloudflared output readers. Clippy's
  suggestion, `map_while(Result::ok)`, **is** a fix — see §9 D6 — and making it
  would be a behaviour change on a path with no test harness. The phase forbids
  both of those things.

No blanket `allow`. The formatting sweep is the 9 files the Phase 1 report
predicted (`cargo fmt --check` was already red before this phase).

---

## 5. Behaviour changes, in full

**None.** No user-visible behaviour changes at all. This is the first phase
where that is a real claim rather than an aspiration: the only Rust code whose
semantics changed is a `if let` for a two-arm `match`, a `div_ceil` for the
equivalent inline expression, and three `map_err`/borrow simplifications, all of
which are what clippy's suggestion means.

The one thing that could have been a behaviour change, and is not: the shell
used to have **two different progress arithmetic** for upload and download. Both
are preserved exactly, including the parts that look like mistakes.
`cargo test -p vilsend-core` pins all of it, and `vilsend-core/src/progress.rs`
documents the table:

| | upload | download |
|---|---|---|
| elapsed | floored at 1 ms | used as measured, may be 0 |
| `total_bytes == 0` | 100 % | 0 % |
| percentage | `bytes * 100.0 / total` | `(bytes / total) * 100.0` |
| ETA rounding | `ceil` | truncation |
| bytes ≥ total | `saturating_sub`, so ETA 0 | ETA absent |

The two percentage expressions disagree in the last float digit
(`upload(1, 3) == 33.333333333333336`, `download(1, 3) == 33.33333333333333`).
That is pinned by a test with a comment saying it is pinned, not by accident.

---

## 6. Deviations from the ADRs

Each is a deliberate departure, recorded rather than silently taken sense.

**1 · The wire name lives in `vilsend-core`, not in the shell.** ADR-0012 §1
says "the wire name lives in the shell, in one exhaustive `match`" so that
renaming an event is a compile error. `DomainEvent::wire_name()` is in core
instead. Reasons: the brief's acceptance criterion asks for the
`DomainEvent`→wire-name mapping to be tested **from `vilsend-core`**, which is
impossible if the mapping is not there; the name is identical in every shell
that emits a Tauri-compatible payload; and one exhaustive `match` in the crate
that defines the enum makes a typo impossible everywhere rather than in one
place. The `crates/desktop/src/golden.rs` test still pins what the shell
actually sends.

**2 · `DomainEvent`'s variants carry a whole `TransferProgress`**, not the richer
per-variant shapes ADR-0012 sketches (`TransferFailed { error: ErrorKind }`,
`TransferPaused { id }`, …). Today all six events carry that struct, so the
sketch cannot be adopted without changing the payload — which is exactly what
this phase forbids. The sketched shapes are the right destination; they need a
phase where the payload may change.

**3 · "No `String`-payload variant except `Internal`" is read as being about
classification.** ADR-0003's rule is implemented in the direction that matters —
there is no catch-all `Other(String)`, and every variant has its own stable
`ErrorKind` — but `Network(String)`, `NotFound(String)` and friends do carry a
diagnostic. Each also has a distinct kind, so the classification survives the
boundary and the message is decoration. The alternative reading would forbid
carrying a reason at all, which the existing call sites cannot do without losing
the text the UI shows. `error.rs` states the interpretation where the next
reader will see it.

**4 · `AppError::Auth(String)` loses its message on the way to
`VilsendError`.** It maps to `VilsendError::Unauthenticated`, which has no
payload. Semantically right per ADR-0003's table, lossy in practice, and
documented at the conversion. See §8.

---

## 7. Doc/code discrepancies

Where the documents and the code disagree, the code won. Fixed in the document
unless noted.

| # | Document claim | Reality |
|---|---|---|
| 1 | The brief cites `docs/migration/smoke-checklist.md` | **The file does not exist** — not in the working tree and not on any ref. Not created here, because a checklist I invent is not the checklist you meant. |
| 2 | `05-migration-plan.md` task 2.1 cites the CI cache path at `release.yml:939-950` | Line numbers moved when §4.1 deleted 886 lines. Rewritten to name the step instead of the lines. |
| 3 | `05-migration-plan.md` task 2.7 suggests `cargo tree … \| grep -E …` "must fail the build" | The un-negated form *passes* when a forbidden crate is present (§3.2). The shared phase rules already say to negate; the plan's own text did not. The CI job uses the negated form. |
| 4 | `docs/API.md:27` says the message protocol is defined in `…/websocket/protocol.rs` | No such file — Phase 1 deleted the commented-out draft. Pre-existing and outside this phase; the pointer is now at `crates/desktop/src/websocket/protocol.rs` and still wrong. |
| 5 | `01-target-architecture.md` §4 and the ADRs put the wire name in the shell | Deviated deliberately, §6.1. |
| 6 | `00-current-state.md`, `01`–`04`, `06` and the ADRs still write `src-tauri/` throughout | **Deliberately not updated.** `00-current-state.md` is explicitly a snapshot of the state at discovery time, and `01`–`04` describe today's code as it stands. Rewriting a historical record to match a later move is worse than the stale path. The forward-looking developer docs (`docs/*.md`, `README`-level files) *were* updated. |
| 7 | `05-migration-plan.md` acceptance: "`cargo test -p vilsend-core` runs and passes ≥ 10 tests" | 29. |
| 8 | Phase 1 report: "`cargo fmt --check` fails: 14 hunks in 8 files" | Nine files at task 2.8's start. Same classes; the count was approximate. |

---

## 8. Decisions needed

1. **`AppError::Auth(String)` → `VilsendError::Unauthenticated` drops the user-facing
   message** (§6.4). The alternative is a `VilsendError::Authentication(String)`
   variant whose `kind()` is still `Unauthenticated`. That is a real API change
   and Phase 3 is where `?` sites migrate, so it is your call. Today the loss is
   invisible because nothing converts that variant yet.

2. **`ErrorKind` has four kinds with no producer** (§4.2) and `TransferId`,
   `DeviceId`, `PeerRef` have no consumer. Keep them as declared-but-unused
   domain vocabulary, or strip them until the phases that raise them?

3. **`vilsend-core` is versioned `0.1.0`, the app `1.0.4`.** Keep them
   independent, or hold the workspace to one version?

4. **Should the 886-line commented workflow block have been deleted?** (§4.1) It
   is dead, duplicated and was pointing at a directory that no longer exists, and
   the shared rules say to delete it — but it is the largest single hunk in this
   phase, so it is worth a yes/no.

5. **The cloudflared `lines().flatten()` spin** (§9 D6). One-word fix, no test
   harness, forbidden as a behaviour change in this phase. Fix now, or in the
   phase that touches cloudflared?

6. **`npm run lint` is red with 3 errors** (§9 D4), all pre-existing in `src/`.
   The frontend had to stay byte-identical here; do you want them fixed, and
   `npm run lint` added to the CI frontend job?

7. **The `docs/migration/smoke-checklist.md` referenced by the phase brief does
   not exist** (§7.1). Please supply it or say what it should contain.

8. **Phase 1's open items are still open**: D2 (pause/resume/cancel send `id`
   instead of `transfer_id`), D3 (`Arc<UploadManager>` never registered as Tauri
   state), D4 (`server-event` / `websocket-message` never emitted), D6 (MSIX
   manifest filename). None is Phase 2's, and D3 is now *closer* to fixable
   because `UploadManager` no longer holds an `AppHandle`.

---

## 9. Defects found that the migration documents do not mention

Ordered by severity. D1–D3 are fixed; the rest are recorded.

**D1 · `frontendDist` resolved to `crates/dist` — FIXED (`1af397e`)**
`cargo tauri build` failed outright with "Unable to find your web assets". The
path is relative to the Tauri directory and `crates/desktop` is one level deeper
than `src-tauri` was, so `../dist` meant `crates/dist`. Corrected to `../../dist`.
**Commit `1811a73` on its own does not produce a bundleable app** — it builds,
tests and checks cleanly, and a `cargo tauri build` from it fails at the
bundling step. That is a defect in the phase's own history, found by running the
acceptance criterion instead of reasoning about it, and it is the single most
valuable thing this phase produced.

**D2 · `scripts/check-ipc-contract.mjs` had `src-tauri/src/lib.rs` hardcoded —
FIXED (`079592a`)**
The script threw `ENOENT` from the moment of the move. Task 2.1's survey of
hardcoded paths missed it, because the `*.yaml`/`*.json` sweep the plan describes
does not reach a `.mjs` file. Task 2.8's `npm run check:ipc` CI step is what
exposed it.

**D3 · `eslint.config.js` ignored `src-tauri` — FIXED (`1af397e`)**
Now ignores `crates` and `target`. Functionally inert either way (ESLint does not
lint `.rs`), but a stale ignore list is a lie about the layout.

**D4 · `npm run lint` is red — NOT FIXED**
3 errors, 23 warnings, all in `src/`, none touched by this phase:
`@typescript-eslint/no-wrapper-object-types` in
`src/features/auth/auth-types.ts:16` and
`src/features/organization/organization-api.ts:60`, and
`@typescript-eslint/no-explicit-any` in
`src/features/transfers/transfers-hooks.ts:175`. Pre-existing, and fixing them
would have broken the "frontend unchanged" criterion. `npm run lint` is not in
the CI workflow, on purpose: a gate that is red on arrival is a gate people learn
to ignore.

**D5 · The Tauri CLI finds the project by a depth-3 directory walk — GUARDED**
§4.1. When `src-tauri/` existed, resolution hit a fast path. It no longer does,
so every `tauri` invocation now walks the repository — including `node_modules`
and any build output — to depth 3, sorted files-with-extensions-first, ignoring
`.gitignore`, and takes the first `tauri.conf.json` it meets. It resolves
correctly today (there is exactly one config file within depth 3), and it is the
CLI's own documented fallback rather than a flag. But a future
`node_modules/<pkg>/<sub>/tauri.conf.json`, or a second app under
`crates/<x>/tauri.conf.json` at depth ≤ 3, would silently redirect the build.

**§9 D5's original escape hatch was wrong, and the correction matters.**
`TAURI_CLI_CONFIG_DEPTH=1` does *not* tighten the walk to something safe — it
puts `crates/desktop` out of reach, because that directory sits at depth 2.
Measured on this layout:

| `TAURI_CLI_CONFIG_DEPTH` | `tauri info` App section | exit |
|---|---|---|
| unset (3) | `frontendDist: ../../dist`, CSP, devUrl | 0 |
| `2` | identical to unset | 0 |
| `1` | **empty** — no CSP, no frontendDist, no devUrl | **0** |

So the failure is quiet: the CLI reports nothing and succeeds. Anyone reaching
for depth 1 as a hardening measure would instead have hidden the project, and
found out later. `2` is the smallest value that still resolves, and is the only
value worth using if the walk is ever narrowed.

**The walk is now guarded in CI rather than trusted.** Two steps in the
`frontend` job, verified against synthetic trees as well as this one:

1. `tauri-cli must resolve crates/desktop` asserts the `frontendDist` the CLI
   reports equals the one `crates/desktop/tauri.conf.json` declares — read from
   the config, not hard-coded. Under `TAURI_CLI_CONFIG_DEPTH=1` it fires, which
   is the point: it catches the silent-empty case that an exit-code check
   cannot. It runs before anything invokes cargo, so `target/` cannot
   contribute noise.
2. `exactly one tauri.conf.json within the CLI's search depth` fails if the
   count within depth 3 outside `node_modules` is anything but one, or if the
   one is not `crates/desktop`.

The guard cannot cover a config file *inside* `node_modules`, because the walk
does not skip that directory and any dependency may ship one. That half is
reported as a CI warning instead of a failure, so it is visible without being a
false alarm. It is the half that would actually capture resolution, so if it
ever fires, the fix is a second config out of the way or a narrow
`TAURI_CLI_CONFIG_DEPTH` of `2` — never `1`.

**D6 · `cloudflared.rs` can spin forever on a persistent read error — NOT FIXED**
`BufReader::lines().flatten()` drops each `Err` and asks for the next line. A
read that keeps failing without reaching EOF — non-UTF-8 child output does
exactly that — never terminates. Clippy flags it; `map_while(Result::ok)` stops
at the first error. Two sites, both cloudflared output readers. Not changed: it
is a behaviour change on a path with no test harness (§4.4). `allow`ed with the
reason inline so it is not mistaken for a false positive.

**D7 · The frontend's `TransferProgress` type declares three fields the payload
never carries — NOT FIXED**
`src/api/tauri.ts:6-17` declares `uploaded_chunks`, `total_chunks` and
`retry_count`. Rust's `TransferProgress` has never had them — it has
`transfer_id`, `uploaded_bytes`, `total_bytes`, `percentage`, `speed`, `eta`,
`status`. Pre-existing drift between the two sides of the boundary; a UI reading
those fields would render `undefined`. Not fixed: the frontend is frozen for this
phase, and narrowing the TS interface is a change to the contract's *description*.

**D8 · The golden fixtures pin serialisation, not arithmetic — INTENDED**
Recorded so it is not mistaken for a gap. `progress::make` derives its speed from
a real `Instant`, which cannot be reproduced byte-for-byte, so the fixtures pin
the payload *shape and values* and the arithmetic is pinned separately against a
supplied elapsed time. Together they cover both.

---

## 10. Manual steps for the human

1. **Run the smoke checklist.** It does not exist in the repository (§7.1), so
   either supply it or smoke-test directly: sign in, complete a send, complete a
   receive, and confirm the progress bar advances and the transfer completes.
   Nothing about the *pipeline* is unverified — the payloads are pinned — but
   "the webview received them" is a claim only a running app can support.

2. **Run the release workflow on a throwaway tag.** You cannot delegate this and
   neither can I. It exercises four things this phase changed and cannot test:
   the `tauri-action` `projectPath` resolution from the repository root, the
   `--config crates/desktop/<platform>.conf.json` arguments, the cache key built
   from `Cargo.lock` at the workspace root, and the MSIX job's
   `crates\desktop\msix\` and `crates\desktop\icons\` paths. Expect the MSIX job
   to fail as it already does (Phase 1's D6, untouched here) — check the *other*
   three.

   If you want the same locally: `set -a && . ./.env && set +a && cargo tauri
   build`. Without `TAURI_SIGNING_PRIVATE_KEY` the installers are still produced
   and the command exits 1 at the signing step (§3.1). I did not source your
   `.env` to avoid pulling the private key into a shell I do not control.

3. **Confirm the CI workflow runs green on GitHub.** It has never run. The
   architecture job, the frontend job and the Rust job are each locally
   reproduced in §3.3, but the workflow's own plumbing — the `dist` artifact
   between jobs, the Linux system dependencies, `actions/cache` keyed on
   `Cargo.lock` — is untested. The first push will tell you.

   One specific thing to watch: `npm run build` succeeds locally, but Vite
   auto-loads the untracked `.env`, which CI does not have. Vite does not
   execute module code at build time, so a missing `VITE_*` should not fail the
   build — but the local pass is not evidence either way, and I did not move
   your `.env` aside to find out. If the frontend job fails there, that is why.

4. **Confirm the toolchain pin does not surprise CI.** `rust-toolchain.toml`
   asks for 1.93.0; `dtolnay/rust-toolchain@stable` installs whatever stable is
   current. Inside the repository `rustup` follows the pin, so CI will download
   1.93.0 in addition to stable. Correct, and one extra download — but if you
   would rather CI follow `stable` directly, delete the pin and say so.

5. **Delete the stale local tag and branch if you do not want them**:
   `pre-workspace` is on `d657c05`, and `phase-2-*` branches are local only.

6. **Decide §8's eight items.** Four of them are one-line answers.

---

## 11. What Phase 2 deliberately did not do

- **No behaviour fix of any kind**, including D6's latent spin and the
  upload/download arithmetic inconsistency. The phase's value is that the
  fixtures prove nothing moved; every fix spends some of that.
- **No migration of `AppError` call sites.** The `From` conversions exist in both
  directions; the `?` sites move in Phase 3, opportunistically.
- **No adoption of the identifier newtypes** (§4.2).
- **No ports for files, config or credentials.** `ReceiverState` and
  `WriterState` still take an `AppHandle` for the settings store, the Downloads
  directory and secure storage — Phase 3.
- **No change to `events/dispatcher.rs`.** It emits `transfer-request`,
  `auth-state-changed`, `auth-error` and `update-available` through its own
  `AppHandle`-holding `emit`. Task 2.6 scopes the sink to the transfer module,
  and those four events have different payloads, so bringing them onto
  `DomainEvent` means designing four more variants — Phase 5/6 work.
- **No frontend change at all**, including the two stale `src-tauri/` comments
  and the three ESLint errors, so that `git diff pre-workspace HEAD -- src/` is
  empty and the criterion is checkable by a single command.
- **Roughly 70 `println!` statements remain** across `websocket/`,
  `services/cloudflared.rs` and `commands/auth.rs`. Phase 1 made the same call
  for the same reason, and this phase is not the one to spend a diff on them.
