//! The event verb: a `Stream`, not a callback.
//!
//! `04-sdk-cli-mobile-build-plan.md` §2.3 gives the reason in one sentence — "a
//! stream composes (`select!`, `merge`, `throttle`), applies backpressure
//! naturally, and maps cleanly onto every binding target: Node
//! `AsyncIterator`, Swift `AsyncSequence`, Kotlin `Flow`, Python async
//! generator. A callback API forces every binding to re-implement backpressure
//! and re-entrancy protection."
//!
//! The `Item` is [`DomainEvent`] from `vilsend-core`, which is the same enum
//! the desktop shell's `EventSink` carries (ADR-0012). There is one event
//! vocabulary in the product and the SDK does not get a second one.

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_channel::mpsc;
use futures_core::Stream;

use vilsend_core::{DomainEvent, EventSink};

/// How many events one subscriber may fall behind before it starts losing
/// them.
///
/// Bounded on purpose. Phase 4 task 4.9 records "unbounded queues are an OOM
/// reachable from a LAN peer", and a fan-out with an unbounded per-subscriber
/// queue would put the same reachable OOM in the SDK. A subscriber that is
/// slower than the transfer loses events and can see how many via
/// [`EventStream::dropped_events`] — which is a visible, countable loss rather
/// than a silent one.
pub(crate) const EVENT_BUFFER: usize = 1024;

/// A subscriber's queue, plus the count of what it missed.
struct Subscriber {
    sender: mpsc::Sender<DomainEvent>,
    dropped: Arc<AtomicU64>,
}

/// Where the engine publishes events, and where subscribers come from.
///
/// Implements [`EventSink`] so that anything in the engine that already knows
/// how to report through the port needs no new vocabulary to report through
/// the SDK.
#[derive(Clone, Default)]
pub(crate) struct EventBus {
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
    /// A host-supplied sink, if the builder was given one.
    forward: Option<Arc<dyn EventSink>>,
}

impl EventBus {
    pub(crate) fn new(forward: Option<Arc<dyn EventSink>>) -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(Vec::new())),
            forward,
        }
    }

    /// Registers a new subscriber and returns the stream it reads from.
    ///
    /// Every call returns an independent stream: two subscribers both see
    /// every event emitted after they subscribed. Events emitted before a
    /// subscription are not replayed — the stream is a live feed, not a log,
    /// and a log would have to be bounded to be safe.
    pub(crate) fn subscribe(&self) -> EventStream {
        let (sender, receiver) = mpsc::channel(EVENT_BUFFER);
        let dropped = Arc::new(AtomicU64::new(0));

        self.subscribers
            .lock()
            .expect("event subscriber lock poisoned")
            .push(Subscriber {
                sender,
                dropped: Arc::clone(&dropped),
            });

        EventStream { receiver, dropped }
    }

    /// How many subscribers are currently attached.
    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .expect("event subscriber lock poisoned")
            .len()
    }
}

impl EventSink for EventBus {
    fn emit(&self, event: DomainEvent) {
        let mut subscribers = self
            .subscribers
            .lock()
            .expect("event subscriber lock poisoned");

        // A subscriber whose receiver has been dropped is removed rather than
        // written to forever: dropping an `EventStream` is how a host
        // unsubscribes.
        // `retain_mut`, because `try_send` needs the sender by `&mut`.
        subscribers.retain_mut(|subscriber| {
            match subscriber.sender.try_send(event.clone()) {
                Ok(()) => true,
                // `futures` 0.3's `TrySendError` is a struct rather than an
                // enum, and `is_disconnected` is the distinction that matters:
                // a full queue is a slow subscriber, a disconnected one is a
                // subscriber that has gone away.
                Err(error) if error.is_disconnected() => false,
                Err(_) => {
                    subscriber.dropped.fetch_add(1, Ordering::Relaxed);
                    true
                }
            }
        });

        if let Some(forward) = &self.forward {
            forward.emit(event);
        }
    }
}

/// A live feed of [`DomainEvent`]s.
///
/// `futures_core::Stream` rather than an `async fn next()`: a stream is what
/// the four binding targets in §2.3 all consume, and a stream is what
/// composes.
#[derive(Debug)]
pub struct EventStream {
    receiver: mpsc::Receiver<DomainEvent>,
    dropped: Arc<AtomicU64>,
}

