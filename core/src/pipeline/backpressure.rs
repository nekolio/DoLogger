//! Backpressure system.
//!
//! Implements the backpressure strategy table, binding
//! each `PerformanceProfile` to concrete runtime behavior:
//!
//! | Profile | block_timeout_ms | drop_strategy | AUDIT behavior |
//! |---------|-----------------|---------------|----------------|
//! | dev | 100 | newest | force 0 (infinite block) |
//! | prod-performance | 3000 | below_warn | force 0 |
//! | prod-audit | 3000 | below_warn | force 0 |
//! | balanced | 2000 | oldest | force 0 |
//!
//! Safety non-downgradable items:
//! - AUDIT domain `block_timeout_ms` MUST be 0
//! - AUDIT domain MUST NOT configure any `drop_strategy`
//! - Non-AUDIT `block_timeout_ms` MUST NOT be less than 100ms

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::PerformanceProfile;
use crate::record::LogLevel;

/// Drop strategy when ring buffer is full and timeout expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropStrategy {
    /// Drop the newest record (the one being submitted)
    Newest,
    /// Drop the oldest record in the buffer
    Oldest,
    /// Drop records below WARN level first
    BelowWarn,
    /// Drop records below ERROR level first
    BelowError,
    /// Never drop — block indefinitely
    Never,
}

/// Backpressure configuration bound to a performance profile.
#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    /// Maximum time to block before dropping (ms). 0 = infinite.
    pub block_timeout_ms: u64,
    /// Strategy for selecting which record to drop
    pub drop_strategy: DropStrategy,
    /// Whether this domain is AUDIT (enforces iron-law rules)
    pub is_audit_domain: bool,
}

impl BackpressureConfig {
    /// Create the backpressure config for a given performance profile.
    ///
    /// profile → (timeout, strategy, audit_behavior)
    pub fn for_profile(profile: PerformanceProfile, is_audit: bool) -> Self {
        let (timeout, strategy) = match profile {
            PerformanceProfile::Dev => (100, DropStrategy::Newest),
            PerformanceProfile::ProdPerformance => (3000, DropStrategy::BelowWarn),
            PerformanceProfile::ProdAudit => (3000, DropStrategy::BelowWarn),
            PerformanceProfile::Balanced => (2000, DropStrategy::Oldest),
        };

        // AUDIT iron law: force timeout=0, strategy=Never
        if is_audit {
            Self {
                block_timeout_ms: 0, // infinite
                drop_strategy: DropStrategy::Never,
                is_audit_domain: true,
            }
        } else {
            Self {
                block_timeout_ms: timeout,
                drop_strategy: strategy,
                is_audit_domain: false,
            }
        }
    }

    /// Validate that this config doesn't violate safety non-downgradable items.
    ///
    /// Returns `Ok(())` if valid, `Err(message)` if violation detected.
    pub fn validate(&self) -> Result<(), String> {
        // Non-AUDIT block_timeout_ms MUST NOT be < 100ms
        if !self.is_audit_domain && self.block_timeout_ms > 0 && self.block_timeout_ms < 100 {
            return Err(format!(
                "block_timeout_ms ({}) must be >= 100ms for non-AUDIT domains",
                self.block_timeout_ms
            ));
        }

        // AUDIT domain MUST have infinite block (0)
        if self.is_audit_domain && self.block_timeout_ms != 0 {
            return Err(format!(
                "AUDIT domain block_timeout_ms must be 0 (infinite), got {}",
                self.block_timeout_ms
            ));
        }

        // AUDIT domain MUST NOT have any drop_strategy
        if self.is_audit_domain && self.drop_strategy != DropStrategy::Never {
            return Err(
                "AUDIT domain must not configure any drop_strategy — forced to Never".into(),
            );
        }

        Ok(())
    }
}

