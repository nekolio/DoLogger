//! Canary probing for Sink health.
//!
//! Periodic health-check writes to each active Sink to detect failures
//! before real log data is lost. A canary is a synthetic log record
//! written at a configurable interval; if the canary write fails,
//! the Sink is marked unhealthy and the circuit breaker is tripped.
//!
//! # Design
//!
//! - Interval: configurable (default: 30 seconds)
//! - Canary record: fixed content with `canary=true` marker
//! - Failure handling: on canary failure → trip circuit breaker →
//!   trigger fallback chain → sysmon CRITICAL alert
//! - Recovery: on canary success after failure → close circuit breaker →
//!   sysmon INFO recovery event
//! - AUDIT: canary interval MUST be ≤ 10 seconds for AUDIT sinks
//! - Overhead: canary records are minimal (~64 bytes) and counted separately
//!   from real log throughput statistics

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Result of a single canary probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanaryResult {
    /// Canary write succeeded — sink is healthy
    Healthy,
    /// Canary write failed — sink is unhealthy
    Unhealthy,
    /// Canary probe skipped (e.g., sink not yet opened)
    Skipped,
}

/// Health status of a probed Sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkHealth {
    /// Sink is healthy
    Healthy,
    /// Sink is degraded (last canary failed, recovery possible)
    Degraded,
    /// Sink is dead (consecutive canary failures exceeded threshold)
    Dead,
}

/// Configuration for a canary prober.
#[derive(Debug, Clone)]
pub struct CanaryConfig {
    /// Interval between canary writes
    pub interval_secs: u64,
    /// Consecutive failures before marking as DEAD
    pub failure_threshold: u32,
    /// Whether this canary is for an AUDIT sink (stricter timing)
    pub is_audit: bool,
}

impl Default for CanaryConfig {
    fn default() -> Self {
        Self {
            interval_secs: 10, // core probes every 10 seconds
            failure_threshold: 3,
            is_audit: false,
        }
    }
}

impl CanaryConfig {
    /// Create config for an AUDIT sink (≤10s interval, 3 consecutive failures).
    pub fn audit() -> Self {
        Self {
            interval_secs: 10,
            failure_threshold: 3, // 3 consecutive probe failures triggers DEAD
            is_audit: true,
        }
    }

    /// Validate configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.interval_secs == 0 {
            return Err("Canary interval must be > 0".into());
        }
        if self.is_audit && self.interval_secs > 10 {
            return Err(format!(
                "AUDIT canary interval must be ≤ 10s, got {}s",
                self.interval_secs
            ));
        }
        if self.failure_threshold == 0 {
            return Err("Canary failure_threshold must be > 0".into());
        }
        Ok(())
    }
}

/// Statistics for canary probing.
#[derive(Debug, Clone, Default)]
pub struct CanaryStats {
    /// Total canary probes sent
    pub total_probes: u64,
    /// Successful probes
    pub successful: u64,
    /// Failed probes
    pub failed: u64,
    /// Consecutive failures (current streak)
    pub consecutive_failures: u32,
    /// Whether the sink is currently dead
    pub is_dead: bool,
    /// Time of the last successful probe
    pub last_success: Option<Instant>,
    /// Time of the last failed probe
    pub last_failure: Option<Instant>,
}

/// Canary prober for a single Sink.
///
/// Thread-safe: periodic probes from a background timer thread.
/// The actual write is performed by the caller (who has access to the Sink).
pub struct CanaryProber {
    config: CanaryConfig,
    /// When the last canary was sent
    last_probe: Mutex<Instant>,
    /// Statistics
    total_probes: AtomicU64,
    successful: AtomicU64,
    failed: AtomicU64,
    consecutive_failures: AtomicU64,
    /// Whether the sink is dead (stopped probing)
    dead: AtomicBool,
    /// Sink name for diagnostics
    sink_name: String,
}

impl CanaryProber {
    /// Create a new canary prober for a Sink.
    pub fn new(sink_name: &str, config: CanaryConfig) -> Self {
        Self {
            config,
            last_probe: Mutex::new(Instant::now()),
            total_probes: AtomicU64::new(0),
            successful: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            consecutive_failures: AtomicU64::new(0),
            dead: AtomicBool::new(false),
            sink_name: sink_name.to_string(),
        }
    }

    /// Check if it's time to send a canary probe.
    ///
    /// Returns `true` if the interval has elapsed since the last probe.
    pub fn should_probe(&self) -> bool {
        if self.dead.load(Ordering::Acquire) {
            return false; // Don't probe dead sinks
        }

        let last = self.last_probe.lock().unwrap();
        last.elapsed() >= Duration::from_secs(self.config.interval_secs)
    }