impl EventStream {
    /// How many events this stream missed because it was not drained fast
    /// enough.
    ///
    /// Never silently non-zero: if this is above zero, the subscriber knows it
    /// is looking at an incomplete history and can say so.
    pub fn dropped_events(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Stream for EventStream {
    type Item = DomainEvent;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // `EventStream` is `Unpin` — it is a channel receiver and an `Arc` —
        // so getting a `&mut` out of the pin needs no unsafe.
        let this = self.get_mut();

        Pin::new(&mut this.receiver).poll_next(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::StreamExt;
    use vilsend_core::{RecordingEventSink, TransferProgress, TransferStatus};

    fn progress(id: &str) -> TransferProgress {
        TransferProgress {
            transfer_id: id.into(),
            uploaded_bytes: 1,
            total_bytes: 2,
            percentage: 50.0,
            speed: 1.0,
            eta: Some(1),
            status: TransferStatus::Uploading,
        }
    }

    fn event(id: &str) -> DomainEvent {
        DomainEvent::TransferProgress {
            progress: progress(id),
        }
    }

    #[test]
    fn a_subscriber_sees_what_is_emitted_after_it_subscribes() {
        let bus = EventBus::new(None);
        let mut stream = bus.subscribe();

        bus.emit(event("t-1"));
        bus.emit(event("t-2"));

        assert_eq!(
            block_on(stream.next())
                .expect("two events")
                .progress()
                .transfer_id,
            "t-1"
        );
        assert_eq!(
            block_on(stream.next())
                .expect("two events")
                .progress()
                .transfer_id,
            "t-2"
        );
        assert_eq!(stream.dropped_events(), 0);
    }

    #[test]
    fn two_subscribers_both_see_the_same_event() {
        let bus = EventBus::new(None);
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();

        assert_eq!(bus.subscriber_count(), 2);

        bus.emit(event("t-1"));

        assert!(block_on(first.next()).is_some());
        assert!(block_on(second.next()).is_some());
    }

    #[test]
    fn an_event_emitted_before_a_subscription_is_not_replayed() {
        let bus = EventBus::new(None);

        bus.emit(event("t-1"));

        let mut stream = bus.subscribe();

        bus.emit(event("t-2"));

        let seen = block_on(stream.next()).expect("one event");

        assert_eq!(seen.progress().transfer_id, "t-2");
    }

    #[test]
    fn dropping_a_stream_unsubscribes_it() {
        let bus = EventBus::new(None);

        let stream = bus.subscribe();

        assert_eq!(bus.subscriber_count(), 1);

        drop(stream);
        bus.emit(event("t-1"));

        assert_eq!(bus.subscriber_count(), 0, "the dropped stream was reaped");
    }

    #[test]
    fn a_subscriber_that_never_drains_loses_the_overflow_and_can_see_that_it_did() {
        // The queue is bounded (Phase 4 task 4.9). A burst larger than the
        // buffer must not grow it and must not be silently forgotten.
        //
        // The invariant asserted is the one that matters and the one that does
        // not encode the channel's exact capacity: every event emitted is
        // either delivered or counted as dropped. `poll_immediate` is what
        // makes it checkable — `next().await` would park once the queue
        // drained, because the bus still holds a sender.
        let bus = EventBus::new(None);
        let mut stream = bus.subscribe();

        let emitted = EVENT_BUFFER + 5;

        for index in 0..emitted {
            bus.emit(event(&format!("t-{index}")));
        }

        let dropped = stream.dropped_events();

        assert!(
            dropped > 0,
            "{emitted} events into a buffer of {EVENT_BUFFER} must overflow"
        );

        let mut delivered = 0u64;

        while let Some(Some(_)) = block_on(futures::future::poll_immediate(stream.next())) {
            delivered += 1;
        }

        assert_eq!(
            delivered + dropped,
            emitted as u64,
            "nothing may be lost without being counted"
        );
    }

    #[test]
    fn a_host_supplied_sink_is_forwarded_to_as_well() {
        // `VilsendBuilder::with_event_sink` is how a shell that wants push
        // rather than pull keeps working; the stream and the sink are not
        // alternatives.
        let forwarded = Arc::new(RecordingEventSink::new());
        let bus = EventBus::new(Some(Arc::clone(&forwarded) as Arc<dyn EventSink>));
        let mut stream = bus.subscribe();

        bus.emit(event("t-1"));

        assert_eq!(forwarded.len(), 1);
        assert!(block_on(stream.next()).is_some());
    }

    #[test]
    fn the_event_bus_is_a_send_sync_event_sink() {
        // `EventSink` requires `Send + Sync + 'static`; the engine calls it
        // from wherever the work happens.
        fn assert_sink<T: EventSink>() {}

        assert_sink::<EventBus>();
    }
}
