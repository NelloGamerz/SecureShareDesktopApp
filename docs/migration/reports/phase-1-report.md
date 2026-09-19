# Phase 1 — Containment · Report

**Branch:** `phase-1-containment` (from `dev` @ `b80683a`, which equals `main`)
**Commits:** 15 — `84986a4` … `1a04f99`
**Plan:** [`05-migration-plan.md`](../05-migration-plan.md) § "Phase 1 — Containment"
**Diff:** 59 files changed, 455 insertions, 2,488 deletions; Rust files under
`src-tauri/src/` go from **80 to 61**.

---

## 1. Summary

Fourteen plan tasks are implemented. `cargo check --all-targets` went from
**21 warnings to 0**, which is the phase's headline acceptance criterion. Nine
defects that the migration documents do not mention were found; two are fixed
because the phase's own deliverables could not otherwise be green, and the
other seven are recorded below.

Nothing in this phase changes the wire format, a command payload, or an event
payload. Two changes are user-visible and are called out explicitly in §4:
the CSP is now enforcing (§3, task 1.3) and `stop_websocket` exists (§3, task
1.6).

---

## 2. Per-task status

| Task | Commit | Status | Notes |
|---|---|---|---|
| 1.1 gate `open_devtools` | `84986a4` | ✅ done, widened | See §3.1 |
| 1.2 signing key | `b09a69b` | ⚠️ premise false | See §3.2 and §5 |
| 1.3 real CSP | `9edcbb3` | ✅ done | See §3.3 |
| 1.4 log level | `786e8d0` | ✅ done | Trace (dev) / Info (release) |
| 1.5 token logging | `4c2052f` | ✅ done, relocated | See §3.4 |
| 1.6 register `stop_websocket` | `e3562b2` | ✅ done | Plus the IPC contract check |
| 1.7 delete commented-out code | `efe213a` | ✅ done | 1,417 lines |
| 1.7 eliminate warnings | `1a04f99` | ✅ done | 21 → 0 |
| 1.8 delete case-duplicate orphans | `345ac8d` | ✅ done | Four orphans, not three |
| 1.9 delete empty stubs | `93cbeca` | ✅ done | Eleven files, not ten |
| 1.10 remove dead dependencies | `efaccd1` | ✅ done | Plus one build fix |
| 1.11 remove `src-tauri/2` | `141511e` | ✅ done | |
| 1.12 `ConnectionStatus` casing | `ce090c3` | ✅ done | Fix is in the frontend |
| 1.13 macOS arm64 cloudflared | `c9c90b4` | ✅ done | Not runnable on this host |
| 1.14 single-source the version | `72fa989` | ✅ done | See §6.4 |

---

## 3. Tasks that differed from the plan

### 3.1 Task 1.1 — the devtools gate was widened

The plan says to gate `window.open_devtools()` behind
`#[cfg(all(desktop, debug_assertions))]`. That is done, but it is not
sufficient on its own: `tauri.conf.json` set `"devtools": true` on the main
window and `Cargo.toml` enabled Tauri's `devtools` feature, and per the
`tauri-utils` source the window flag *"works in debug builds, but requires
`devtools` feature flag to enable it in release builds"*. Together they meant a
release build still shipped a reachable inspector. All three are now closed:
the call is gated, the window flag is removed, and the Cargo feature is off.
Debug builds keep devtools, because the feature is only required for release.

### 3.2 Task 1.2 — the stated premise is not true

The task and finding **H1** both say `TAURI_SIGNING_PRIVATE_KEY` is *"present
in the committed root `.env`"*. It is not:

```
$ git log --all --oneline -- .env          # empty
$ git check-ignore -v .env
.gitignore:14:.env  .env
```

`.env` is untracked and has never existed on any ref, so nothing was leaked
into the repository and no rotation is forced by this repository.

The real defect next door is that `.gitignore` *also* ignored `.env.example`,
so the documented template shipped nowhere and a new contributor could not
obtain it. That rule is removed, `.env.example` is now tracked, and the
signing-key requirement is written into it as prose with no value.

