# Phase 4a — Crypto v2 · Report

**Sub-phase:** `4a` — tasks 4.1, 4.2, 4.4, plus the `PROTOCOL_VERSION` constant,
the `protocol-v2` feature flag, and the wire-compatibility suite.
**Branch:** `phase-4a-protocol-crypto-v2` (from `phase-2-workspace-core` @ `c005748`)
**Commits:** seven — `5a24010` … `1997e25`
**Plan:** [`05-migration-plan.md`](../05-migration-plan.md) § "Phase 4 — Protocol
hardening + resume persistence"
**Read:** `README.md`, `05` § Phase 4, ADR-0007 (AAD section), ADR-0010,
`02-transport-layer.md` §8, `06-testing-and-quality.md` §5/§6.2, the Phase 1 and
Phase 2 reports (including their "Decisions needed").
**Diff:** 12 files changed, 2,974 insertions, 3 deletions — of which roughly
2,250 lines are tests or test fixtures (344 at the end of `crypto.rs`, 909 in
`crypto_v2.rs`, 958 across `compat_v1.rs` and `v1_fixture/mod.rs`, 39 in
`protocol.rs`).

---

## 1. Summary

Phase 4a adds the v2 chunk crypto — associated data, a salted KDF, and a bounded
nonce-repeat window — behind a `protocol-v2` feature flag that is **default
off**, together with the `PROTOCOL_VERSION` constant ADR-0010 asks for and the
wire-compatibility suite the brief requires once, in this first sub-phase.

**Nothing on the wire changed.** The v2 module is not called by any production
path: the endpoints that will call it are task 4.11 and the sender that will
choose between v1 and v2 by capability probe is task 4.11 as well. A build with
`protocol-v2` on still speaks v1, and that is asserted rather than reasoned
about (§3.4).

**Two things must be read before anything else in this report:**

1. **The `SUBPHASE` field in the brief was not substituted.** The brief arrived
   with `SUBPHASE = <4a | 4b | 4c | 4d | 4e>   ← I will replace this before
   running` still a placeholder. Rule 12 forbids asking, so this went ahead with
   **4a**, on the grounds that it is the first sub-phase, that the brief binds
   the one-off compatibility suite to "the first sub-phase you are asked to
   run", and that 4a defines `PROTOCOL_VERSION` and the feature flag that 4b–4e
   all build on — so it is a prerequisite for every other choice and the
   lowest-regret default. **If a different sub-phase was intended, say so; the
   work here is a prerequisite for it and stays.** Recorded again in §8.

2. **Phase 3 has not been implemented, and Phase 4 depends on it.** The plan's
   dependency graph is `P3 → P4`. `docs/migration/reports/` contains only
   phases 1 and 2, and `crates/` contains only `core` and `desktop` — no
   `crates/runtime`, no `ChunkSource`/`ChunkSink`, no `Clock`, no
   `CredentialStore`, no `TransferStore`. This does not block 4a, which is
   self-contained crypto, but it **does** block 4c (`TransferStore`,
   `CredentialStore`) and 4d (the `writer.rs`-successor locks), and it blocks
   the target crate layout the plan and `06` §5 assume for this phase. See §8.

---

## 2. Per-task status

| Task | Commit | Status | Notes |
|---|---|---|---|
| 4a.0 characterise v1 crypto | `5a24010` | ✅ done | Not a plan task; rule 6 requires it before anything is refactored beside it |
| 4a `PROTOCOL_VERSION` + flag | `2a606ea` | ✅ done | ADR-0010 decision 5; flag forwarded from `vilsend-core` |
| 4.1 AAD | `947d2c9` | ✅ done | Framing decision recorded in §4.1 |
| 4.2 HKDF salt | `6120a50` | ✅ done | Both nonces, order frozen |
| 4.4 nonce discipline | `d230ba2` | ✅ done | Bounded sliding window; limit in §7 |
| 4a compat test | `4b298b1` | ⚠️ done as a fixture | A pinned v1 binary is not producible — §6 |
| (CI) name the gate | `1997e25` | ✅ done | `06` §6.2 gives the wire-compat suite its own job |

Commit order is the plan's task order, except 4a.0 which precedes everything
because rule 6 requires characterisation tests on untested code *before* it is
touched or duplicated.

---

## 3. Acceptance criteria

The Phase 4 acceptance list, with the 4a items marked.

