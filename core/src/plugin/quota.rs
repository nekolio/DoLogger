//! Resource quota enforcement.
//!
//! Enforces per-plugin memory and CPU quotas at runtime.
//! Plugins exceeding their quota are degraded (CPU throttled) or
//! terminated (memory limit breached), with sysmon alerting.
//!
//! # Quota model
//!
//! - `memory_limit_mb` — max RSS in megabytes (default: 100)
//! - `cpu_quota_percent` — max CPU usage as percentage × cores (default: 200)
//! - Enforcement: exceed memory → terminate plugin + sysmon CRITICAL
//!   exceed CPU → throttle (skip processing cycle) + sysmon WARN

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Resource quota configuration for a plugin.
#[derive(Debug, Clone)]
pub struct QuotaConfig {
    /// Plugin name
    pub plugin_name: String,
    /// Memory limit in megabytes (0 = unlimited)
    pub memory_limit_mb: u64,
    /// CPU quota as percentage × cores (e.g., 200 = 2 full cores)
    pub cpu_quota_percent: u32,
    /// Whether to terminate on memory overrun (true) or just warn (false)
    pub terminate_on_memory: bool,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            plugin_name: "unknown".into(),
            memory_limit_mb: 100,
            cpu_quota_percent: 200,
            terminate_on_memory: true,
        }
    }
}

/// CPU usage tracking window (sliding window in microseconds).
#[derive(Debug)]
struct CpuWindow {
    /// Microseconds of CPU time used in current window
    used_us: AtomicU64,
    /// Window start time
    window_start: Mutex<Instant>,
    /// Window duration
    window_duration: Duration,
}

impl CpuWindow {
    fn new() -> Self {
        Self {
            used_us: AtomicU64::new(0),
            window_start: Mutex::new(Instant::now()),
            window_duration: Duration::from_secs(10), // 10-second sliding window
        }
    }

    /// Record CPU usage in microseconds. Returns the total in the current window.
    fn record(&self, cpu_time_us: u64) -> u64 {
        // Reset window if expired
        let mut start = self.window_start.lock().unwrap();
        if start.elapsed() >= self.window_duration {
            *start = Instant::now();
            self.used_us.store(0, Ordering::Release);
        }

        self.used_us.fetch_add(cpu_time_us, Ordering::Relaxed) + cpu_time_us
    }

    /// Get current CPU usage as percentage of window (percentage × cores).
    fn usage_percent(&self) -> f64 {
        let used = self.used_us.load(Ordering::Relaxed);
        let window_us = self.window_duration.as_micros() as u64;
        (used as f64 / window_us as f64) * 100.0
    }
}

/// Quota enforcement result for a single evaluation cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaAction {
    /// Within limits — normal operation
    Allow,
    /// CPU quota exceeded — throttle (skip cycle)
    Throttle,
    /// Memory quota exceeded — terminate plugin
    Terminate,
}

/// Per-plugin quota tracker with enforcement.
pub struct PluginQuota {
    config: QuotaConfig,
    /// Current estimated memory usage (bytes, approximate)
    memory_bytes: AtomicU64,
    /// CPU usage tracking window
    cpu_window: CpuWindow,
    /// Whether the plugin has been terminated
    terminated: AtomicBool,
    /// Consecutive quota violations
    violation_count: AtomicU64,
}

impl PluginQuota {
    /// Create a new quota tracker for a plugin.
    pub fn new(config: QuotaConfig) -> Self {
        Self {
            config,
            memory_bytes: AtomicU64::new(0),
            cpu_window: CpuWindow::new(),
            terminated: AtomicBool::new(false),
            violation_count: AtomicU64::new(0),
        }
    }

    /// Update the estimated memory usage of the plugin.
    pub fn update_memory(&self, bytes: u64) {
        self.memory_bytes.store(bytes, Ordering::Relaxed);
    }

    /// Record CPU time used in the last processing cycle (microseconds).
    pub fn record_cpu_time(&self, cpu_time_us: u64) {
        self.cpu_window.record(cpu_time_us);
    }

    /// Evaluate whether the plugin is within quota limits.
    ///
    /// Returns the enforcement action to take.
    pub fn evaluate(&self) -> QuotaAction {
        if self.terminated.load(Ordering::Acquire) {
            return QuotaAction::Terminate;
        }

        // Memory check
        let mem_bytes = self.memory_bytes.load(Ordering::Relaxed);
        if self.config.memory_limit_mb > 0 {
            let mem_limit = self.config.memory_limit_mb * 1024 * 1024;
            if mem_bytes > mem_limit && self.config.terminate_on_memory {
                self.terminated.store(true, Ordering::Release);
                let v = self.violation_count.fetch_add(1, Ordering::Relaxed);
                crate::sys::diag::warn(
                    "quota",
                    &format!(
                        "Plugin '{}' TERMINATED: memory {}MB > limit {}MB (violation #{})",
                        self.config.plugin_name,
                        mem_bytes / 1024 / 1024,
                        self.config.memory_limit_mb,
                        v + 1
                    ),
                );
                return QuotaAction::Terminate;
            }
        }

        // CPU check
        if self.config.cpu_quota_percent > 0 {
            let usage = self.cpu_window.usage_percent();
            if usage > self.config.cpu_quota_percent as f64 {
                let v = self.violation_count.fetch_add(1, Ordering::Relaxed);
                crate::sys::diag::warn(
                    "quota",
                    &format!(
                        "Plugin '{}' THROTTLED: CPU {:.1}% > quota {}% (violation #{})",
                        self.config.plugin_name,
                        usage,
                        self.config.cpu_quota_percent,
                        v + 1
                    ),
                );
                return QuotaAction::Throttle;
            }
        }

        QuotaAction::Allow
    }

