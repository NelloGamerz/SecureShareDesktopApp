# vilsend-sdk

The stable public API for VilSend, and the surface every other product is
built on.

This README is the crate's documentation, and **the example below is compiled
and run by `cargo test -p vilsend-sdk --doc`** — it is not illustrative, it is
the test.

## The four verbs

```rust
use futures::executor::block_on;

use vilsend_sdk::{
    Destination, FileRef, MemoryFiles, PeerRef, ReceiveRequest, SendRequest,
    VilsendBuilder, LOOPBACK_PEER,
};

fn main() -> Result<(), vilsend_sdk::VilsendError> {
    block_on(run())
}

async fn run() -> Result<(), vilsend_sdk::VilsendError> {
    // The bytes to send, and somewhere for the received bytes to land. Both
    // are in memory: this client touches nothing external.
    let source = MemoryFiles::new();
    source.insert("report.txt", b"hello from VilSend".to_vec());

    let sink = MemoryFiles::new();

    let vilsend = VilsendBuilder::in_memory()
        .with_memory_source(source)
        .with_memory_sink(sink.clone())
        .build()?;

    let peer = PeerRef::from(LOOPBACK_PEER);

    // Verb 2: receive. This registers a listener and hands back a handle.
    let receiving =
        vilsend.receive(ReceiveRequest::from(peer.clone(), Destination::root())).await?;

    // Verb 1: send. It finds the listener waiting on `peer` and adopts its id,
    // so both handles name the same transfer.
    let sending = vilsend
        .send(SendRequest::to(peer, vec![FileRef::from("report.txt")]))
        .await?;

    // Verb 4: events. Each call is an independent live subscription; dropping
    // the stream is how a host unsubscribes.
    let _events = vilsend.events();

    // Verb 3: auth. `in_memory()` has no session, and cannot have one.
    assert_eq!(
        vilsend.auth().await.state(),
        vilsend_sdk::AuthState::SignedOut
    );

    sending.wait().await?;
    let received = receiving.wait().await?;

    assert_eq!(received.bytes(), 18);
    assert_eq!(sink.get("report.txt"), Some(b"hello from VilSend".to_vec()));

    Ok(())
}
```

Any executor drives it. The SDK never names one in a public signature — no
`tokio::` appears in the example above, and none appears in this crate's
dependency tree.

## What is here, and what is not

| Verb | Works today |
|---|---|
| `send` | yes, over the in-memory backend |
| `receive` | yes, over the in-memory backend |
| `auth` | the shape only — Phase 6 supplies the provider |
| `events` | yes |

`VilsendBuilder::in_memory()` is fully implemented and is what every test in
this repository uses. `VilsendBuilder::native()` exists and **refuses to
build**, with an error naming what is missing: the file, credential and
transport adapters that `01-target-architecture.md` §4.1 places in
`vilsend-runtime`, and the engine it places in `vilsend-engine`. Neither crate
exists yet — Phase 3 of `docs/migration/05-migration-plan.md` creates them. See
`docs/migration/reports/phase-5-report.md`.

The types `01-target-architecture.md` §4.1 says the native backend needs —
`ChunkSource`, `ChunkSink`, `CredentialStore`, `TransferStore`, `AuthProvider`,
`Transport`, `Policy` — are deliberately **absent** from this crate rather than
stubbed. A stub is a shape a caller can write against and that will then have
to be kept compatible; the phase that owns each port should introduce it.

## Semver policy

This crate is the product boundary for third parties, so it is treated as a
semver-stable artifact from day one — including while it is pre-1.0, because
the moment it is published someone will pin it.

| Rule | Detail |
|---|---|
| **Strict semver** | Breaking changes require a major version. There is no "it was only a patch" exception. |
| **Pre-1.0** | At `0.x`, semver permits a minor release to break. **This crate will not.** A break takes a major bump, and `0.1` is published with the documented intent that it will not break without one. Intent is not a contract; the version number is. |
| **Enums are open** | Every public enum is `#[non_exhaustive]`. *Adding* a variant is not a breaking change. *Changing* or *removing* one is. A caller should have a catch-all arm, and `crates/sdk/tests/public_api.rs` fails the build if a public enum is declared without the attribute. |
| **The one closed struct** | `Policy` is deliberately **not** `#[non_exhaustive]`, because a configuration struct a caller has to fill in needs `..Policy::default()` — which a non-exhaustive struct forbids. Adding a field to `Policy` *is* a breaking change. Every other public struct (the outcome DTOs, `Progress`) is `#[non_exhaustive]` and read-only. |
| **No lifetimes** | No public type carries a lifetime parameter (`04` §3.3 rule 2). Everything is owned or `Arc`. `crates/sdk/tests/public_api.rs` checks this too. |
| **No shell types** | No public item names `tauri`, `axum`, `sqlx`, `keyring` or `reqwest`. The crate does not depend on them at all, directly or transitively. |
| **Deprecation** | `#[deprecated(since = "x.y.0", note = "...")]`, kept for at least one minor release before removal. |
| **Internals are not public** | `vilsend-core`, and later `vilsend-engine` and `vilsend-protocol`, are **not** published and may break in any release. Only types the SDK re-exports are covered by this policy. |
| **The protocol version is not the crate version** | The wire contract is `vilsend_core::PROTOCOL_VERSION`, an independent `u16`. Bumping this crate changes nothing on the wire, and nothing about the wire may bump this crate. |

### How the policy is enforced

Two gates, and neither replaces the other:

- **`cargo-semver-checks`** — "strict semver" and "the one closed struct" are
  claims about the API, and this tool is what makes them mechanical. It runs in
  CI (`.github/workflows/ci.yml`, the `public-api` job) against the last
  release; the first release has nothing to compare against, and the job says
  so rather than passing quietly.
- **A committed public-API snapshot** (`public-api.txt`, `cargo public-api`) —
  regenerated and diffed in the same job. `cargo-semver-checks` tells you
  whether a change is *breaking*; a snapshot tells you that a change *happened*,
  which is what a reviewer needs to see in the diff.

Nothing is published: the crate is `publish = false` until Phase 7 is complete
and the CLI has exercised the surface (ADR-0005).

## See also

- [`docs/migration/04-sdk-cli-mobile-build-plan.md`](../../docs/migration/04-sdk-cli-mobile-build-plan.md) — the API design this implements
- [`docs/migration/adr/0003-unified-error-model.md`](../../docs/migration/adr/0003-unified-error-model.md) — why errors are enums
- [`docs/migration/adr/0011-testing-and-contract-suites.md`](../../docs/migration/adr/0011-testing-and-contract-suites.md) — why `in_memory()` is a contract
