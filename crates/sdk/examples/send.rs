//! Sends a file, and receives it.
//!
//! `05-migration-plan.md` § "Phase 5 — Acceptance criteria" asks for "a ~20-line
//! example in `crates/sdk/examples/send.rs` [that] completes a real transfer".
//! It completes a real one over [`VilsendBuilder::in_memory`]: real chunking,
//! real per-chunk integrity, a real handle, a real outcome.
//!
//! **It does not go over a network, and cannot yet.** The native backend needs
//! `vilsend-runtime` and `vilsend-engine`, which Phase 3 has not created; see
//! `VilsendBuilder::native`. Running this against a real receiver is a manual
//! step until then, and it is written down in
//! `docs/migration/smoke-checklist.md`.
//!
//! Run it with `cargo run -p vilsend-sdk --example send`.

use futures::executor::block_on;

use vilsend_sdk::{
    Destination, FileRef, MemoryFiles, PeerRef, ReceiveRequest, SendRequest, VilsendBuilder,
};

fn main() -> Result<(), vilsend_sdk::VilsendError> {
    let source = MemoryFiles::new();
    source.insert("hello.txt", b"hello, world".to_vec());

    let sink = MemoryFiles::new();

    let vilsend = VilsendBuilder::in_memory()
        .with_memory_source(source)
        .with_memory_sink(sink.clone())
        .build()?;

    block_on(async {
        let peer = PeerRef::from(vilsend_sdk::LOOPBACK_PEER);

        let receiving = vilsend
            .receive(ReceiveRequest::from(peer.clone(), Destination::root()))
            .await?;

        let sending = vilsend
            .send(SendRequest::to(peer, vec![FileRef::from("hello.txt")]))
            .await?;

        let sent = sending.wait().await?;
        receiving.wait().await?;

        println!("sent {} bytes in {} chunk(s)", sent.bytes(), sent.chunks());

        Ok(())
    })?;

    let received = sink.get("hello.txt").expect("the file arrived");

    println!("received: {}", String::from_utf8_lossy(&received));

    Ok(())
}