**The local untracked `.env` was deliberately not edited.** It holds the only
known copy of the updater signing key, and the phase's own rollback note warns
that losing or rotating this key breaks auto-update for every installed client.
Deleting a secret from its only known location is not reversible by `git
revert`, so it is left for you — see §6.

### 3.3 Task 1.3 — the CSP is enforcing, not report-only

The plan's risk table proposes shipping the CSP *"in report-only mode first"*.
Tauri has no report-only mode, so that mitigation is not available; the policy
is enforcing from the first run. The policy was derived from the origins the
code actually contacts:

- `script-src 'self' https://checkout.razorpay.com` — Razorpay's checkout is
  the only external script (`src/services/razorpay-checkout.ts:101`)
- `connect-src 'self' ipc: … https://api.vilsend.in wss://api.vilsend.in` —
  the axios client and the control-plane socket
- `frame-src https://api.razorpay.com https://checkout.razorpay.com`
- `img-src 'self' data: blob:` — the QR code is a `data:` URL
- `object-src 'none'`, `base-uri 'self'`, `frame-ancestors 'none'`,
  `form-action 'self'`

`style-src` keeps `'unsafe-inline'`: Radix, framer-motion and recharts write
inline style attributes, and `src/components/ui/chart.tsx:79` injects a
`<style>` element via `dangerouslySetInnerHTML`. Tightening that is a separate
change with its own testing burden.

A separate `devCsp` keeps `localhost:1420` and its WebSocket working, so
`tauri dev` is unaffected. **Whether the production policy breaks anything can
only be settled by running the app** — see §6.

### 3.4 Task 1.5 — the pointer is stale; the real sites are in Rust

The task points at `src/lib/api.ts:14-44`. That file is already clean: its
interceptor logs the failure only, and line 38 says *"Never log the token or
the header value."* No change was needed there.

The live instances are five `println!` calls in Rust that report token
*presence*: `services/websocket_service.rs` (2), `state/websocket_state.rs`
(2) and `websocket/manager.rs` (2, plus a commented-out copy). All are removed,
and each site keeps its behaviour — the two `start` paths still return
`NotAuthenticated`, and the reconnect loop still waits and retries, now with a
structured `tracing::debug!` event instead of a presence flag. Two dead
bindings fall out with them (`token` in `WebSocketManager::start`, and
`token_for_loop`).

`services/cloudflared.rs:31` logs `"Tunnel token loaded"`. It is **kept**: it
records an operational step, carries no token material, and says nothing about
a session. Flagging it here so the decision is visible rather than silent.

### 3.5 Task 1.13 — fixed for macOS only

Both macOS binaries are now bundled. The identical defect exists for Linux —
`tauri.linux.conf.json` lists only `linux-x64` while the resolver expects
`linux-arm64` on aarch64 — and is **not** fixed, because CI has no Linux arm64
target and fixing it would add ~37 MB to a bundle that never needs it. See §6.

The ~37 MB cost of the macOS fix is accepted per the plan's own note
("bundling is simpler"). Bundling only the host architecture is not
expressible in a single static platform config.

---

## 4. Behaviour changes, in full

Everything else in this phase is deletion, config, or types. These two change
what the app does at runtime:

1. **Devtools are gone from release builds.** Intended; that is task 1.1.
2. **`stop_websocket` now exists**, so sign-out and teardown stop the
   control-plane socket instead of failing with "command not found" and
   leaving it running against a deleted session. Intended; that is task 1.6.

The CSP (§3.3) is a third, but it is enforcement of a policy that was
previously absent, and it is the item most likely to surprise you.

---

## 5. Acceptance criteria

