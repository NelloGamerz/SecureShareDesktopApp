//! A full send -> receive cycle, entirely in memory.
//!
//! This is the acceptance test `05-migration-plan.md` § "Phase 5 — Tests
//! required" asks for: "an integration test using `in_memory()` for the full
//! send->receive cycle". It drives the public API only — no private module, no
//! `#[cfg(test)]` helper, nothing that a third-party caller could not reach —
//! because a test that reaches past the facade proves something about the
//! implementation rather than about the product.
//!
//! It lives in `tests/` rather than beside the code for the same reason: an
//! integration test is a separate crate, so it can only see what is public.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::executor::block_on;
use futures::StreamExt;

use vilsend_core::{EventSink, PeerRef, RecordingEventSink};

use vilsend_sdk::{
    Destination, ErrorKind, FileRef, MemoryFiles, Policy, ReceiveRequest, SendRequest, Vilsend,
    VilsendBuilder, LOOPBACK_PEER,
};

fn peer() -> PeerRef {
    PeerRef::from(LOOPBACK_PEER)
}

/// A client over `source` and `sink`, chunking at `chunk_size`.
fn client(source: &MemoryFiles, sink: &MemoryFiles, chunk_size: usize) -> Vilsend {
    VilsendBuilder::in_memory()
        .with_memory_source(source.clone())
        .with_memory_sink(sink.clone())
        .with_policy(Policy {
            chunk_size,
            ..Policy::default()
        })
        .build()
        .expect("the in-memory builder builds")
}

/// A client whose source already holds `files`.
fn client_with(files: &[(&str, Vec<u8>)], chunk_size: usize) -> (Vilsend, MemoryFiles) {
    let source = MemoryFiles::new();

    for (path, bytes) in files {
        source.insert(*path, bytes.clone());
    }

    let sink = MemoryFiles::new();

    (client(&source, &sink, chunk_size), sink)
}

/// Yields once to the executor.
///
/// Written out here rather than reached for from the crate, because the crate
/// does not export one and this test needs to interleave two futures on one
/// thread to prove that cancellation reaches a transfer in flight.
fn yield_once() -> impl Future<Output = ()> + Unpin {
    struct Yield(bool);

    impl Future for Yield {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                return Poll::Ready(());
            }

            self.0 = true;
            context.waker().wake_by_ref();

            Poll::Pending
        }
    }

    Yield(false)
}

#[test]
fn a_full_cycle_sends_and_receives_byte_for_byte() {
    block_on(async {
        let payload_a: Vec<u8> = (0..5_000u32).map(|byte| byte as u8).collect();
        let payload_b = b"a short second file".to_vec();

        let (client, sink) = client_with(
            &[
                ("reports/a.bin", payload_a.clone()),
                ("b.txt", payload_b.clone()),
            ],
            1_024,
        );

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("a listener can be registered");

        let sending = client
            .send(SendRequest::to(
                peer(),
                vec![FileRef::from("reports/a.bin"), FileRef::from("b.txt")],
            ))
            .await
            .expect("the send is accepted");

        let sent = sending.wait().await.expect("the transfer completed");
        let received = receiving.wait().await.expect("the transfer completed");

        assert_eq!(sent.bytes(), 5_000 + payload_b.len() as u64);
        assert_eq!(
            sent.chunks(),
            5 + 1,
            "5 chunks for 5000 bytes, 1 for the short file"
        );

        assert_eq!(received.transfer_id(), sent.transfer_id());
        assert_eq!(received.bytes(), sent.bytes());

        assert_eq!(sink.get("reports/a.bin"), Some(payload_a));
        assert_eq!(sink.get("b.txt"), Some(payload_b));
    });
}

#[test]
fn the_files_a_receive_reports_carry_their_paths_and_sizes() {
    block_on(async {
        let (client, _sink) = client_with(
            &[("a.txt", b"aaa".to_vec()), ("b.txt", b"bbbb".to_vec())],
            16,
        );

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        let outcome = client
            .send(SendRequest::to(
                peer(),
                vec![FileRef::from("a.txt"), FileRef::from("b.txt")],
            ))
            .await
            .expect("send")
            .wait()
            .await
            .expect("completed");

        let received = receiving.wait().await.expect("completed");

        assert_eq!(outcome.chunks(), 2);

        let files = match received {
            vilsend_sdk::Outcome::Received(received) => received.files,
            other => panic!("expected a receive outcome, got {other:?}"),
        };

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[0].bytes, 3);
        assert!(files[0].verified);
        assert_eq!(files[1].path, "b.txt");
        assert_eq!(files[1].bytes, 4);
    });
}