### 3.1 ✅ "A captured v2 chunk replayed at a different index is **rejected**"

**Pass.** Three independent tests:

* `crypto_v2::tests::a_chunk_replayed_at_a_different_index_is_rejected` — seal at
  index 3, open at index 99, `ErrorKind::IntegrityMismatch`.
* `crypto_v2::tests::a_chunk_replayed_into_another_slot_is_rejected` — the same
  capture replayed into a different transfer, a different file and a different
  path, each asserted separately because each is a separate route.
* `crypto_v2::tests::a_chunk_replayed_at_another_index_is_rejected_through_the_window`
  — the same property through the entry point the receiver will actually use.

The v1 counterpart is *deliberately* the inverse:
`crypto::tests::the_v1_aead_does_not_bind_a_chunk_to_its_index` asserts that a v1
chunk **is** replayable, so that "fixing" v1 fails the build.

### 3.2 ✅ Tamper tests: flip ciphertext, AAD, nonce → all fail closed

**Pass.** `crypto_v2::tests`: `a_flipped_ciphertext_byte_fails_closed`,
`a_flipped_tag_byte_fails_closed`, `a_flipped_nonce_byte_fails_closed`,
`a_flipped_aad_byte_fails_closed`,
`a_flipped_aad_length_prefix_fails_closed` (the framing is covered, not just the
payload), `a_different_key_fails_closed`, and
`an_empty_aad_is_still_covered_by_the_tag`.

The v1 module has its own equivalent set, pinning v1's fail-closed behaviour as
it already is.

### 3.3 ✅ Crypto vectors: fixed key + nonce + AAD → fixed ciphertext

**Pass.** Four frozen values, all by hex literal:

| Vector | Test |
|---|---|
| v1 transfer key from `[0x42; 32]` | `crypto::tests::the_v1_kdf_is_hkdf_sha256_without_a_salt_and_a_frozen_info_string` |
| v1 transfer key from `[0x00; 32]` | `crypto::tests::the_v1_kdf_varies_with_its_input` |
| v2 transfer key from `[0x42;32]` + nonces | `crypto_v2::tests::the_v2_kdf_vector_is_frozen` |
| v2 chunk ciphertext, fixed key/nonce/AAD | `crypto_v2::tests::the_v2_chunk_vector_is_frozen` |
| v2 AAD encoding | `crypto_v2::tests::the_v2_aad_vector_is_frozen` |

Plus `the_aad_encoding_is_pinned_byte_for_byte`, which spells the expected bytes
out field by field.

### 3.4 ✅ v1↔v2 matrix: v1 → v1

**Pass.** `tests/compat_v1.rs`, seven tests, green in both feature
configurations:

| Test | What it pins |
|---|---|
| `a_transfer_to_a_v1_receiver_completes_byte_for_byte` | The floor. Also asserts staging cleanup matches v1's exactly. |
| `a_transfer_to_a_v1_receiver_completes_with_chunks_out_of_order` | The normal case at concurrency 4. |
| `re_sending_a_chunk_is_a_no_op_on_a_v1_receiver` | The `.part` gate that `02` §5.4 promises and no more. |
| `the_fixture_rejects_a_chunk_it_cannot_decrypt` | Guards the fixture: without it the round trips would pass against a receiver that accepts anything. |
| `the_fixture_requires_every_v1_chunk_header` | Guards the fixture: every header in the frozen set is required, and the complete set is accepted. |
| `a_v2_sealed_chunk_is_rejected_by_a_v1_receiver` *(protocol-v2)* | Proves 4.1's AAD reaches the wire rather than being computed and discarded. |
| `a_protocol_v2_build_speaks_v1_and_reports_it_as_the_fallback` *(protocol-v2)* | The "must work" cell: compiling the v2 wire in changes nothing outbound until 4.11 adds capability probing. |

**The asymmetric pinning was verified, not asserted.** A first draft had the
test-local sender derive its own v1 key as well as the fixture. That draft
**passed with the product's v1 KDF deliberately corrupted**, which means it
proved nothing about the product. Rewritten so the sender uses the product's
`transfer::crypto` and only the receiver is independent, changing the v1 `info`
string then failed **four of the five** tests. The check is reproducible:

```sh
sed -i 's/carsdv-transfer-key-v1/carsdv-transfer-key-v2/' crates/desktop/src/transfer/crypto.rs
cargo test -p vilsend --test compat_v1     # 4 failed, 1 passed
```

