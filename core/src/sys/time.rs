//! High-precision timestamp generation for DoLogger.
//!
//! # Implementation
//!
//! Uses `std::time::SystemTime` for basic wall-clock timestamps.
//!
//! # Planned Enhancements
//!
//! - TSC (Time Stamp Counter) via `rdtsc` for sub-nanosecond resolution
//! - VDSO clock_gettime on Linux (no syscall overhead)
//! - QueryPerformanceCounter on Windows
//! - mach_absolute_time on macOS
//! - NTP-calibrated monotonic clock mixing

use crate::ffi::dologger_uint128_t;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// High-precision timestamp source.
///
/// Generates nanosecond-resolution UTC timestamps and monotonic
/// sequence numbers for record IDs.
pub struct TimeSource {
    /// Monotonically increasing sequence counter (for record ID generation)
    sequence: AtomicU64,
    /// Node fingerprint (machine + process identifier combined)
    node_fingerprint: u64,
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
        }
    }

    /// Get the current UTC timestamp as a 128-bit value.
    ///
    /// The high 64 bits contain seconds since Unix epoch.
    /// The low 64 bits contain nanoseconds within the second.
    pub fn now_utc(&self) -> dologger_uint128_t {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        dologger_uint128_t {
            hi: now.as_secs(),
            lo: now.subsec_nanos() as u64,
        }
    }

    /// Get the current time as milliseconds since Unix epoch.
    pub fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Generate a globally unique record ID using a modified snowflake algorithm.
    ///
    /// Layout:
    /// - hi: Timestamp in milliseconds (42 bits effective, stored in upper bits)
    /// - lo: 32-bit node fingerprint + 22-bit sequence number + reserved bits
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
                // Sequence overflow — block until next millisecond
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
}
