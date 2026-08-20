//! High-precision timestamp generation for DoLogger.
//!
//! Two clocks are provided:
//!
//! - [`MonotonicClock`] — a cross-platform monotonic clock backed by
//!   `std::time::Instant`. Guaranteed to never go backwards (QPC on Windows,
//!   `CLOCK_MONOTONIC` on Linux, `mach_absolute_time` on macOS), meaningful
//!   only within a single process. Used for ordering and elapsed-time
//!   measurement.
//! - [`TimeSource`] — a hybrid source mixing the wall clock with the
//!   monotonic clock. Record IDs and millisecond timestamps are driven by
//!   monotonic-elapsed time anchored at construction, so a later wall-clock
//!   step (NTP correction, manual `date` change) can never make them regress.
//!   True wall-clock UTC remains available via [`TimeSource::now_utc`].
//!
//! # Implementation
//!
//! The hybrid value is `wall_base + monotonic_elapsed`: the offset between the
//! two clocks is sampled once at construction, then the monotonic clock drives
//! the result. This is the standard hybrid-clock technique (cf. CockroachDB's
//! HLC) and makes the sequence/ID generators independent of wall-clock steps.
//!
//! # Planned Enhancements
//!
//! - TSC (Time Stamp Counter) via `rdtsc` for sub-nanosecond resolution
//! - VDSO clock_gettime on Linux (no syscall overhead)
//! - NTP error-bounded anchoring at construction

use crate::ffi::dologger_uint128_t;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A cross-platform monotonic clock.
///
/// All values are elapsed since the clock's construction instant, so they are
/// comparable within one process but carry no epoch meaning. Backed by
/// [`std::time::Instant`], which every supported platform guarantees to be
/// monotonic.
pub struct MonotonicClock {
    /// Instant at construction; every `now_ns()` value is elapsed since here.
    origin: Instant,
}

impl MonotonicClock {
    /// Create a new monotonic clock anchored at the current instant.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    /// Nanoseconds elapsed since this clock was created.
    ///
    /// Monotonic and non-decreasing across calls within the same process.
    pub fn now_ns(&self) -> u64 {
        self.elapsed().as_nanos() as u64
    }

    /// [`Duration`] elapsed since this clock was created.
    pub fn elapsed(&self) -> Duration {
        self.origin.elapsed()
    }

    /// Nanoseconds elapsed since a previously captured `now_ns()` value.
    ///
    /// `earlier` must be a value previously returned by [`MonotonicClock::now_ns`]
    /// on this clock. Clamped at zero if `earlier` is somehow in the future.
    pub fn elapsed_since(&self, earlier: u64) -> u64 {
        self.now_ns().saturating_sub(earlier)
    }