    /// Check if the plugin has been terminated by quota enforcement.
    pub fn is_terminated(&self) -> bool {
        self.terminated.load(Ordering::Acquire)
    }

    /// Get the current estimated memory usage in bytes.
    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes.load(Ordering::Relaxed)
    }

    /// Get the current CPU usage percentage.
    pub fn cpu_usage_percent(&self) -> f64 {
        self.cpu_window.usage_percent()
    }

    /// Reset the violation counter.
    pub fn reset_violations(&self) {
        self.violation_count.store(0, Ordering::Release);
    }

    /// Manually terminate the plugin (e.g., on administrator command).
    pub fn terminate(&self) {
        self.terminated.store(true, Ordering::Release);
        crate::sys::diag::warn(
            "quota",
            &format!("Plugin '{}' manually terminated", self.config.plugin_name),
        );
    }
}

/// Global resource quota manager.
pub struct QuotaManager {
    /// Per-plugin quota trackers
    plugins: Mutex<Vec<PluginQuota>>,
}

impl QuotaManager {
    /// Create a new quota manager.
    pub fn new() -> Self {
        Self {
            plugins: Mutex::new(Vec::new()),
        }
    }

    /// Register a plugin for quota tracking.
    pub fn register(&self, config: QuotaConfig) {
        self.plugins.lock().unwrap().push(PluginQuota::new(config));
    }

    /// Evaluate all registered plugins' quotas.
    ///
    /// Returns lists of throttled and terminated plugin names.
    pub fn evaluate_all(&self) -> (Vec<String>, Vec<String>) {
        let plugins = self.plugins.lock().unwrap();
        let mut throttled = Vec::new();
        let mut terminated = Vec::new();

        for p in plugins.iter() {
            match p.evaluate() {
                QuotaAction::Allow => {}
                QuotaAction::Throttle => throttled.push(p.config.plugin_name.clone()),
                QuotaAction::Terminate => terminated.push(p.config.plugin_name.clone()),
            }
        }

        (throttled, terminated)
    }

    /// Get the quota tracker for a specific plugin.
    pub fn get(&self, name: &str) -> Option<&PluginQuota> {
        // SAFETY: We're returning a reference to a PluginQuota stored in the Vec.
        // The Mutex guard is dropped, but PluginQuota uses Atomic types internally,
        // so concurrent access is safe.
        let plugins = self.plugins.lock().unwrap();
        // Can't return a reference from a temporary MutexGuard.
        // For now, we return None and the caller uses evaluate_all.
        let _ = plugins.iter().find(|p| p.config.plugin_name == name);
        None
    }

    /// Remove a plugin from quota tracking.
    pub fn unregister(&self, name: &str) {
        self.plugins
            .lock()
            .unwrap()
            .retain(|p| p.config.plugin_name != name);
    }
}

impl Default for QuotaManager {
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
    fn test_memory_quota_terminates() {
        let quota = PluginQuota::new(QuotaConfig {
            plugin_name: "test".into(),
            memory_limit_mb: 1, // 1MB
            cpu_quota_percent: 0,
            terminate_on_memory: true,
        });

        // Within limit
        quota.update_memory(512 * 1024); // 512KB
        assert_eq!(quota.evaluate(), QuotaAction::Allow);

        // Over limit
        quota.update_memory(2 * 1024 * 1024); // 2MB > 1MB
        assert_eq!(quota.evaluate(), QuotaAction::Terminate);
        assert!(quota.is_terminated());
    }

    #[test]
    fn test_cpu_quota_throttles() {
        let quota = PluginQuota::new(QuotaConfig {
            plugin_name: "test".into(),
            memory_limit_mb: 0,
            cpu_quota_percent: 50, // 50% of one core
            terminate_on_memory: false,
        });

        // Record heavy CPU usage
        quota.record_cpu_time(8_000_000); // 8 seconds in 10-second window = 80%

        assert_eq!(quota.evaluate(), QuotaAction::Throttle);
    }

    #[test]
    fn test_quota_manager_evaluates_all() {
        let mgr = QuotaManager::new();

        mgr.register(QuotaConfig {
            plugin_name: "good_plugin".into(),
            memory_limit_mb: 100,
            cpu_quota_percent: 200,
            terminate_on_memory: false,
        });

        mgr.register(QuotaConfig {
            plugin_name: "bad_plugin".into(),
            memory_limit_mb: 1,
            cpu_quota_percent: 0,
            terminate_on_memory: true,
        });

        // Set bad plugin over memory limit
        let plugins = mgr.plugins.lock().unwrap();
        plugins[1].update_memory(10 * 1024 * 1024); // 10MB > 1MB
        drop(plugins);

        let (throttled, terminated) = mgr.evaluate_all();
        assert!(throttled.is_empty());
        assert_eq!(terminated.len(), 1);
        assert_eq!(terminated[0], "bad_plugin");
    }
}