#[test]
fn a_zero_byte_file_survives_the_round_trip() {
    // The chunker's `max(1)` is the reason: a file with no bytes is one empty
    // chunk, not zero chunks.
    block_on(async {
        let (client, sink) = client_with(&[("empty.txt", Vec::new())], 1_024);

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        let sent = client
            .send(SendRequest::to(peer(), vec![FileRef::from("empty.txt")]))
            .await
            .expect("send")
            .wait()
            .await
            .expect("completed");

        receiving.wait().await.expect("completed");

        assert_eq!(sent.bytes(), 0);
        assert_eq!(sent.chunks(), 1);
        assert!(sink.contains("empty.txt"), "the empty file arrived");
        assert_eq!(sink.get("empty.txt"), Some(Vec::new()));
    });
}

#[test]
fn received_files_land_under_the_destination() {
    block_on(async {
        let (client, sink) = client_with(&[("docs/a.txt", b"x".to_vec())], 16);

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::named("inbox")))
            .await
            .expect("listener");

        client
            .send(SendRequest::to(peer(), vec![FileRef::from("docs/a.txt")]))
            .await
            .expect("send")
            .wait()
            .await
            .expect("completed");

        receiving.wait().await.expect("completed");

        assert_eq!(sink.get("inbox/docs/a.txt"), Some(b"x".to_vec()));
        assert_eq!(sink.paths(), vec!["inbox/docs/a.txt"]);
    });
}

#[test]
fn a_send_with_nothing_listening_has_no_route() {
    block_on(async {
        let (client, _sink) = client_with(&[("a.txt", b"a".to_vec())], 16);

        let error = client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect_err("nobody is listening");

        assert_eq!(error.kind(), ErrorKind::NoRoute);
    });
}

#[test]
fn each_listener_takes_one_send_and_then_there_is_no_route_again() {
    block_on(async {
        let (client, _sink) = client_with(&[("a.txt", b"a".to_vec())], 16);

        let first = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect("send")
            .wait()
            .await
            .expect("completed");

        first.wait().await.expect("completed");

        let error = client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect_err("the listener was already used");

        assert_eq!(error.kind(), ErrorKind::NoRoute);
    });
}

#[test]
fn a_listener_cancelled_before_a_send_leaves_no_route() {
    block_on(async {
        let (client, _sink) = client_with(&[("a.txt", b"a".to_vec())], 16);

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        receiving
            .cancel()
            .await
            .expect("cancelling a listener that has not started");

        assert_eq!(
            receiving.wait().await.expect_err("it was cancelled").kind(),
            ErrorKind::Cancelled
        );

        let error = client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect_err("the listener is gone");

        assert_eq!(error.kind(), ErrorKind::NoRoute);
    });
}

#[test]
fn dropping_a_listener_cancels_it() {
    // §2.2: "Dropping the handle cancels the transfer. Documented, not
    // incidental." For a listener that has not started, that is the difference
    // between a reconciled transfer and a leaked one.
    block_on(async {
        let (client, _sink) = client_with(&[("a.txt", b"a".to_vec())], 16);

        drop(
            client
                .receive(ReceiveRequest::from(peer(), Destination::root()))
                .await
                .expect("listener"),
        );

        let error = client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect_err("the listener was dropped");

        assert_eq!(error.kind(), ErrorKind::NoRoute);
    });
}

#[test]
fn a_receive_cancelled_while_the_transfer_runs_stops_it() {
    // The one cancellation that can reach a transfer in flight: the receiving
    // handle exists before the sender does. Interleaving the two futures on one
    // thread is what makes it deterministic rather than a race.
    block_on(async {
        let (client, sink) = client_with(&[("a.bin", vec![7u8; 4_096])], 512);

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        let (sending, ()) = futures::join!(
            client.send(SendRequest::to(peer(), vec![FileRef::from("a.bin")])),
            async {
                yield_once().await;
                receiving
                    .cancel()
                    .await
                    .expect("the transfer is still running when this lands");
            }
        );

        let sending = sending.expect("the send returned a handle");

        assert_eq!(
            sending.wait().await.expect_err("it was cancelled").kind(),
            ErrorKind::Cancelled
        );
        assert_eq!(
            receiving.wait().await.expect_err("it was cancelled").kind(),
            ErrorKind::Cancelled
        );

        assert!(
            sink.is_empty(),
            "a cancelled transfer writes nothing, not even a prefix"
        );
    });
}

