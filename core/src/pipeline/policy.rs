//! PolicyProvider — pre-filter policies for rate limiting, sampling, etc.
//!
//! # Implementation
//!
//! - Rate limiter using a token bucket algorithm
//! - Configurable rate limit and burst size
//! - Thread-safe atomic implementation
//!
//! # Planned Extensions
//!
//! - Deterministic sampler (hash-based)
//! - Multi-domain rate limiting
//! - Dynamic policy updates via control plane

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::record::LogLevel;

/// Rate limiter using token bucket algorithm.
///
/// Thread-safe: uses atomic operations for the token counter.
pub struct RateLimiter {
    /// Maximum tokens per second
    rate_per_sec: u64,
    /// Maximum burst size (tokens that can be accumulated)
    burst_size: u64,
    /// Current token count (in units of 1/1000 token for precision)
    tokens: AtomicU64,
    /// Last refill timestamp
    last_refill: std::sync::Mutex<Instant>,
    /// Refill interval
    refill_interval: Duration,
    /// Tokens added per refill
    tokens_per_refill: u64,
    /// Total records allowed
    allowed_count: AtomicU64,
    /// Total records dropped
    dropped_count: AtomicU64,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// * `rate_per_sec` — maximum tokens per second (0 = disabled)
    /// * `burst_size` — maximum burst capacity
    pub fn new(rate_per_sec: u64, burst_size: u64) -> Self {
        let tokens_per_refill = if rate_per_sec > 0 {
            // Refill every 100ms → tokens per refill = rate / 10
            (rate_per_sec / 10).max(1)
        } else {
            0
        };

        Self {
            rate_per_sec,
            burst_size,
            tokens: AtomicU64::new(burst_size.saturating_mul(1000)), // Store as millitokens
            last_refill: std::sync::Mutex::new(Instant::now()),
            refill_interval: Duration::from_millis(100),
            tokens_per_refill,
            allowed_count: AtomicU64::new(0),
            dropped_count: AtomicU64::new(0),
        }
    }