### 3.5 ⏸ Remaining Phase 4 criteria — not this sub-phase

| Criterion | Sub-phase | Status |
|---|---|---|
| Truncated file → `IntegrityMismatch` | 4b | not started |
| Kill mid-transfer, restart, resumes with ≤ 1 chunk re-sent | 4c | not started |
| 4 concurrent transfers do not serialize | 4d | not started |
| Bounded memory, fast producer / slow consumer | 4d | not started |
| 404 fallback works | 4e | not started |

---

## 4. Design decisions

Everything here is a decision the plan or the ADRs left open. Each is reversible
only by a protocol renumbering, which is why they are written down.

### 4.1 The AAD is length-prefixed, not concatenated

ADR-0007 decision 4 writes `AAD = transfer_id ‖ file_id ‖ chunk_index ‖
relative_path ‖ proto_v`. That `‖` reads as "concatenated", and taken literally
it is **ambiguous**: `("ab", "c")` and `("a", "bc")` produce identical bytes, so
a chunk sealed for one transfer would open for another. Every field but
`chunk_index` is variable-length, so the collision is reachable, and ambiguity in
a MAC's associated data is a defect rather than a style question.

So each string is written as `u32_le(byte_len) ‖ bytes`. The field set and order
are the ADR's exactly; only the framing is pinned down, in the doc comment and in
`the_aad_encoding_is_pinned_byte_for_byte`. Two tests assert the collision stays
unreachable.

### 4.2 The AAD is written by position, not by matching values

A transfer and a file may legitimately share an id. An encoding that emits the
index when it sees `file_id` writes it twice in that case.
`the_index_is_written_exactly_once_when_two_ids_are_equal` asserts the length.

### 4.3 Two KDF separations, not one

