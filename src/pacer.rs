//! Uniform-rate pacing for the transmit loop.
//!
//! Slice 2 wants 10,000 packets per second *at a uniform rate* — not 10,000
//! packets somewhere inside a second. That is a 100 µs budget per message, and
//! two obvious implementations both get it wrong:
//!
//! - `thread::sleep(100µs)` in a loop **accumulates drift**. Every oversleep is
//!   permanent; the loop never catches up, and after 100,000 messages you are
//!   seconds behind with no record of it.
//! - A pure spin loop is accurate and burns a core solid.
//!
//! So: deadlines are absolute — the *n*-th message is due at `origin + n·interval`,
//! computed from the origin every time, so an overshoot on one message is
//! absorbed by the next rather than pushed into it. And the wait is a hybrid:
//! sleep until [`SPIN_MARGIN`] before the deadline, then spin. The sleep gives
//! the CPU back for most of the gap; the spin covers the part where the OS
//! scheduler's granularity (~1 ms worst case on a loaded machine) is larger than
//! the interval we are trying to hit.
//!
//! Lateness is measured, not assumed. If the machine cannot hold the rate, the
//! stats say so instead of the run quietly producing 6,000 packets per second.

use std::time::{Duration, Instant};

/// How long before the deadline to stop sleeping and start spinning.
///
/// Below this, `thread::sleep` cannot be trusted to return on time — its
/// resolution is a scheduler tick, not a nanosecond. Above it, spinning is pure
/// waste. 60 µs is a compromise sized for a 100 µs interval.
pub const SPIN_MARGIN: Duration = Duration::from_micros(60);

/// What the pacer actually achieved, as opposed to what it was asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaceStats {
    /// Deadlines waited on.
    pub waits: u64,
    /// Deadlines already in the past when we got to them.
    pub late: u64,
    /// Worst single overshoot.
    pub max_lateness: Duration,
    /// Summed overshoot, for a mean.
    total_lateness_nanos: u128,
}

impl PaceStats {
    pub fn mean_lateness(&self) -> Duration {
        if self.waits == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos((self.total_lateness_nanos / self.waits as u128) as u64)
        }
    }

    /// Fraction of deadlines missed, in `[0, 1]`.
    pub fn late_fraction(&self) -> f64 {
        if self.waits == 0 {
            0.0
        } else {
            self.late as f64 / self.waits as f64
        }
    }
}

/// A uniform-rate clock. Construct it, then call [`Pacer::wait`] with a
/// monotonically increasing index.
#[derive(Debug, Clone)]
pub struct Pacer {
    origin: Instant,
    interval: Duration,
    spin_margin: Duration,
    stats: PaceStats,
}

