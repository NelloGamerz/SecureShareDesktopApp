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

## See also

- [`docs/migration/04-sdk-cli-mobile-build-plan.md`](../../docs/migration/04-sdk-cli-mobile-build-plan.md) — the API design this implements
- [`docs/migration/adr/0003-unified-error-model.md`](../../docs/migration/adr/0003-unified-error-model.md) — why errors are enums
- [`docs/migration/adr/0011-testing-and-contract-suites.md`](../../docs/migration/adr/0011-testing-and-contract-suites.md) — why `in_memory()` is a contract
