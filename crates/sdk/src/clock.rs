//! The time source, and the one implementation the SDK ships.
//!
//! ADR-0011 §3 makes `VilsendBuilder::in_memory()` a contract rather than a
//! convenience: it "must touch **nothing external** — no filesystem, no
//! keychain, no network, **no real clock**." Throughput and ETA are computed
//! from a time source, so a backend that reads `Instant::now()` cannot honour
//! that contract, and the seam has to exist before `in_memory()` can be
//! trusted.
//!
//! **`vilsend-core` gains a `Clock` port in Phase 3** (task 3.2), "required for
//! deterministic retry/backoff tests". This is that port's shape, defined here
//! only because Phase 3 has not run: the SDK cannot be built against a port
//! that does not exist. When 3.2 lands, this trait should be *deleted* and the
//! core one re-exported, which is a one-line change at each use site — the
//! method is deliberately the smallest thing a clock can be.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A monotonic time source.
///
/// Nanoseconds since an arbitrary origin. The origin is arbitrary *by design*:
/// an implementation that returned wall-clock time would make throughput
/// depend on the system date, and a SDK that cannot be run deterministically is
/// a SDK whose tests are a matter of opinion.
pub trait Clock: Send + Sync + 'static {
    /// Nanoseconds since this clock's origin. Never decreases.
    fn now_nanos(&self) -> u64;
}

/// A [`Clock`] that only moves when a test moves it.
///
/// This is the default for [`crate::VilsendBuilder::in_memory`], and it starts
/// stopped: with no `advance`, every sample reports the same instant, so
/// `throughput_bps` is `0` and `eta` is `None` — deterministically, not
/// approximately.
#[derive(Debug, Default)]
pub struct FakeClock {
    nanos: AtomicU64,
}

impl FakeClock {
    /// A clock stopped at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward. Returns the new reading.
    ///
    /// `advance(Duration::ZERO)` is a no-op rather than an error — a caller
    /// that computed a zero delta has not made a mistake.
    pub fn advance(&self, by: Duration) -> u64 {
        self.nanos
            .fetch_add(by.as_nanos() as u64, Ordering::SeqCst)
            .saturating_add(by.as_nanos() as u64)
    }

    /// Sets the reading. Lowering it would break [`Clock`]'s monotonicity
    /// guarantee, so a lower value is ignored and the current reading is
    /// returned instead.
    pub fn set(&self, nanos: u64) -> u64 {
        self.nanos.fetch_max(nanos, Ordering::SeqCst).max(nanos)
    }
}

impl Clock for FakeClock {
    fn now_nanos(&self) -> u64 {
        self.nanos.load(Ordering::SeqCst)
    }
}

/// The throughput implied by `bytes` over the interval `from..to`, in bytes per
/// second.
///
/// Returns `0` rather than dividing by zero when no time has passed. A transfer
/// that has confirmed bytes but spent no measurable time on them has no
/// throughput; "infinitely fast" is arithmetic, not information.
pub(crate) fn throughput_bps(bytes: u64, from_nanos: u64, to_nanos: u64) -> u64 {
    let elapsed = to_nanos.saturating_sub(from_nanos);

    if elapsed == 0 {
        return 0;
    }

    let per_second = (bytes as u128) * 1_000_000_000u128 / (elapsed as u128);

    u64::try_from(per_second).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Clock` that is not `Send + Sync` cannot be injected, so the trait's
    /// bounds are part of its contract rather than a detail.
    #[test]
    fn a_fake_clock_can_be_shared_across_threads() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}

        assert_send_sync::<FakeClock>();
    }

    #[test]
    fn a_new_fake_clock_is_stopped_at_zero() {
        let clock = FakeClock::new();

        assert_eq!(clock.now_nanos(), 0);
        assert_eq!(clock.now_nanos(), 0, "reading it does not move it");
    }

    #[test]
    fn advancing_moves_the_clock_by_exactly_what_was_asked() {
        let clock = FakeClock::new();

        assert_eq!(clock.advance(Duration::from_millis(250)), 250_000_000);
        assert_eq!(clock.advance(Duration::from_secs(1)), 1_250_000_000);
        assert_eq!(clock.advance(Duration::ZERO), 1_250_000_000);
    }

    #[test]
    fn a_fake_clock_never_goes_backwards_even_when_asked_to() {
        let clock = FakeClock::new();

        clock.set(5_000);

        assert_eq!(clock.set(1), 5_000, "a lower value is refused");
        assert_eq!(clock.now_nanos(), 5_000);

        assert_eq!(clock.set(9_000), 9_000);
    }

    #[test]
    fn throughput_is_zero_when_no_time_passed() {
        assert_eq!(throughput_bps(1_000, 42, 42), 0);
        assert_eq!(throughput_bps(0, 0, 0), 0);
    }

    #[test]
    fn throughput_is_bytes_per_second() {
        // 1 KiB over one second.
        assert_eq!(throughput_bps(1_024, 0, 1_000_000_000), 1_024);

        // The same 1 KiB over half a second.
        assert_eq!(throughput_bps(1_024, 500_000_000, 1_000_000_000), 2_048);
    }

    #[test]
    fn throughput_saturates_rather_than_wrapping_on_an_absurd_rate() {
        // One nanosecond for u64::MAX bytes is more throughput than a u64 can
        // hold. Wrapping would report a small number; saturating reports the
        // truth, which is "at least this much".
        assert_eq!(throughput_bps(u64::MAX, 0, 1), u64::MAX);
    }

    #[test]
    fn a_clock_that_went_backwards_does_not_produce_a_negative_interval() {
        // `saturating_sub` rather than `-`: an injected clock may misbehave,
        // and the failure mode of trusting it would be a panic in a library.
        assert_eq!(throughput_bps(1_024, 900, 100), 0);
    }
}