    /// The instant this clock was anchored at, for external `Instant` arithmetic.
    pub fn origin(&self) -> Instant {
        self.origin
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

/// High-precision timestamp source.
///
/// Generates nanosecond-resolution UTC timestamps, monotonic nanosecond
/// counters, and monotonic-corrected snowflake record IDs.
pub struct TimeSource {
    /// Monotonically increasing sequence counter (for record ID generation)
    sequence: AtomicU64,
    /// Node fingerprint (machine + process identifier combined)
    node_fingerprint: u64,
    /// Monotonic clock driving the hybrid `now_ms()` and sequence ordering
    monotonic: MonotonicClock,
    /// Wall-clock anchor sampled at construction, used by the hybrid `now_ms()`
    wall_base: SystemTime,
}

impl TimeSource {
    /// Create a new TimeSource with the given node fingerprint.
    pub fn new() -> Self {
        // Generate a node fingerprint from process ID + random component
        let pid = std::process::id() as u64;
        let random: u64 = {
            use std::hash::{Hash, Hasher};
            let mut s = std::collections::hash_map::DefaultHasher::new();
            SystemTime::now().hash(&mut s);
            std::process::id().hash(&mut s);
            s.finish()
        };

        Self {
            sequence: AtomicU64::new(0),
            node_fingerprint: (pid << 32) | (random & 0xFFFF_FFFF),
            monotonic: MonotonicClock::new(),
            wall_base: SystemTime::now(),
        }
    }

    /// Get the current UTC timestamp as a 128-bit value.
    ///
    /// The high 64 bits contain seconds since Unix epoch.
    /// The low 64 bits contain nanoseconds within the second.
    ///
    /// This is the *true* wall clock: it follows NTP corrections and manual
    /// changes. Use [`TimeSource::next_id`] for ordering guarantees, and
    /// [`TimeSource::now_monotonic_ns`] for duration measurement.
    pub fn now_utc(&self) -> dologger_uint128_t {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        dologger_uint128_t {
            hi: now.as_secs(),
            lo: now.subsec_nanos() as u64,
        }
    }

    /// Get the current UTC timestamp as nanoseconds since Unix epoch (u64).
    ///
    /// Returns `secs * 1_000_000_000 + nanos` as a single `u64`. This is the
    /// *true* wall clock (follows NTP corrections). Suitable for storing in
    /// `Record::timestamp` which uses a compact u64 layout.
    pub fn now_nanos(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        now.as_secs()
            .saturating_mul(1_000_000_000)
            .saturating_add(now.subsec_nanos() as u64)
    }

    /// Get the current time as milliseconds since Unix epoch.
    ///
    /// This is the *hybrid* clock: `wall_base + monotonic-elapsed`. It always
    /// matches the wall clock to within the precision of the wall-to-monotonic
    /// offset sampled at construction, and it never regresses when the wall
    /// clock is stepped. Used for snowflake ID generation and sequence ordering.
    pub fn now_ms(&self) -> u64 {
        let anchor = self
            .wall_base
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        (anchor + self.monotonic.elapsed()).as_millis() as u64
    }

    /// Monotonic nanoseconds since this TimeSource was created.
    ///
    /// Comparable only within this process. Use for ordering and elapsed-time
    /// measurement where wall-clock steps must not interfere.
    pub fn now_monotonic_ns(&self) -> u64 {
        self.monotonic.now_ns()
    }

    /// The underlying monotonic clock, for elapsed-time measurement.
    pub fn monotonic_clock(&self) -> &MonotonicClock {
        &self.monotonic
    }

    /// Generate a globally unique record ID using a modified snowflake algorithm.
    ///
    /// Layout:
    /// - hi: Timestamp in milliseconds (42 bits effective, stored in upper bits)
    /// - lo: 32-bit node fingerprint + 22-bit sequence number + reserved bits
    ///
    /// The millisecond component comes from the hybrid [`TimeSource::now_ms`],
    /// so IDs never regress even if the wall clock is stepped backwards.
    ///
    /// Sequence overflow: when the 22-bit sequence exceeds capacity
    /// (~4.19M/ms), blocks until next millisecond and logs sysmon WARN.
    pub fn next_id(&self) -> dologger_uint128_t {
        const MAX_SEQUENCE: u64 = 0x3F_FFFF; // 22-bit max = 4,194,303

        loop {
            let timestamp_ms = self.now_ms();
            let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
            let masked_seq = seq & MAX_SEQUENCE;

            // Check for sequence overflow within the same millisecond
            if masked_seq == 0 && seq > 0 {
                // Sequence overflow — block until next millisecond.
                // `now_ms()` is monotonic-driven, so this loop always terminates
                // even under wall-clock rollback.
                crate::sys::diagnostics::warn(
                    "time",
                    &format!("Snowflake sequence overflow at seq={seq} — blocking for next ms"),
                );
                let current_ms = timestamp_ms;
                while self.now_ms() <= current_ms {
                    std::hint::spin_loop();
                }
                // Retry with new timestamp
                continue;
            }

            let lo = (self.node_fingerprint << 32) | masked_seq;

            return dologger_uint128_t {
                hi: timestamp_ms,
                lo,
            };
        }
    }

    /// Get the node fingerprint for this process.
    pub fn node_fingerprint(&self) -> u64 {
        self.node_fingerprint
    }
}

impl Default for TimeSource {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_timestamp_monotonic() {
        let ts = TimeSource::new();
        let id1 = ts.next_id();
        let id2 = ts.next_id();
        let id3 = ts.next_id();

        // IDs should be monotonically increasing
        assert!(id2.lo > id1.lo || id2.hi > id1.hi);
        assert!(id3.lo > id2.lo || id3.hi > id2.hi);
    }

    #[test]
    fn test_now_utc_nonzero() {
        let ts = TimeSource::new();
        let now = ts.now_utc();
        // Should be > year 2026 (approx 1.7 billion seconds since epoch)
        assert!(now.hi > 1_700_000_000);
    }

    #[test]
    fn test_node_fingerprint() {
        let ts = TimeSource::new();
        assert_ne!(ts.node_fingerprint(), 0);
    }

    #[test]
    fn test_monotonic_clock_increases() {
        let clock = MonotonicClock::new();
        let n1 = clock.now_ns();
        std::thread::sleep(Duration::from_millis(5));
        let n2 = clock.now_ns();
        assert!(n2 > n1, "monotonic clock must advance: {n2} <= {n1}");
    }

    #[test]
    fn test_monotonic_clock_non_decreasing_rapid_calls() {
        let clock = MonotonicClock::new();
        let mut prev = clock.now_ns();
        for _ in 0..10_000 {
            let cur = clock.now_ns();
            assert!(cur >= prev, "monotonic clock regressed: {cur} < {prev}");
            prev = cur;
        }
    }

    #[test]
    fn test_monotonic_clock_elapsed_since() {
        let clock = MonotonicClock::new();
        let start = clock.now_ns();
        std::thread::sleep(Duration::from_millis(5));
        let elapsed = clock.elapsed_since(start);
        assert!(elapsed >= 5_000_000, "elapsed_since too small: {elapsed}");
    }

    #[test]
    fn test_hybrid_now_ms_monotonic() {
        let ts = TimeSource::new();
        let mut prev = ts.now_ms();
        // A few thousand calls across a sleep exercise the monotonic path and
        // confirm the hybrid value never regresses.
        for _ in 0..100 {
            std::thread::sleep(Duration::from_micros(50));
            let cur = ts.now_ms();
            assert!(cur >= prev, "hybrid now_ms regressed: {cur} < {prev}");
            prev = cur;
        }
    }

    #[test]
    fn test_hybrid_now_ms_aligns_with_wall_clock() {
        let ts = TimeSource::new();
        let hybrid = ts.now_ms();
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        // Construction happens moments before `wall` is read; the hybrid value
        // is anchored at construction, so it may be slightly behind but must be
        // within a generous tolerance.
        let delta = wall.saturating_sub(hybrid);
        assert!(
            delta < 5_000,
            "hybrid now_ms drifted from wall clock: {delta}ms"
        );
    }

    #[test]
    fn test_now_monotonic_ns_increases() {
        let ts = TimeSource::new();
        let n1 = ts.now_monotonic_ns();
        std::thread::sleep(Duration::from_millis(5));
        let n2 = ts.now_monotonic_ns();
        assert!(n2 > n1, "now_monotonic_ns must advance: {n2} <= {n1}");
    }

    #[test]
    fn test_monotonic_clock_default() {
        let clock = MonotonicClock::default();
        let _ = clock.now_ns();
    }
}