| Criterion | Result | Evidence |
|---|---|---|
| `cargo check` produces zero warnings | ✅ **PASS** | 21 → 0. `cargo check --all-targets` after `touch src/lib.rs`. Baseline list preserved in the 1.7 commit message. |
| No `devtools` window in a release build | ✅ **PASS (by construction)** | The call is `#[cfg(all(desktop, debug_assertions))]`, the window flag is removed, and `open_devtools` does not exist in a release build without the `devtools` Cargo feature, which is off. `cargo check --release` result in §5.1. |
| `grep -rn "your-stronghold\|vilSend-strongHold" src-tauri/` returns nothing | ✅ **PASS** | No matches, target/ excluded. |
| `git ls-files src-tauri/src \| wc -l` drops by ≥ 14 | ✅ **PASS** | 80 → 61, a drop of 19. |
| App launches, signs in, sends and receives a file | ⛔ **NOT VERIFIED** | Requires a GUI run and a real peer. §6. |
| macOS arm64 build starts cloudflared successfully | ⛔ **NOT VERIFIED** | Requires an Apple Silicon Mac. §6. |

### 5.1 The release-mode check

`cargo check --release --all-targets` is the only local way to prove the
release configuration compiles without the `devtools` feature.

**Result: PASS.** `Finished \`release\` profile [optimized] target(s) in 3m
48s`, exit code 0, no warnings. Because the `devtools` Cargo feature is off
and `tauri::WebviewWindow::open_devtools` only exists under
`debug_assertions` or that feature, the release build compiling at all is what
proves the inspector call is compiled out — and therefore that a release build
cannot open one.

### 5.2 Other gates run

| Gate | Result |
|---|---|
| `cargo test --workspace` | Passes. **Zero tests exist** — the crate has no `#[test]` anywhere, and Phase 1's plan requires none ("there is no harness yet"). |
| Frontend `npm run typecheck` | Passes. |
| Frontend `npm run build` | Passes (15 s). |
| `npm run check:ipc` | Passes: 28 call sites, 33 registered commands. |
| `cargo fmt --check` | **Fails: 14 hunks in 8 files.** All pre-existing (module declaration ordering, two over-long lines, a doubled blank line). None introduced by this phase; the plan assigns the formatting gate to task 2.8. |
| `cargo clippy --all-targets` | 12 warnings, 7 lint classes (`upper_case_acronyms`, `unnecessary_cast`, `single_match`, `redundant_closure`, `needless_borrows_for_generic_args`, `manual_div_ceil`, `lines_filter_map_ok`). Not a Phase 1 gate; task 2.8 introduces it. |

---

## 6. Manual steps for the human

1. **Smoke-test the app.** Sign in, open the Razorpay checkout, and complete a
   real send and receive. Task 1.3's CSP and task 1.6's new command are both
   only provable by running the GUI. If the CSP breaks something, revert
   `9edcbb3` alone — it is self-contained.
2. **Do a release build** (`npm run tauri build`) and confirm (a) the version
   reported by the installer is `1.0.4`, and (b) no devtools can be opened.
   Task 1.14 removes `version` from `tauri.conf.json` in favour of
   `Cargo.toml`; that fallback is documented in `tauri-utils` but only a real
   bundle exercises the bundler's resolution.
3. **Deal with the local `.env`.** The signing key is not in the repository
   and never was, so this is housekeeping rather than an incident: move the
   value into your password manager and your CI secret and remove it from the
   working copy. Do **not** rotate it casually — the phase's rollback note
   explains that a rotation without a dual-key transition breaks auto-update
   for every installed client.
4. **Verify the macOS fix on an Apple Silicon Mac.** Bundle with
   `--config tauri.macos.conf.json` and confirm cloudflared starts. This is
   the one acceptance criterion this host cannot test.
5. **Dry-run the release workflow on a throwaway tag** before merging, because
   the MSIX job is currently broken (§7 D6) and because task 1.14 touched the
   version plumbing.

---

## 7. Defects found that the migration documents do not mention

Ordered by severity. D1 and D2 are fixed; the rest are not, per the phase rule
that discovered work outside the phase is recorded rather than done.

