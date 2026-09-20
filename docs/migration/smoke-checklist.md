# Smoke checklist

Manual steps that the automated suite cannot cover, because they need a GUI, a
second machine, the OS keychain, a real network, or a signed installer.

Phase 2's report asked for this file (§8 decision 7) and it did not exist. It
does now. Each phase appends to the section for its own phase rather than
rewriting the ones above it: a smoke test that stops being run is worse than one
that was never written, and the only way to notice is to see how long it has
been sitting there.

**Every step says what it is confirming and what failure looks like.** "It
works" is not a step.

---

## Phase 5 — `vilsend-sdk` facade

The SDK has no native backend yet, so most of what a shipped SDK needs is not
testable here. These are the steps that *are*.

### 5.1 — `examples/send.rs` completes a transfer

```sh
cargo run -p vilsend-sdk --example send
```

**Confirms:** the example in the repository runs, not just compiles.

**Expected:** `sent 12 bytes in 1 chunk(s)` then `received: hello, world`.

**Failure looks like:** a non-zero exit, or `sent` reporting zero chunks.

> This is an *in-memory* transfer. There is no receiver process and no network
> involved; the "real receiver" half of the Phase 5 acceptance criterion is
> **not met** and cannot be until Phase 3 exists — see
> [`reports/phase-5-report.md`](./reports/phase-5-report.md) §"Decisions
> needed". Running it against a real receiver is 5.3 below, once there is one.

### 5.2 — the doctest in the README is the README's example

```sh
cargo test -p vilsend-sdk --doc
```

**Confirms:** the published example is compiled and executed on every run, so it
cannot rot into something that no longer type-checks.

**Expected:** `1 passed`.

**Failure looks like:** a doctest failure naming `crates/sdk/src/lib.rs` — the
README changed and the example was not updated, or the API changed and the
README was not.

### 5.3 — `examples/send.rs` against a real receiver *(blocked)*

**Blocked by Phase 3.** This step is recorded rather than performed, so that it
is not quietly dropped.

Once `crates/runtime` and `crates/engine` exist and `VilsendBuilder::native()`
builds:

1. Start the desktop app on machine A and complete sign-in.
2. Start a `vilsend-receiver` (or the desktop app again) on machine B.
3. Run the example, pointed at B, over the tunnel.

**Confirms:** the facade drives the real engine end to end, and the desktop's
own transfer path and the SDK's produce the same result.

**Failure looks like:** the transfer completing on one side only, or a payload
that arrives byte-different.

### 5.4 — the desktop app still ships and still transfers

The desktop crate was **not** touched by Phase 5 (see the report's §"Files"),
but the phase boundary is a release boundary and the Phase 2 acceptance still
applies:

1. `npm run build && cargo build --workspace`.
2. Launch the app, sign in.
3. Send a file to a second signed-in install and confirm it arrives intact.
4. Confirm the sender's progress bar reaches 100 % and the receiver's file is
   byte-identical to the source.

**Confirms:** the phase did not regress the shipping product.

**Failure looks like:** anything in `docs/migration/reports/phase-2-report.md`
§"Acceptance criteria" that used to pass.

### 5.5 — the public-API gates run

```sh
cargo install cargo-public-api cargo-semver-checks
rustup toolchain install nightly-2026-09-19
cargo public-api --manifest-path crates/sdk/Cargo.toml > /tmp/public-api.txt
diff -u crates/sdk/public-api.txt /tmp/public-api.txt
```

**Confirms:** the committed snapshot matches the crate, so the CI job is
checking something. `cargo-semver-checks` has no baseline on the revision that
introduces the crate and says so — see the CI job's last step.

**Expected:** no diff.

**Failure looks like:** a diff. That means the public API changed; regenerate the
snapshot *in the same commit as the change*, and say in the commit message
whether the change is breaking.

---

## Standing checks (every phase)

These do not change, and they are here so that a release candidate is one list
rather than several.

| Step | Confirms |
|---|---|
| `cargo fmt --all --check` | formatting |
| `cargo clippy --workspace --all-targets -- -D warnings` | lints |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | lints, v2 wire compiled in |
| `cargo test --workspace` and `--all-features` | the suite, both configurations |
| `npm run typecheck && npm run build && npm run check:ipc` | the frontend, and that every `invoke` names a registered command |
| `cargo tree -p vilsend-core --prefix none` | the domain layer has no outbound adapter |
| Install the previous release and send a file to/from this build | the installed base still interoperates (ADR-0010) |