impl Pacer {
    /// A pacer for `rate_hz` events per second. A rate of 0 means "as fast as
    /// possible" — [`Pacer::wait`] then returns immediately, always.
    pub fn new(rate_hz: u64) -> Self {
        let interval = if rate_hz == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(1_000_000_000 / rate_hz)
        };
        Pacer {
            origin: Instant::now(),
            interval,
            spin_margin: SPIN_MARGIN,
            stats: PaceStats::default(),
        }
    }

    pub fn with_spin_margin(mut self, margin: Duration) -> Self {
        self.spin_margin = margin;
        self
    }

    /// Resets the origin to now. Call this immediately before the send loop, so
    /// setup work (reading a CSV, encoding 100,000 messages) is not counted
    /// against the first deadlines.
    pub fn start(&mut self) {
        self.origin = Instant::now();
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn origin(&self) -> Instant {
        self.origin
    }

    pub fn stats(&self) -> PaceStats {
        self.stats
    }

    pub fn elapsed(&self) -> Duration {
        self.origin.elapsed()
    }

    /// The instant message `index` is due.
    pub fn deadline(&self, index: u64) -> Instant {
        // u128 so a long run at a fine interval cannot wrap the nanosecond
        // count; saturating so an absurd index degrades to "far future" rather
        // than panicking.
        let offset = self.interval.as_nanos().saturating_mul(index as u128);
        self.origin + Duration::from_nanos(offset.min(u64::MAX as u128) as u64)
    }

    /// Blocks until message `index` is due. Returns how late we were — zero if
    /// the deadline had not yet passed when the wait completed.
    pub fn wait(&mut self, index: u64) -> Duration {
        if self.interval.is_zero() {
            return Duration::ZERO;
        }
        let deadline = self.deadline(index);
        let now = Instant::now();

        if now < deadline {
            let remaining = deadline - now;
            if remaining > self.spin_margin {
                std::thread::sleep(remaining - self.spin_margin);
            }
            while Instant::now() < deadline {
                std::hint::spin_loop();
            }
        }

        let lateness = Instant::now().saturating_duration_since(deadline);
        self.stats.waits += 1;
        self.stats.total_lateness_nanos += lateness.as_nanos();
        if lateness > Duration::ZERO {
            self.stats.late += 1;
            if lateness > self.stats.max_lateness {
                self.stats.max_lateness = lateness;
            }
        }
        lateness
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_is_the_reciprocal_of_the_rate() {
        assert_eq!(Pacer::new(10_000).interval(), Duration::from_micros(100));
        assert_eq!(Pacer::new(1_000).interval(), Duration::from_millis(1));
        assert_eq!(Pacer::new(1).interval(), Duration::from_secs(1));
        assert_eq!(Pacer::new(0).interval(), Duration::ZERO);
    }

    /// Deadlines are absolute multiples of the interval from a fixed origin.
    /// This is the property that stops drift accumulating.
    #[test]
    fn deadlines_are_absolute_not_incremental() {
        let p = Pacer::new(10_000);
        let origin = p.origin();
        for n in [0u64, 1, 1_000, 100_000] {
            assert_eq!(p.deadline(n), origin + Duration::from_micros(100 * n));
        }
    }

    #[test]
    fn a_huge_index_saturates_instead_of_wrapping() {
        let p = Pacer::new(10_000);
        // 100 µs × u64::MAX overflows u64 nanoseconds by a wide margin.
        assert!(p.deadline(u64::MAX) > p.origin() + Duration::from_secs(1));
    }

    #[test]
    fn rate_zero_never_blocks() {
        let mut p = Pacer::new(0);
        let start = Instant::now();
        for i in 0..10_000 {
            assert_eq!(p.wait(i), Duration::ZERO);
        }
        assert!(start.elapsed() < Duration::from_millis(200));
    }

    /// The real test: does it actually hold a rate? Kept short and with a loose
    /// upper bound, because a test machine under load is not a trading box.
    #[test]
    fn holds_the_requested_rate() {
        let mut p = Pacer::new(2_000); // 500 µs apart
        p.start();
        let start = Instant::now();
        for i in 0..40 {
            p.wait(i);
        }
        let elapsed = start.elapsed();
        // 40 ticks at 500 µs = 19.5 ms to the last deadline.
        assert!(
            elapsed >= Duration::from_micros(19_000),
            "finished early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(120),
            "way over budget: {elapsed:?}"
        );
        assert_eq!(p.stats().waits, 40);
    }

    /// A stall must not shift later deadlines — the schedule catches up rather
    /// than sliding. This is the difference between absolute and incremental.
    #[test]
    fn a_stall_does_not_push_the_schedule_back() {
        let mut p = Pacer::new(1_000); // 1 ms apart
        p.start();
        p.wait(0);
        std::thread::sleep(Duration::from_millis(12));
        // Deadlines 1..=11 are all in the past now; each returns at once and is
        // recorded as late, instead of adding 11 ms to the run.
        let resume = Instant::now();
        for i in 1..=11 {
            p.wait(i);
        }
        assert!(
            resume.elapsed() < Duration::from_millis(5),
            "the stall was not absorbed"
        );
        assert!(
            p.stats().late >= 11,
            "missed deadlines must be counted, got {}",
            p.stats().late
        );
        assert!(p.stats().max_lateness >= Duration::from_millis(1));
    }

    #[test]
    fn stats_are_empty_before_any_wait() {
        let p = Pacer::new(10_000);
        assert_eq!(p.stats().waits, 0);
        assert_eq!(p.stats().mean_lateness(), Duration::ZERO);
        assert_eq!(p.stats().late_fraction(), 0.0);
    }
}