**D1 · `local_transfer_exists` is not a registered command — FIXED (`e3562b2`)**
`src/api/tauri.ts:217` invoked `local_transfer_exists`; the registered command
is `check_local_transfer_exists`. Same class as B1. The call site was
corrected; `localTransferExists` is currently unused, so nothing changes at
runtime today, but the name is reachable now.

**D2 · Pause, resume and cancel send the wrong argument name — NOT FIXED**
`src/api/tauri.ts:222-232` sends `{ id }`, but the Rust commands take
`transfer_id` (`commands/transfer_commands.rs:15,22,29`). Tauri maps
`transferId` → `transfer_id`, not `id` → `transfer_id`, so all three calls
fail argument deserialisation. These are **live**: `transfers-hooks.ts:80-86`
calls them. The fix is three one-word edits, but it makes a broken feature
work, which is a behaviour change and does not belong in containment.

**D3 · `Arc<UploadManager>` is never registered as Tauri state — NOT FIXED**
All five commands in `commands/transfer_commands.rs` take
`State<'_, Arc<UploadManager>>`, but `lib.rs` only calls `app.manage` for
`Arc<AppState>`, `Arc<LocalTransferFileService>` and `Cloudflared`. Every one
of those commands therefore fails with "state not managed" when invoked,
independently of D2. This is why transfers start from the `START_TRANSFER`
WebSocket command instead. One `app.manage(...)` line fixes it — but it is a
behaviour change.

**D4 · `server-event` and `websocket-message` are never emitted — NOT FIXED**
`src/contexts/websocket-context.tsx:19,25` listens for both. `grep -rn
"server-event" src-tauri/src` returns nothing: the only emitted events are
`auth-state-changed`, `auth-error`, `transfer-request`, `update-available` and
the six `transfer-*` progress events. So the context is inert — `status` never
leaves its initial value and `connected` is always `false`. Task 1.12 makes
that comparison correct; it does not make the status observable.

**D5 · Linux arm64 cloudflared has the same bundling bug as macOS — NOT FIXED**
See §3.5.

**D6 · The MSIX job reads a filename that does not exist — NOT FIXED**
`.github/workflows/release.yml:1213` reads
`src-tauri\msix\AppManifest.xml`. The file on disk is `AppxManifest.xml`
(`.github/workflows/release.yml:1216` copies it to `AppxManifest.xml`). The
"Prepare AppxManifest.xml" step therefore fails with
`AppManifest.xml was not found`, so the Microsoft Store build cannot produce a
package. One-word fix; outside this phase.

**D7 · `detect_device_type` had unreachable code — FIXED (`1a04f99`)**
On Windows the function `return`ed inside a `cfg` block and then fell through
to `"UNKNOWN".into()`. Rewritten as one expression per platform.

**D8 · Dead frontend listeners remain — NOT REPORTED FIXED**
`src/hooks/use-tauri-events.ts` is used nowhere (H9), and
`src/services/registerTransferNotificationListener.ts:9` duplicates the
`transfer-request` listener in `useDesktopServices.ts:294-314` (H11) — if both
ran, every request would be added twice. Both are outside the task list; note
that H10 (`src/store/websocket-store.ts`) **is** now fixed, as dead code in a
file task 1.12 touched.

**D9 · A fourth orphan file — FIXED (`345ac8d`)**
`src-tauri/src/state/receiver_state.rs` was undeclared and would not compile
(it names `PathBuf` and `Arc` with no imports).

---

## 8. Doc/code discrepancies

Where the code and the migration documents disagree, the code won. Each is
fixed in the document as well.

