//! Circuit breaker for remote sinks.
//!
//! Protects against cascading failures when remote downstream services
//! (Kafka, Syslog, Webhook) become unavailable. Each remote Sink has
//! an independent circuit breaker.
//!
//! # State machine
//!
//! ```text
//! CLOSED ──(failures >= threshold)──▶ OPEN
//!   ▲                                    │
//!   │                              (timeout_ms)
//!   │                                    ▼
//!   └────(probe success)──── HALF_OPEN ◀─┘
//!          (probe failure)────────────▶ OPEN
//! ```
//!
//! For AUDIT domains: `failure_threshold >= 3`, `timeout_ms >= 60000`.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests pass through
    Closed,
    /// Failure threshold exceeded — all requests rejected immediately
    Open,
    /// Probing — limited requests allowed to test if service recovered
    HalfOpen,
}

/// Circuit breaker configuration per remote Sink.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Consecutive failures before opening the circuit
    pub failure_threshold: u32,
    /// Time in OPEN state before transitioning to HALF_OPEN (ms)
    pub timeout_ms: u64,
    /// Max probe requests allowed in HALF_OPEN state
    pub half_open_max_requests: u32,
    /// Sliding window for failure counting (seconds, 0 = no window)
    pub roll_window_sec: u64,
    /// Whether this breaker is for an AUDIT domain (enforces stricter limits)
    pub is_audit: bool,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            timeout_ms: 30_000,
            half_open_max_requests: 3,
            roll_window_sec: 60,
            is_audit: false,
        }
    }
}

impl CircuitBreakerConfig {
    /// Validate circuit breaker constraints.
    pub fn validate(&self) -> Result<(), String> {
        if self.is_audit {
            if self.failure_threshold < 3 {
                return Err(format!(
                    "AUDIT circuit breaker failure_threshold must be >= 3, got {}",
                    self.failure_threshold
                ));
            }
            if self.timeout_ms < 60_000 {
                return Err(format!(
                    "AUDIT circuit breaker timeout_ms must be >= 60000, got {}",
                    self.timeout_ms
                ));
            }
        }
        Ok(())
    }
}

/// Internal circuit breaker state (guarded by a single Mutex to avoid deadlocks).
struct InnerState {
    state: CircuitState,
    opened_at: Option<Instant>,
}