    /// Mark that a canary probe is about to be sent.
    /// Resets the timer so we don't spam probes.
    pub fn mark_probe_sent(&self) {
        *self.last_probe.lock().unwrap() = Instant::now();
        self.total_probes.fetch_add(1, Ordering::Relaxed);
    }

    /// Report the result of a canary probe.
    ///
    /// Returns the new health status.
    pub fn report_result(&self, result: CanaryResult) -> SinkHealth {
        match result {
            CanaryResult::Healthy => {
                self.successful.fetch_add(1, Ordering::Relaxed);
                self.consecutive_failures.store(0, Ordering::Release);

                // Recovery: if previously dead or degraded, log recovery
                if self.dead.load(Ordering::Acquire) {
                    self.dead.store(false, Ordering::Release);
                    crate::sys::diag::info(
                        "canary",
                        &format!(
                            "Sink '{}' RECOVERED — canary probe succeeded",
                            self.sink_name
                        ),
                    );
                }

                SinkHealth::Healthy
            }
            CanaryResult::Unhealthy => {
                self.failed.fetch_add(1, Ordering::Relaxed);
                let consecutive = self.consecutive_failures.fetch_add(1, Ordering::AcqRel) + 1;

                if consecutive >= self.config.failure_threshold as u64 {
                    self.dead.store(true, Ordering::Release);
                    crate::sys::diag::error(
                        "canary",
                        &format!(
                            "Sink '{}' DEAD — {} consecutive canary failures (threshold: {})",
                            self.sink_name, consecutive, self.config.failure_threshold
                        ),
                    );
                    SinkHealth::Dead
                } else {
                    crate::sys::diag::warn(
                        "canary",
                        &format!(
                            "Sink '{}' DEGRADED — canary probe failed ({}/{})",
                            self.sink_name, consecutive, self.config.failure_threshold
                        ),
                    );
                    SinkHealth::Degraded
                }
            }
            CanaryResult::Skipped => {
                // Don't count skipped probes
                if self.dead.load(Ordering::Acquire) {
                    SinkHealth::Dead
                } else if self.consecutive_failures.load(Ordering::Acquire) > 0 {
                    SinkHealth::Degraded
                } else {
                    SinkHealth::Healthy
                }
            }
        }
    }

    /// Get the current health status without probing.
    pub fn health(&self) -> SinkHealth {
        if self.dead.load(Ordering::Acquire) {
            SinkHealth::Dead
        } else if self.consecutive_failures.load(Ordering::Acquire) > 0 {
            SinkHealth::Degraded
        } else {
            SinkHealth::Healthy
        }
    }

    /// Get canary statistics.
    pub fn stats(&self) -> CanaryStats {
        CanaryStats {
            total_probes: self.total_probes.load(Ordering::Relaxed),
            successful: self.successful.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed) as u32,
            is_dead: self.dead.load(Ordering::Acquire),
            last_success: None, // Updated on report
            last_failure: None,
        }
    }

    /// Reset the canary prober (e.g., after sink reconnection).
    pub fn reset(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
        self.dead.store(false, Ordering::Release);
        *self.last_probe.lock().unwrap() = Instant::now();
        crate::sys::diag::info(
            "canary",
            &format!("Sink '{}' canary prober reset", self.sink_name),
        );
    }

    /// Get the sink name.
    pub fn sink_name(&self) -> &str {
        &self.sink_name
    }
}

/// Manager for all canary probers in the pipeline.
pub struct CanaryManager {
    /// All registered canary probers
    probers: Mutex<Vec<Arc<CanaryProber>>>,
}

impl CanaryManager {
    /// Create a new canary manager.
    pub fn new() -> Self {
        Self {
            probers: Mutex::new(Vec::new()),
        }
    }

    /// Register a canary prober.
    pub fn register(&self, prober: Arc<CanaryProber>) {
        self.probers.lock().unwrap().push(prober);
    }

    /// Unregister a canary prober by sink name.
    pub fn unregister(&self, sink_name: &str) {
        self.probers
            .lock()
            .unwrap()
            .retain(|p| p.sink_name() != sink_name);
    }