/// Runtime backpressure state — determines whether to accept or drop a record.
///
/// Thread-safe: uses atomic counters for statistics.
pub struct BackpressureController {
    config: BackpressureConfig,
    /// Total records blocked (waiting for space)
    blocked_count: AtomicU64,
    /// Total records dropped
    dropped_count: AtomicU64,
    /// Total records accepted
    accepted_count: AtomicU64,
    /// Whether emergency buffer is active
    emergency_active: AtomicBool,
    /// When emergency mode was activated
    emergency_since: Mutex<Option<Instant>>,
    /// Current ring buffer fill level (0.0–1.0)
    fill_level: AtomicU64, // stored as permille (0–1000)
}

impl BackpressureController {
    /// Create a new backpressure controller with the given configuration.
    pub fn new(config: BackpressureConfig) -> Self {
        Self {
            config,
            blocked_count: AtomicU64::new(0),
            dropped_count: AtomicU64::new(0),
            accepted_count: AtomicU64::new(0),
            emergency_active: AtomicBool::new(false),
            emergency_since: Mutex::new(None),
            fill_level: AtomicU64::new(0),
        }
    }

    /// Evaluate whether to accept a record given the current ring buffer fill ratio.
    ///
    /// `fill_ratio` is 0.0–1.0 (0 = empty, 1.0 = full).
    /// `level` is the record's log level.
    ///
    /// Thresholds:
    /// - <90%: always accept
    /// - 90-95%: accept + cooperative helping + sysmon CRITICAL alert
    /// - ≥95%: apply drop_strategy + emergency buffer eligible
    ///
    /// Returns `true` if the record should be accepted, `false` if dropped.
    pub fn evaluate(&self, fill_ratio: f64, level: LogLevel) -> bool {
        let fill_permille = (fill_ratio * 1000.0) as u64;
        self.fill_level.store(fill_permille, Ordering::Relaxed);

        // Below 90% fill → always accept
        if fill_ratio < 0.90 {
            self.accepted_count.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        // AUDIT domain: never drop (iron law)
        if self.config.is_audit_domain {
            self.blocked_count.fetch_add(1, Ordering::Relaxed);
            self.accepted_count.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        // 90-95%: sysmon CRITICAL + cooperative helping signal
        if fill_ratio < 0.95 {
            if fill_permille >= 900 {
                crate::sys::diagnostics::warn(
                    "backpressure",
                    &format!(
                        "Ring buffer at {}% — cooperative helping recommended, blocked={}",
                        fill_ratio * 100.0,
                        self.blocked_count.load(Ordering::Relaxed)
                    ),
                );
            }
            self.blocked_count.fetch_add(1, Ordering::Relaxed);
            self.accepted_count.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        // ≥95%: apply drop strategy
        let should_drop = match self.config.drop_strategy {
            DropStrategy::Never => false,
            DropStrategy::Newest => {
                // Drop the record currently being submitted (this one)
                true
            }
            DropStrategy::Oldest => {
                // Caller handles oldest-eviction; here we accept
                false
            }
            DropStrategy::BelowWarn => level < LogLevel::Warn,
            DropStrategy::BelowError => level < LogLevel::Error,
        };

        if should_drop {
            self.dropped_count.fetch_add(1, Ordering::Relaxed);
            false
        } else {
            self.blocked_count.fetch_add(1, Ordering::Relaxed);
            self.accepted_count.fetch_add(1, Ordering::Relaxed);
            true
        }
    }

    /// Check if emergency buffer should be activated (>95% full for >5 seconds).
    pub fn check_emergency(&self) -> bool {
        let fill = self.fill_level.load(Ordering::Relaxed);
        let emergency = self.emergency_active.load(Ordering::Relaxed);

        if fill > 950 {
            // >95% full
            if !emergency {
                let mut since = self.emergency_since.lock().unwrap();
                if since.is_none() {
                    *since = Some(Instant::now());
                } else if since.as_ref().unwrap().elapsed() > Duration::from_secs(5) {
                    self.emergency_active.store(true, Ordering::Release);
                    crate::sys::diagnostics::warn(
                        "backpressure",
                        &format!(
                            "Emergency buffer activated: fill={}‰, accepted={}, dropped={}",
                            fill,
                            self.accepted_count(),
                            self.dropped_count(),
                        ),
                    );
                    return true;
                }
            }
        } else if emergency && fill < 500 {
            // Recovered
            self.emergency_active.store(false, Ordering::Release);
            *self.emergency_since.lock().unwrap() = None;
            crate::sys::diagnostics::info(
                "backpressure",
                "Emergency buffer deactivated — fill below 50%",
            );
        } else if fill < 950 {
            *self.emergency_since.lock().unwrap() = None;
        }

        emergency
    }

    /// Get the block timeout duration for this profile.
    pub fn block_timeout(&self) -> Duration {
        if self.config.block_timeout_ms == 0 {
            Duration::MAX // Infinite
        } else {
            Duration::from_millis(self.config.block_timeout_ms)
        }
    }

    /// Whether drops are allowed at all.
    pub fn drops_allowed(&self) -> bool {
        self.config.drop_strategy != DropStrategy::Never
    }

    /// Total records accepted.
    pub fn accepted_count(&self) -> u64 {
        self.accepted_count.load(Ordering::Relaxed)
    }

    /// Total records dropped.
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }

    /// Whether emergency mode is active.
    pub fn is_emergency(&self) -> bool {
        self.emergency_active.load(Ordering::Relaxed)
    }

    /// Current fill level as 0.0–1.0.
    pub fn fill_level(&self) -> f64 {
        self.fill_level.load(Ordering::Relaxed) as f64 / 1000.0
    }

    /// Returns `true` when the ring buffer occupancy is ≥90%, signalling
    /// that cooperative helping should kick in.
    pub fn should_help(&self) -> bool {
        self.fill_level.load(Ordering::Relaxed) >= 900
    }

    /// Returns the batch size for cooperative helping drains.
    ///
    /// Smaller than the consumer batch size (32 records) — just enough
    /// to relieve pressure without blocking the calling thread for too long.
    /// specifies cooperative helping uses a reduced drain window.
    pub fn helping_batch_size(&self) -> usize {
        32
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_never_drops() {
        let config = BackpressureConfig::for_profile(PerformanceProfile::ProdAudit, true);
        let ctrl = BackpressureController::new(config);

        // Even at 99% full, AUDIT records must be accepted
        assert!(ctrl.evaluate(0.99, LogLevel::Info));
        assert!(ctrl.evaluate(0.99, LogLevel::Trace));
        assert_eq!(ctrl.dropped_count(), 0);
    }

    #[test]
    fn test_below_warn_drops_low_levels() {
        let config = BackpressureConfig::for_profile(PerformanceProfile::ProdPerformance, false);
        let ctrl = BackpressureController::new(config);

        // Drops only above 95% fill. At 96%: TRACE/DEBUG/INFO dropped, WARN+ kept
        assert!(!ctrl.evaluate(0.96, LogLevel::Trace));
        assert!(!ctrl.evaluate(0.96, LogLevel::Debug));
        assert!(!ctrl.evaluate(0.96, LogLevel::Info));
        assert!(ctrl.evaluate(0.96, LogLevel::Warn));
        assert!(ctrl.evaluate(0.96, LogLevel::Error));
        assert!(ctrl.evaluate(0.96, LogLevel::Fatal));
        assert!(ctrl.evaluate(0.96, LogLevel::Audit));
    }

    #[test]
    fn test_dev_profile_accepts_below_90() {
        let config = BackpressureConfig::for_profile(PerformanceProfile::Dev, false);
        let ctrl = BackpressureController::new(config);

        // Below 90% always accept
        assert!(ctrl.evaluate(0.50, LogLevel::Trace));
        assert!(ctrl.evaluate(0.85, LogLevel::Trace));
        assert_eq!(ctrl.accepted_count(), 2);
    }

    #[test]
    fn test_audit_config_validation() {
        // AUDIT with non-zero timeout should fail
        let config = BackpressureConfig {
            block_timeout_ms: 1000,
            drop_strategy: DropStrategy::Newest,
            is_audit_domain: true,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_non_audit_min_timeout() {
        // Non-AUDIT with timeout < 100ms should fail
        let config = BackpressureConfig {
            block_timeout_ms: 50,
            drop_strategy: DropStrategy::Never,
            is_audit_domain: false,
        };
        assert!(config.validate().is_err());
    }
}