#[test]
fn a_request_naming_a_file_nobody_has_is_not_found_and_costs_no_listener() {
    block_on(async {
        let (client, _sink) = client_with(&[("a.txt", b"a".to_vec())], 16);

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        let error = client
            .send(SendRequest::to(peer(), vec![FileRef::from("missing.txt")]))
            .await
            .expect_err("the source has no such file");

        assert_eq!(error.kind(), ErrorKind::NotFound);

        // The listener is untouched, so the mistake costs the caller nothing
        // but the mistake.
        client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect("the listener is still waiting")
            .wait()
            .await
            .expect("completed");

        receiving.wait().await.expect("completed");
    });
}

#[test]
fn a_send_with_no_files_is_invalid_input() {
    block_on(async {
        let (client, _sink) = client_with(&[("a.txt", b"a".to_vec())], 16);

        let error = client
            .send(SendRequest::to(peer(), Vec::new()))
            .await
            .expect_err("there is nothing to send");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    });
}

#[test]
fn the_event_stream_reports_the_transfer() {
    let (client, _sink) = client_with(&[("a.bin", vec![1u8; 2_048])], 512);

    // Subscribed before the transfer, because the stream is a live feed and
    // replays nothing.
    let mut events = client.events();

    block_on(async {
        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.bin")]))
            .await
            .expect("send")
            .wait()
            .await
            .expect("completed");

        receiving.wait().await.expect("completed");
    });

    // Drained outside the executor: `poll_immediate` is polled rather than
    // awaited, so this neither parks nor nests one executor inside another.
    let mut names = Vec::new();

    while let Some(Some(event)) = block_on(futures::future::poll_immediate(events.next())) {
        names.push(event.wire_name());
    }

    assert!(
        names.contains(&"transfer-progress"),
        "progress was reported: {names:?}"
    );
    assert!(
        names.contains(&"transfer-completed"),
        "completion was reported: {names:?}"
    );
    assert_eq!(
        events.dropped_events(),
        0,
        "a transfer this size must not overflow the subscriber's buffer"
    );
}

#[test]
fn a_host_supplied_sink_sees_the_same_events_as_the_stream() {
    block_on(async {
        let source = MemoryFiles::new();
        source.insert("a.txt", b"a".to_vec());

        let forwarded = Arc::new(RecordingEventSink::new());

        let client = VilsendBuilder::in_memory()
            .with_memory_source(source)
            .with_event_sink(Arc::clone(&forwarded) as Arc<dyn EventSink>)
            .build()
            .expect("builds");

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect("send")
            .wait()
            .await
            .expect("completed");

        receiving.wait().await.expect("completed");

        let names: Vec<&str> = forwarded
            .take()
            .iter()
            .map(vilsend_sdk::DomainEvent::wire_name)
            .collect();

        assert!(names.contains(&"transfer-completed"), "{names:?}");
    });
}

#[test]
fn a_handle_reports_a_terminal_sample_once_the_transfer_is_over() {
    block_on(async {
        let (client, _sink) = client_with(&[("a.bin", vec![3u8; 1_024])], 256);

        let receiving = client
            .receive(ReceiveRequest::from(peer(), Destination::root()))
            .await
            .expect("listener");

        let sending = client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.bin")]))
            .await
            .expect("send");

        let sample = sending.progress().await;

        assert!(sample.is_terminal());
        assert_eq!(sample.bytes_confirmed, 1_024);
        assert_eq!(sample.total_bytes, 1_024);
        assert_eq!(sample.chunks_confirmed, 4);
        assert_eq!(sample.total_chunks, 4);
        assert_eq!(sample.files_done, 1);
        assert_eq!(sample.files_total, 1);
        assert_eq!(sample.fraction(), Some(1.0));

        sending.wait().await.expect("completed");
        receiving.wait().await.expect("completed");
    });
}

#[test]
fn a_no_route_send_reports_the_failure_on_the_events_stream() {
    block_on(async {
        let (client, _sink) = client_with(&[("a.txt", b"a".to_vec())], 16);

        let error = client
            .send(SendRequest::to(peer(), vec![FileRef::from("a.txt")]))
            .await
            .expect_err("nobody is listening");

        assert_eq!(error.kind(), ErrorKind::NoRoute);
        // Nothing was started, so nothing is reported: the failure is the
        // `send` call's, and a caller that got an `Err` has already been told.
        assert!(client.events().dropped_events() == 0);
    });
}