    /// Check all registered probers and return those due for probing.
    pub fn due_probes(&self) -> Vec<Arc<CanaryProber>> {
        self.probers
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.should_probe())
            .cloned()
            .collect()
    }

    /// Get health status for all registered sinks.
    pub fn all_health(&self) -> Vec<(String, SinkHealth)> {
        self.probers
            .lock()
            .unwrap()
            .iter()
            .map(|p| (p.sink_name().to_string(), p.health()))
            .collect()
    }

    /// Check if any registered sink is dead.
    pub fn any_dead(&self) -> bool {
        self.probers
            .lock()
            .unwrap()
            .iter()
            .any(|p| matches!(p.health(), SinkHealth::Dead))
    }

    /// Get the count of registered probers.
    pub fn count(&self) -> usize {
        self.probers.lock().unwrap().len()
    }
}

impl Default for CanaryManager {
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
    fn test_healthy_after_success() {
        let prober = CanaryProber::new("test_sink", CanaryConfig::default());

        // Initial state: healthy (no failures)
        assert_eq!(prober.health(), SinkHealth::Healthy);

        // Report success
        let health = prober.report_result(CanaryResult::Healthy);
        assert_eq!(health, SinkHealth::Healthy);
        assert_eq!(prober.stats().successful, 1);
        assert_eq!(prober.stats().consecutive_failures, 0);
    }

    #[test]
    fn test_degraded_after_failure() {
        let prober = CanaryProber::new("test_sink", CanaryConfig::default());

        // Report one failure → degraded
        let health = prober.report_result(CanaryResult::Unhealthy);
        assert_eq!(health, SinkHealth::Degraded);
        assert_eq!(prober.stats().consecutive_failures, 1);
    }

    #[test]
    fn test_dead_after_threshold_failures() {
        let config = CanaryConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let prober = CanaryProber::new("test_sink", config);

        // Two failures → still degraded
        assert_eq!(
            prober.report_result(CanaryResult::Unhealthy),
            SinkHealth::Degraded
        );
        assert_eq!(
            prober.report_result(CanaryResult::Unhealthy),
            SinkHealth::Degraded
        );

        // Third failure → dead
        assert_eq!(
            prober.report_result(CanaryResult::Unhealthy),
            SinkHealth::Dead
        );
        assert!(prober.stats().is_dead);
    }

    #[test]
    fn test_recovery_after_success() {
        let config = CanaryConfig {
            failure_threshold: 2,
            ..Default::default()
        };
        let prober = CanaryProber::new("test_sink", config);

        // One failure
        prober.report_result(CanaryResult::Unhealthy);
        assert_eq!(prober.health(), SinkHealth::Degraded);

        // Then success → recovery
        let health = prober.report_result(CanaryResult::Healthy);
        assert_eq!(health, SinkHealth::Healthy);
        assert_eq!(prober.stats().consecutive_failures, 0);
    }

    #[test]
    fn test_should_probe_timing() {
        let config = CanaryConfig {
            interval_secs: 3600, // 1 hour — won't trigger
            ..Default::default()
        };
        let prober = CanaryProber::new("test_sink", config);

        // Just created → should not probe yet (interval hasn't elapsed)
        assert!(!prober.should_probe());
    }

    #[test]
    fn test_dead_sink_should_not_probe() {
        let config = CanaryConfig {
            failure_threshold: 1,
            interval_secs: 0, // Immediate — but dead sinks don't probe
            ..Default::default()
        };
        let prober = CanaryProber::new("test_sink", config);

        // Kill the sink
        prober.report_result(CanaryResult::Unhealthy);
        assert_eq!(prober.health(), SinkHealth::Dead);

        // Dead sinks should not probe
        assert!(!prober.should_probe());
    }

    #[test]
    fn test_audit_config_validation() {
        // AUDIT canary with interval > 10s should fail
        let config = CanaryConfig {
            interval_secs: 30,
            is_audit: true,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        // AUDIT canary with interval ≤ 10s should pass
        let config = CanaryConfig {
            interval_secs: 10,
            is_audit: true,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_canary_manager_registration() {
        let mgr = CanaryManager::new();
        let prober = Arc::new(CanaryProber::new("sink1", CanaryConfig::default()));

        mgr.register(Arc::clone(&prober));
        assert_eq!(mgr.count(), 1);

        mgr.unregister("sink1");
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn test_reset_clears_state() {
        let prober = CanaryProber::new("test_sink", CanaryConfig::default());

        // Cause some failures
        prober.report_result(CanaryResult::Unhealthy);
        prober.report_result(CanaryResult::Unhealthy);
        assert_eq!(prober.health(), SinkHealth::Degraded);

        // Reset
        prober.reset();
        assert_eq!(prober.health(), SinkHealth::Healthy);
        assert_eq!(prober.stats().consecutive_failures, 0);
    }
}