v2 differs from v1 in the salt **and** in the `info` string
(`b"vilsend-transfer-key-v2"` against v1's `b"carsdv-transfer-key-v1"`).
`the_info_string_alone_separates_v1_from_v2` asserts that even with an empty salt
— which HKDF treats as no salt, because HMAC zero-pads an empty key and a
32-zero-byte key to the same block — the two keys differ. That case is
unreachable in the protocol; it is stated as a property of the KDF.

### 4.4 `seal`/`open` take an explicit nonce; `NonceWindow` is the entry point

Keeping the AEAD a pure function of its inputs is what makes the frozen vector
possible. It also puts nonce generation in exactly one place, so the bounded
repeat check cannot be bypassed by a caller that forgot about it. There is
deliberately no nonce-less convenience wrapper on the free functions.

`NonceWindow::open` verifies the tag **first** and records the nonce **only on
success** — an unauthenticated chunk has not spent its nonce, and letting forged
chunks evict real window entries would turn a replay defence into a DoS against
the sender. Asserted by `a_tampered_chunk_does_not_spend_its_nonce`.

`NonceWindow::seal` records the nonce **before** the AEAD runs, so a seal failure
still spends it.

### 4.5 One window per direction

The sender holds a window so it never emits a repeat; the receiver holds a
different one so it never accepts one. They are not shared. Two tests in this
sub-phase failed on the shared version before the docs said so, which is why the
type documentation leads with it.

### 4.6 Nonce window capacity: 4,096, ~48 KiB

The plan's risk table says to bound it. A full set for a 100 GB transfer at 4 MiB
chunks is 25,600 entries and grows without limit; 4,096 covers the last 4,096
chunks, which at the default chunk size is a 16 GiB span. Eviction is asserted as
*behaviour* (`an_evicted_nonce_is_accepted_again_which_is_what_makes_it_a_window`)
so it cannot be silently turned back into a set.

---

## 5. Where the code lives, and the discrepancy with the plan

The plan and `06-testing-and-quality.md` §5 assume a crate layout this phase
cannot use, because Phase 3 has not run:

| Document says | Reality here | Why |
|---|---|---|
| `vilsend-crypto` crate with the vectors (`06` §5, `01` §table) | `crates/desktop/src/transfer/crypto_v2.rs` | No such crate exists; creating one is not a 4a task, and a new crate needs an ADR (rule 11) |
| `cargo test -p vilsend-engine --test compat_v1` (`06` §6.2) | `crates/desktop/tests/compat_v1.rs` | Neither `vilsend-engine` nor that path exists yet |
| `vilsend-engine` for the fixture's receiver | `crates/desktop/tests/v1_fixture/mod.rs` | Same |

The v1 crypto stays in `transfer/crypto.rs` and the v2 crypto is a **sibling
module**, not a flag inside it. Two modules rather than one is what makes it
structurally impossible for the two to share a key derivation by accident, and
it keeps the frozen file frozen.

Integration tests cannot see a private module and are compiled without `--cfg
test`, so `lib.rs` gained a `#[doc(hidden)] pub mod wire` facade exposing exactly
what `tests/` needs. It is not an API surface — Phase 5 replaces it with the
`vilsend-sdk` facade — and the alternative was a test that re-implements the code
it is testing.

---

## 6. The pinned v1 receiver: what was built and why it is a fixture

The brief requires a pinned v1 receiver, and allows a fixture built from the
frozen schema if a pinned build cannot be produced, with the limitation stated.

**A pinned v1 binary cannot be produced.** Verified, not assumed:

* `v1.0.4` is a real tag in this repository, so the source exists.
* At that tag, `src-tauri/src/transfer/writer.rs` takes a `tauri::AppHandle` and
  uses it **on the request path** for three things:
  `resolve_download_root` (`app.store("settings.json")`, `app.path().download_dir()`)
  and `KeyringService::get_device_private_key` for the device's long-term X25519
  key. None is constructible without a live Tauri application, so the receiver
  cannot be started headless in CI.
* The tag predates the Phase 2 workspace (`Cargo.toml` does not exist at
  `v1.0.4`), so it builds as a desktop app rather than as a crate a test can
  link.

So `crates/desktop/tests/v1_fixture/mod.rs` is a complete v1 receiver written
from the frozen schema: the three endpoints, the eight-header set, the
presence-only `Authorization` check, the v1 KDF, the v1 AEAD with no associated
data, and the `.part`-existence gate.

**It is deliberately faithful where v1 is wrong.** It checks `Authorization` for
presence only, because that is the High finding in `docs/SECURITY.md:5-18` and
Phase 9's to fix; a fixture that hardened it would be testing a receiver that does
not exist in the field. It also leaves `incoming/{transfer}/` behind after a
successful merge, exactly as v1 does, rather than tidying up.

**It does not call the product's crypto** — see §3.4 for why that is what makes
the suite load-bearing, and for the reproduction that confirms it.

---

## 7. Limits and gaps in this sub-phase

1. **The nonce window is in memory and per session.** A receiver that restarts
   loses it, so a chunk captured before the restart and replayed into the *same*
   slot after it is caught neither by this window nor by the AAD (which catches
   replays into a different slot). Task 4.7's resume bitmap is what covers that
   case. Task 4.4 does not ask for the window to be persisted and it is not, but
   this is a real hole in the composed defence and is recorded rather than
   claimed closed.
2. **`NonceWindow` does not scale to a per-transfer key ring.** It is one window
   per transfer key by construction; 4.11 owns how many exist at once and when
   they are dropped.
3. **Nothing is wired.** The v2 module has no production caller, so the feature
   flag currently gates only tests. That is the plan's intent for 4a — the
   endpoints are 4.11 — but it means "v2 works" is not yet a statement about the
   application, only about the primitives.
4. **`vilsend-core` gained constants, not a code path.** `PROTOCOL_VERSION` is
   read only by the feature-gate tests so far.
5. **`cargo doc` is not wired and the feature-gated module reference is plain
   text.** `crypto.rs`'s header names `crate::transfer::crypto_v2` in backticks
   rather than as an intra-doc link, because the link target does not exist when
   the feature is off. Minor, but it is the kind of thing that compounds.

---

## 8. Decisions needed

1. **Which sub-phase did you mean?** The `SUBPHASE` placeholder was not
   substituted (§1). 4a was implemented. This is a prerequisite for 4b–4e either
   way, so nothing here is wasted, but confirm before the next run.

2. **Phase 3 has not been implemented and Phase 4 depends on it.** The plan's
   graph is `P3 → P4`; only phases 1 and 2 have reports, and `crates/` holds only
   `core` and `desktop`. Concretely:
   * **4c cannot be implemented as written** — it needs `TransferStore` (task
     3.4) and `CredentialStore` (task 3.3), neither of which exists.
   * **4d cannot be implemented as written** — it replaces the global write
     mutex "at `writer.rs`-successor code", and the successor of `writer.rs` is
     task 3.7.
   * **4e is affected** — the `/v2/*` endpoints belong in the receiver module
     that task 3.6 extracts.
   * 4b is affected in the same way 4a was: it can be built as a module, but
     `transfer/checksum.rs` (0 bytes today) and the merge in `writer.rs` are the
     Phase 3 refactor's.
   Options: (a) implement Phase 3 first, then resume 4b; (b) treat this as a
   replan and fold 3.3/3.4/3.6/3.7 into 4c/4d — that is a plan change, not mine
   to make; (c) proceed with the sub-phases that are phase-independent, which is
   only 4b. **This is the blocking one.**

3. **Key-derived material is logged at `INFO` in two places** (§9 D1). This is a
   defect I found and did **not** fix, because it is not 4a's and rule 2 says to
   record rather than do. It is a two-file, six-line deletion with no wire
   impact: fix now, or assign it?

4. **`01-target-architecture.md:179` lists "key fingerprints" as a
   `vilsend-crypto` responsibility**, which reads as intent. If fingerprints are
   meant to stay, the question is *where they may go* — a log line at `INFO` with
   no rotation and no access control is not a safe place for a value that is a
   deterministic function of the transfer key. Worth deciding before 4c persists
   anything key-adjacent.

5. **The AAD framing decision (§4.1) is a deviation from the ADR's literal
   text.** It preserves the intent and closes a real ambiguity, but ADR-0007
   decision 4 and ADR-0010 should be updated to say "length-prefixed" so the next
   implementation does not re-derive it. Want the ADR edits in a follow-up commit
   here, or in the docs pass at the end of Phase 4?

6. **`06` §5 and §6.2 name crates and paths that do not exist** (§5). Do you want
   `06` corrected now to point at `crates/desktop`, or left until Phase 3 creates
   the target layout and the references become true?

7. **Phase 2's open decisions are still open** and unchanged by this phase, except
   that decision 5 (`lines().flatten()`) was resolved by `c005748`, which
   supersedes it. Open: Phase 2 §8 items 1–4, 6–8, and Phase 1's D2/D3/D4/D6.

---

## 9. Defects found that the migration documents do not mention

### D1 — the sender's and receiver's transfer key are logged as a SHA-256 hash at `INFO`

`crates/desktop/src/transfer/manager.rs:111-116` and
`crates/desktop/src/transfer/writer.rs:522-527` both compute
`Sha256::digest(transfer_key)` and emit it as `transfer_key_hash`. It lands at
`INFO`, which is the level a release build runs at (Phase 1 task 1.4), on both
ends of every transfer, with no rotation and no redaction.

A transfer key is not recoverable from its hash, so this is not a key disclosure.
It is still key-derived material in a log: it is a stable per-transfer
identifier that ties two peers' logs together, and it is an oracle an attacker
with log access can use to *confirm* a guessed key. The stated purpose is
debugging two peers' key derivation agreeing — which the handshake's own failure
already tells you, more cheaply.

**Recommendation:** delete both blocks. Six lines, no wire impact, no test
depends on them. Not done here (rule 2).

### D2 — Phase 1's open items are still open, and one is now closer

D3 (`Arc<UploadManager>` never registered as Tauri state) and D4 (`server-event`
/ `websocket-message` never emitted) are unchanged. D2 (`pause`/`resume`/`cancel`
send `id` instead of `transfer_id`) is unchanged and unblocked.

---

## 10. Doc/code discrepancies

Where the documents and the code disagree, the code wins and the document is
wrong. None of these is corrected here — see §8 decisions 5 and 6, and rule 2.

| Doc | Says | Code / reality |
|---|---|---|
| `05` Phase 4 dependency | Phase 4 blocked by Phase 3 | True, and Phase 3 has not run — §8 decision 2 |
| ADR-0007 decision 4, ADR-0010 | `AAD = a ‖ b ‖ c` | Implemented length-prefixed; the `‖` is notation, not a wire instruction — §4.1 |
| `06` §5, `01` §table | `vilsend-crypto` owns the vectors | `crates/desktop/src/transfer/crypto_v2.rs` |
| `06` §6.2 | `cargo test -p vilsend-engine --test compat_v1` | `cargo test -p vilsend --test compat_v1`, in `crates/desktop` |
| `06` §6.2 | CI has a `wire-compat` job | `ci.yml` has one `rust` job; the gate is now a named step inside it (`1997e25`) |
| `01` §table | "key fingerprints" are a crypto responsibility | Two `INFO`-level logs of a transfer-key hash; see D1 |
| `05` Phase 4 tasks 4.5–4.7, 4.10 | Assume `TransferStore`/`CredentialStore` | Neither exists — §8 decision 2 |

---

## 11. Manual steps for the human

1. **Cross-version test with the last released installer (1.0.4).** Not possible
   here — it needs a Windows/macOS GUI run and a second machine. The manual
   script: install 1.0.4, sign in on both ends, send a file **from** a
   `protocol-v2` build **to** the 1.0.4 install over the tunnel, and confirm the
   file arrives intact. The automated suite covers the same code path against the
   fixture; this is what confirms the fixture is faithful.
2. **Confirm the sub-phase** (§8 decision 1) and the **Phase 3 sequencing**
   (§8 decision 2) before the next run — 4c and 4d cannot start as written.
3. **Decide on D1** (§9) — the two `INFO` log lines.
4. Optionally, verify the per-commit gates on this branch in CI. Locally:
   `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
   `cargo test --workspace`, `cargo test --workspace --all-features`,
   `npm run typecheck`, `npm run build`, `npm run check:ipc`.

---

## 12. Test and gate evidence

Baseline at `c005748`: **41** tests across the workspace, all passing.

At `1997e25`:

| Configuration | Tests | Result |
|---|---|---|
| `cargo test --workspace` | 70 | all pass |
| `cargo test --workspace --all-features` | 125 | all pass |
| `cargo test -p vilsend --test compat_v1 --features protocol-v2` | 7 | all pass |

29 tests are added in the default configuration, 84 with all features. Of
those 84: 53 are `crypto_v2`, 22 are the v1 characterisation set in `crypto`,
7 are the compatibility suite, and 2 are the `PROTOCOL_VERSION` gate tests —
one per configuration, because the `cfg(feature)` pair compiles exactly one of
them.

Gates, all green in both feature configurations:

* `cargo fmt --all --check`
* `cargo clippy --workspace --all-targets -- -D warnings`
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`
* `cargo test --workspace` and `--all-features`
* `cargo build --workspace` (the desktop app builds)
* `npm run typecheck`, `npm run build`, `node scripts/check-ipc-contract.mjs`
* the architecture lint, negated form:
  `! cargo tree -p vilsend-core --prefix none | grep -E '^(tauri|reqwest|axum|sqlx|keyring)( |$)'`
  — passes; `vilsend-core` gained no dependency.

**CI is now two configurations, not one.** The `rust` job lints and tests with
and without `--all-features`. Neither is a subset of the other, and without the
second pass the entire v2 crypto — vectors, tamper tests, the AAD rejection test
— is not compiled at all and would be silently skipped rather than failing.

---

## 13. Files

### Added

| File | Lines | What |
|---|---|---|
| `crates/desktop/src/transfer/crypto_v2.rs` | 1,451 | AAD, salted KDF, nonce window, 53 tests |
| `crates/desktop/tests/compat_v1.rs` | 534 | The compatibility suite |
| `crates/desktop/tests/v1_fixture/mod.rs` | 424 | The frozen-schema v1 receiver |
| `crates/core/src/protocol.rs` | 99 | `PROTOCOL_V1`, `PROTOCOL_V2`, `PROTOCOL_VERSION` |

### Changed

| File | What |
|---|---|
| `crates/desktop/src/transfer/crypto.rs` | Module header marking it frozen; 22 characterisation tests |
| `crates/desktop/src/transfer/mod.rs` | Registers `crypto_v2` behind the feature |
| `crates/desktop/src/lib.rs` | `#[doc(hidden)] pub mod wire` |
| `crates/desktop/src/transfer/http_client.rs` | `impl Default` (clippy, on newly-reachable type) |
| `crates/core/src/lib.rs`, `crates/core/Cargo.toml` | Export the constants; declare `protocol-v2` |
| `crates/desktop/Cargo.toml` | Forward `protocol-v2` |
| `.github/workflows/ci.yml` | Both feature configurations; the named wire-compat step |

### Not touched

`transfer/writer.rs`, `transfer/manager.rs`, `transfer/upload.rs`, the
frontend, and the v1 wire format. `git diff c005748 HEAD -- src/` is empty, and
the only change in the desktop crate outside `crypto_v2.rs`, `mod.rs` and
`lib.rs` is `impl Default for HttpClient` (five lines, no behaviour change).