    /// Evaluate whether a record should be allowed through.
    ///
    /// Returns `true` if the record is allowed, `false` if it should be dropped.
    pub fn evaluate(&self) -> bool {
        if self.rate_per_sec == 0 {
            // Rate limiting disabled — allow everything
            self.allowed_count.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        // Refill tokens if enough time has passed
        self.refill();

        // Try to consume a token
        loop {
            let current = self.tokens.load(Ordering::Acquire);
            if current < 1000 {
                // Less than 1 token — drop the record
                self.dropped_count.fetch_add(1, Ordering::Relaxed);
                return false;
            }

            match self.tokens.compare_exchange_weak(
                current,
                current - 1000, // Consume 1 token (1000 millitokens)
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.allowed_count.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
                Err(_) => {
                    // CAS failed — another thread consumed tokens, retry
                    continue;
                }
            }
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill(&self) {
        let mut last = self
            .last_refill
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        let elapsed = now.duration_since(*last);

        if elapsed >= self.refill_interval {
            let refills = (elapsed.as_millis() / self.refill_interval.as_millis()) as u64;
            let add = refills
                .saturating_mul(self.tokens_per_refill)
                .saturating_mul(1000); // Convert to millitokens
            let max_tokens = self.burst_size.saturating_mul(1000);

            loop {
                let current = self.tokens.load(Ordering::Acquire);
                let new = current.saturating_add(add).min(max_tokens);
                match self.tokens.compare_exchange_weak(
                    current,
                    new,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(_) => continue,
                }
            }

            *last = now;
        }
    }

    /// Get the current token count (for monitoring).
    pub fn available_tokens(&self) -> u64 {
        self.tokens.load(Ordering::Relaxed) / 1000
    }

    /// Total records allowed since creation.
    pub fn allowed_count(&self) -> u64 {
        self.allowed_count.load(Ordering::Relaxed)
    }

    /// Total records dropped since creation.
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(0, 0) // Disabled by default
    }
}

// ===========================================================================
// Drop-level policy — static rule to drop records by level
// ===========================================================================

/// Runtime policy: drop records below a configured minimum level.
///
/// The level check MUST be inlineable. This is called
/// from the pipeline; will hoist it into the `dologger_log` hot path.
pub struct DropLevelPolicy {
    /// Minimum level to allow through (records below this are dropped).
    ///
    /// The atomic representation keeps `/level` off the producer and
    /// consumer coordination locks. Only the control-plane write is rare;
    /// evaluation remains one acquire load plus the existing counter update.
    min_level: std::sync::atomic::AtomicU8,
    /// Total records allowed
    allowed: AtomicU64,
    /// Total records dropped
    dropped: AtomicU64,
}

impl DropLevelPolicy {
    /// Create a policy that drops records below `min_level`.
    pub fn new(min_level: LogLevel) -> Self {
        Self {
            min_level: std::sync::atomic::AtomicU8::new(min_level as u8),
            allowed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Evaluate: return true if the record should be kept.
    pub fn evaluate(&self, level: LogLevel) -> bool {
        let minimum = LogLevel::from_u8(self.min_level.load(Ordering::Acquire))
            // A corrupted or invalid atomic value must fail closed for
            // ordinary records rather than accidentally enabling all logs.
            .unwrap_or(LogLevel::Audit);
        if level >= minimum {
            self.allowed.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    /// Return the current minimum level.
    pub fn min_level(&self) -> LogLevel {
        LogLevel::from_u8(self.min_level.load(Ordering::Acquire)).unwrap_or(LogLevel::Audit)
    }

    /// Replace the minimum level and return the previous value.
    pub fn set_min_level(&self, min_level: LogLevel) -> LogLevel {
        let previous = self.min_level.swap(min_level as u8, Ordering::AcqRel);
        LogLevel::from_u8(previous).unwrap_or(LogLevel::Audit)
    }

    /// Total records allowed.
    pub fn allowed_count(&self) -> u64 {
        self.allowed.load(Ordering::Relaxed)
    }

    /// Total records dropped.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_limiter_allows_all() {
        let limiter = RateLimiter::new(0, 0);
        for _ in 0..1000 {
            assert!(limiter.evaluate());
        }
        assert_eq!(limiter.allowed_count(), 1000);
        assert_eq!(limiter.dropped_count(), 0);
    }

    #[test]
    fn test_drop_level_policy() {
        let policy = DropLevelPolicy::new(LogLevel::Warn);
        assert!(policy.evaluate(LogLevel::Warn));
        assert!(policy.evaluate(LogLevel::Error));
        assert!(!policy.evaluate(LogLevel::Info));
        assert!(!policy.evaluate(LogLevel::Debug));
        assert_eq!(policy.allowed_count(), 2);
        assert_eq!(policy.dropped_count(), 2);
    }

    #[test]
    fn test_drop_level_policy_updates_without_replacement() {
        let policy = DropLevelPolicy::new(LogLevel::Warn);
        assert!(!policy.evaluate(LogLevel::Info));
        assert_eq!(policy.min_level(), LogLevel::Warn);

        let previous = policy.set_min_level(LogLevel::Info);
        assert_eq!(previous, LogLevel::Warn);
        assert_eq!(policy.min_level(), LogLevel::Info);
        assert!(policy.evaluate(LogLevel::Info));
    }

    #[test]
    fn test_drop_level_policy_invalid_atomic_value_fails_closed() {
        let policy = DropLevelPolicy::new(LogLevel::Trace);
        policy.min_level.store(0xff, Ordering::Release);
        assert_eq!(policy.min_level(), LogLevel::Audit);
        assert!(!policy.evaluate(LogLevel::Info));
    }

    #[test]
    fn test_rate_limiter_drops_excess() {
        // Allow only 100/sec, burst 10
        let limiter = RateLimiter::new(100, 10);

        let mut allowed = 0;
        let mut dropped = 0;
        for _ in 0..50 {
            if limiter.evaluate() {
                allowed += 1;
            } else {
                dropped += 1;
            }
        }

        // After consuming the burst (10 tokens), excess should be dropped
        // until the refill interval passes
        assert!(allowed <= 50);
        assert_eq!(allowed + dropped, 50);
        assert!(
            dropped > 0,
            "Some records should be dropped after burst exhausted"
        );
    }

    #[test]
    fn test_rate_limiter_saturates_extreme_capacity_without_overflow() {
        let limiter = RateLimiter::new(u64::MAX, u64::MAX);
        assert_eq!(limiter.available_tokens(), u64::MAX / 1000);
        assert!(limiter.evaluate());
    }
}