/// Runtime circuit breaker for a single remote Sink.
///
/// Thread-safe: uses atomic counters for statistics; a single Mutex guards
/// state transitions to prevent deadlocks.
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    /// Guarded inner state (state + timestamp)
    inner: Mutex<InnerState>,
    /// Consecutive failure count
    failure_count: AtomicU32,
    /// Total successful calls
    success_count: AtomicU64,
    /// Total rejected calls (circuit open)
    rejected_count: AtomicU64,
    /// Probe requests remaining in HALF_OPEN
    probes_remaining: AtomicU32,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(InnerState {
                state: CircuitState::Closed,
                opened_at: None,
            }),
            failure_count: AtomicU32::new(0),
            success_count: AtomicU64::new(0),
            rejected_count: AtomicU64::new(0),
            probes_remaining: AtomicU32::new(0),
        }
    }

    /// Called before each Sink write. Returns `true` if the request is allowed.
    ///
    /// If the circuit is OPEN and timeout has elapsed, transitions to HALF_OPEN
    /// and allows the request as a probe.
    pub fn allow_request(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();

        match inner.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(opened) = inner.opened_at {
                    if opened.elapsed() >= Duration::from_millis(self.config.timeout_ms) {
                        inner.state = CircuitState::HalfOpen;
                        self.probes_remaining
                            .store(self.config.half_open_max_requests, Ordering::Release);
                        crate::sys::diagnostics::info(
                            "circuit_breaker",
                            "Circuit transitioned: OPEN → HALF_OPEN (probe)",
                        );
                        drop(inner);
                        return self.allow_request();
                    }
                } else {
                    inner.opened_at = Some(Instant::now());
                }
                self.rejected_count.fetch_add(1, Ordering::Relaxed);
                false
            }
            CircuitState::HalfOpen => {
                let remaining = self.probes_remaining.load(Ordering::Acquire);
                if remaining > 0 {
                    self.probes_remaining.fetch_sub(1, Ordering::AcqRel);
                    true
                } else {
                    self.rejected_count.fetch_add(1, Ordering::Relaxed);
                    false
                }
            }
        }
    }

    /// Report a successful Sink write. Resets failure count and closes circuit.
    pub fn report_success(&self) {
        self.success_count.fetch_add(1, Ordering::Relaxed);
        self.failure_count.store(0, Ordering::Release);

        let mut inner = self.inner.lock().unwrap();
        if inner.state == CircuitState::HalfOpen {
            inner.state = CircuitState::Closed;
            inner.opened_at = None;
            crate::sys::diagnostics::info(
                "circuit_breaker",
                "Circuit transitioned: HALF_OPEN → CLOSED (probe success)",
            );
        }
    }

    /// Report a failed Sink write. Increments failure count and opens circuit
    /// if threshold reached.
    pub fn report_failure(&self) {
        let prev = self.failure_count.fetch_add(1, Ordering::AcqRel);

        let mut inner = self.inner.lock().unwrap();
        match inner.state {
            CircuitState::Closed => {
                if prev + 1 >= self.config.failure_threshold {
                    inner.state = CircuitState::Open;
                    inner.opened_at = Some(Instant::now());
                    crate::sys::diagnostics::warn(
                        "circuit_breaker",
                        &format!(
                            "Circuit breaker OPENED after {} consecutive failures (threshold: {})",
                            prev + 1,
                            self.config.failure_threshold
                        ),
                    );
                }
            }
            CircuitState::HalfOpen => {
                inner.state = CircuitState::Open;
                inner.opened_at = Some(Instant::now());
                self.probes_remaining.store(0, Ordering::Release);
                crate::sys::diagnostics::warn(
                    "circuit_breaker",
                    "Circuit transitioned: HALF_OPEN → OPEN (probe failed)",
                );
            }
            CircuitState::Open => {
                inner.opened_at = Some(Instant::now());
            }
        }
    }

    /// Get the current circuit state.
    pub fn state(&self) -> CircuitState {
        self.inner.lock().unwrap().state
    }

    /// Get failure statistics.
    pub fn failure_count(&self) -> u32 {
        self.failure_count.load(Ordering::Relaxed)
    }

    /// Get success statistics.
    pub fn success_count(&self) -> u64 {
        self.success_count.load(Ordering::Relaxed)
    }

    /// Get rejected count.
    pub fn rejected_count(&self) -> u64 {
        self.rejected_count.load(Ordering::Relaxed)
    }

    /// Reset the circuit breaker to CLOSED state.
    pub fn reset(&self) {
        self.failure_count.store(0, Ordering::Release);
        let mut inner = self.inner.lock().unwrap();
        inner.state = CircuitState::Closed;
        inner.opened_at = None;
        self.probes_remaining.store(0, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_opens_after_threshold_failures() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            timeout_ms: 60_000,
            ..Default::default()
        });

        // First 3 requests allowed
        assert!(breaker.allow_request());
        assert!(breaker.allow_request());
        assert!(breaker.allow_request());

        // Report 3 failures
        breaker.report_failure();
        breaker.report_failure();
        breaker.report_failure();

        // Circuit should now be OPEN
        assert_eq!(breaker.state(), CircuitState::Open);
        assert!(!breaker.allow_request(), "Should reject when OPEN");
        assert_eq!(breaker.rejected_count(), 1);
    }

    #[test]
    fn test_half_open_probe_limits() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_ms: 0, // Immediate transition
            half_open_max_requests: 2,
            ..Default::default()
        });

        // Open the circuit
        breaker.report_failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        // Next request should transition to HALF_OPEN (timeout_ms=0)
        assert!(breaker.allow_request()); // Probe 1
        assert_eq!(breaker.state(), CircuitState::HalfOpen);

        assert!(breaker.allow_request()); // Probe 2
        assert!(!breaker.allow_request()); // Probe limit reached
    }

    #[test]
    fn test_success_closes_circuit() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_ms: 0,
            ..Default::default()
        });

        // Open → HALF_OPEN
        breaker.report_failure();
        breaker.allow_request(); // Triggers HALF_OPEN

        // Successful probe
        breaker.report_success();

        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.failure_count(), 0);
    }

    #[test]
    fn test_audit_config_validation() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1, // Too low for AUDIT
            timeout_ms: 30_000,   // Too low for AUDIT
            is_audit: true,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let valid = CircuitBreakerConfig {
            failure_threshold: 3,
            timeout_ms: 60_000,
            is_audit: true,
            ..Default::default()
        };
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn test_non_audit_config_is_permissive() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_ms: 100,
            is_audit: false,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_reset_clears_state() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        });

        breaker.report_failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        breaker.reset();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.failure_count(), 0);
    }
}