| # | Document claim | Reality |
|---|---|---|
| 1 | `00-current-state.md` §9.3 **H1**: `TAURI_SIGNING_PRIVATE_KEY` "is present in the committed root `.env`" | `.env` is untracked and has never been committed on any ref |
| 2 | `00-current-state.md` §3.2: `server-event` and `websocket-message` are emitted from `events/dispatcher.rs` | Neither is emitted anywhere (defect D4) |
| 3 | `05-migration-plan.md` task 1.5 points at `src/lib/api.ts:14-44` | That file already avoids logging the token; the live sites are in Rust |
| 4 | `05-migration-plan.md` acceptance: "`cargo check` … currently reports ~27" warnings | 21 |
| 5 | `00-current-state.md` §9.3 **H3**, §4.6: three case-duplicate orphan files | Three of those, plus a fourth unrelated orphan (D9) |
| 6 | `05-migration-plan.md` task 1.9: ten empty stub files | Eleven — `transfer/worker.rs` is also declared-but-empty |
| 7 | `06-testing-and-quality.md` §7.4 proposes generating a command manifest in CI and diffing against it | Implemented as a source-tree script (`scripts/check-ipc-contract.mjs`) that reads `generate_handler!` directly; no manifest or CI wiring yet |
| 8 | `00-current-state.md` §3.2 lists `update-available` as emitted but unlistened | Confirmed: emitted at `events/dispatcher.rs:209`, no `listen` in `src/` |

Nothing else in `00`–`07` was contradicted by the code while doing this work.
Claims re-verified and found accurate: the `stop_websocket` gap (B1), the
devtools defect (B3), the `ConnectionStatus` casing mismatch (B4), the
`csp: null` setting, the Stronghold stack being dead, the macOS arm64
cloudflared defect (§6.1), the commented-out `protocol.rs`, and the OAuth
implementation's quality.

---

## 9. Decisions needed

1. **D2 and D3** — both are one-line-to-three-line fixes for live, user-facing
   breakage (pause/resume/cancel of a transfer). Neither is in Phase 1's task
   list. Do you want a small `phase-1-followup` commit for them, or do they
   wait for the phase that rewrites the transfer commands?
2. **D4** — should the missing `server-event` emission be scheduled? Until it
   exists, `websocket-context` and the `ConnectionStatus` work in task 1.12 are
   dormant.
3. **D5** — bundle `linux-arm64/cloudflared` (+37 MB) or leave Linux arm64
   resolved-but-missing?
4. **D6** — fix the MSIX manifest path in `release.yml` now, or fold it into
   the Phase 2/3 split of that 1,799-line file?
5. **Task 1.2** — the repository is clean, so this is only about hygiene of
   your working copy: move the signing key out of `.env` into the password
   manager and CI secrets (no rotation)?
6. **`receiver_public_key`** — this phase keeps it under `#[allow(dead_code)]`
   because it is part of the `start_transfer` payload. The sender ignores it
   and fetches the key from `GET /transfer/public-key`. If you would rather
   delete it, that is a deliberate breaking change to the command contract and
   needs its own decision.
7. **Five more unreferenced dependencies** — `zeroize`, `anyhow`,
   `tauri-plugin-http`, `tauri-plugin-os`, `tauri-plugin-device`. Task 1.10
   named three; these five are additionally dead. The three plugin ones are
   the interesting case: their JS packages are not used by the frontend
   either, so no feature depends on them.
8. **`AppConfig.environment` / `api_url`** were deleted as unread. If either
   was intended for future use rather than forgotten, say so and I will
   restore it with its reader.

---

## 10. What Phase 1 deliberately did not do

- No refactor of the transfer engine, the receiver, or the auth service. This
  phase is containment; the architecture work starts at Phase 2.
- No behaviour fixes beyond the two in §4, even where a defect is plainly live
  and the fix is one line (D2, D3, D6).
- No `cargo fmt` sweep, no clippy fixes. Both gates are task 2.8's.
- No CI wiring for `scripts/check-ipc-contract.mjs`. It is runnable as
  `npm run check:ipc`; adding it to a PR gate belongs with whatever CI file
  Phase 2 creates.
- Roughly 70 `println!` debug statements remain across `websocket/`,
  `services/cloudflared.rs` and `commands/auth.rs`. Only those that reported
  token presence were removed (task 1.5). The rest are scaffolding for code
  that Phase 2 restructures, and removing them here would enlarge a
  containment diff for no security gain.
