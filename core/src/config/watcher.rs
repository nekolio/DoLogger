//! Config file watcher for hot reload.
//!
//! Monitors configuration files for changes and triggers reload
//! callbacks. Uses polling-based detection (portable across platforms)
//! with debounce to avoid spurious reloads.
//!
//! # Design
//!
//! 1. Watch config file path for mtime changes
//! 2. On change detection: debounce (wait for writes to settle)
//! 3. Trigger reload callback
//! 4. On reload failure: keep old config, emit sysmon ERROR
//! 5. On reload success: atomic pointer swap, emit sysmon INFO
//!
//! # Future upgrade path
//!
//! The current polling implementation can be upgraded platform-specifically:
//! - Linux: `inotify` via `inotify` crate
//! - Windows: `ReadDirectoryChangesW` via Windows API
//! - macOS: `FSEvents` or `kqueue`
//!
//! The `ConfigWatcher` trait-based design allows swapping the backend
//! without changing the hot-reload integration.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

/// Result of a config reload attempt.
#[derive(Debug, Clone)]
pub struct ReloadEvent {
    /// Path of the changed file
    pub path: PathBuf,
    /// When the change was detected
    pub detected_at: SystemTime,
    /// Whether the reload succeeded
    pub success: bool,
    /// Error message if reload failed
    pub error: Option<String>,
    /// Previous file mtime
    pub old_mtime: Option<SystemTime>,
    /// New file mtime
    pub new_mtime: Option<SystemTime>,
}

/// Platform-native file watcher backends.
///
/// The polling backend (mtime comparison) is the universal fallback.
/// Native backends provide lower latency and TOCTOU-resistant change
/// detection via OS kernel event streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherBackend {
    /// Polling-based mtime detection (universal fallback, ~1000ms latency)
    Polling,
    /// Linux inotify (kernel event stream, ~1ms latency)
    Inotify,
    /// Windows ReadDirectoryChangesW (overlapped I/O, ~1ms latency)
    ReadDirectoryChanges,
    /// macOS FSEvents (kernel event stream, ~1ms latency)
    Fsevents,
}

impl WatcherBackend {
    /// Detect the best available watcher backend for the current platform.
    /// Returns the native backend when available, falling back to Polling.
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            // inotify is available on all Linux kernels ≥ 2.6.13 (2005).
            // Full inotify integration is deferred; use polling for now.
            WatcherBackend::Polling // TODO: upgrade to Inotify via `inotify` crate
        }
        #[cfg(windows)]
        {
            // ReadDirectoryChangesW is available on all supported Windows versions.
            // Full integration is deferred; use polling for now.
            WatcherBackend::Polling // TODO: upgrade to ReadDirectoryChanges
        }
        #[cfg(target_os = "macos")]
        {
            WatcherBackend::Polling // TODO: upgrade to Fsevents via `fsevent` crate
        }
        #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
        {
            WatcherBackend::Polling
        }
    }

    /// Whether this backend uses native kernel events (faster, TOCTOU-resistant).
    pub fn is_native(&self) -> bool {
        matches!(
            self,
            WatcherBackend::Inotify
                | WatcherBackend::ReadDirectoryChanges
                | WatcherBackend::Fsevents
        )
    }
}

/// Configuration for the file watcher.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// Active watcher backend (auto-detected at startup)
    pub backend: WatcherBackend,
    /// Poll interval (how often to check for changes, polling only)
    pub poll_interval_ms: u64,
    /// Debounce duration (wait after last change before reloading)
    pub debounce_ms: u64,
    /// Whether to watch the file at all (can disable hot reload)
    pub enabled: bool,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            backend: WatcherBackend::detect(),
            poll_interval_ms: 1000, // Check every 1 second (polling)
            debounce_ms: 500,       // Wait 500ms after last change
            enabled: true,
        }
    }
}

/// A file being watched for changes.
struct WatchedFile {
    /// Absolute path to the file
    path: PathBuf,
    /// Last known modification time
    last_mtime: Option<SystemTime>,
    /// When the last change was detected (for debounce)
    last_change_at: Option<Instant>,
    /// Whether a reload is pending (debouncing)
    reload_pending: bool,
}

/// Callback type for config reload.
pub type ReloadCallback = Box<dyn Fn(&Path) -> Result<(), String> + Send + 'static>;

/// Config file watcher with polling-based change detection.
///
/// Spawns a background thread that periodically checks file mtimes.
/// When a change is detected, the debounce timer starts; if no further
/// changes occur within the debounce window, the reload callback fires.
pub struct ConfigWatcher {
    /// Watched files
    files: Arc<Mutex<Vec<WatchedFile>>>,
    /// Reload callback
    on_reload: Arc<Mutex<Option<ReloadCallback>>>,
    /// Shutdown flag
    shutdown: Arc<AtomicBool>,
    /// Background watcher thread
    _watcher_thread: Option<JoinHandle<()>>,
    /// Reload history (last N events)
    history: Arc<Mutex<Vec<ReloadEvent>>>,
}

impl ConfigWatcher {
    /// Create a new config watcher.
    ///
    /// The watcher starts immediately and monitors the given file paths.
    /// `on_reload` is called when a file change is detected and debounced.
    pub fn start(
        watch_paths: Vec<PathBuf>,
        on_reload: ReloadCallback,
        config: WatcherConfig,
    ) -> Result<Self, String> {
        if !config.enabled {
            return Ok(Self {
                files: Arc::new(Mutex::new(Vec::new())),
                on_reload: Arc::new(Mutex::new(None)),
                shutdown: Arc::new(AtomicBool::new(false)),
                _watcher_thread: None,
                history: Arc::new(Mutex::new(Vec::new())),
            });
        }

        // Canonicalize all paths
        let watched: Vec<WatchedFile> = watch_paths
            .into_iter()
            .map(|p| {
                let canon = fs::canonicalize(&p).unwrap_or(p);
                let mtime = fs::metadata(&canon).ok().and_then(|m| m.modified().ok());
                WatchedFile {
                    path: canon,
                    last_mtime: mtime,
                    last_change_at: None,
                    reload_pending: false,
                }
            })
            .collect();

        if watched.is_empty() {
            return Err("ConfigWatcher: no valid paths to watch".into());
        }

        let files = Arc::new(Mutex::new(watched));
        let on_reload = Arc::new(Mutex::new(Some(on_reload)));
        let shutdown = Arc::new(AtomicBool::new(false));
        let history = Arc::new(Mutex::new(Vec::new()));

        let files_clone = Arc::clone(&files);
        let reload_clone = Arc::clone(&on_reload);
        let shutdown_clone = Arc::clone(&shutdown);
        let history_clone = Arc::clone(&history);
        let cfg = config.clone();

        let watcher_thread = thread::Builder::new()
            .name("dologger-config-watcher".into())
            .spawn(move || {
                watcher_loop(
                    files_clone,
                    reload_clone,
                    shutdown_clone,
                    history_clone,
                    cfg,
                );
            })
            .map_err(|e| format!("ConfigWatcher: cannot spawn thread: {e}"))?;

        crate::sys::diagnostics::info(
            "config_watcher",
            &format!(
                "Config watcher started: poll={}ms, debounce={}ms",
                config.poll_interval_ms, config.debounce_ms
            ),
        );

        Ok(Self {
            files,
            on_reload,
            shutdown,
            _watcher_thread: Some(watcher_thread),
            history,
        })
    }

    /// Add a file path to watch.
    pub fn watch(&self, path: &Path) -> Result<(), String> {
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mtime = fs::metadata(&canon).ok().and_then(|m| m.modified().ok());

        let mut files = self.files.lock().unwrap();

        // Don't duplicate
        if files.iter().any(|f| f.path == canon) {
            return Ok(());
        }

        files.push(WatchedFile {
            path: canon,
            last_mtime: mtime,
            last_change_at: None,
            reload_pending: false,
        });

        Ok(())
    }

    /// Remove a file path from watching.
    pub fn unwatch(&self, path: &Path) {
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.files.lock().unwrap().retain(|f| f.path != canon);
    }

    /// Trigger an immediate reload of the given path, bypassing debounce.
    pub fn reload_now(&self, path: &Path) -> Result<(), String> {
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let reload = self.on_reload.lock().unwrap();
        if let Some(ref cb) = *reload {
            cb(&canon)
        } else {
            Err("No reload callback registered".into())
        }
    }

    /// Get reload history.
    pub fn reload_history(&self) -> Vec<ReloadEvent> {
        self.history.lock().unwrap().clone()
    }

    /// Whether the watcher is running.
    pub fn is_running(&self) -> bool {
        self._watcher_thread.is_some() && !self.shutdown.load(Ordering::Acquire)
    }

    /// Shutdown the watcher gracefully.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self._watcher_thread.take() {
            let _ = handle.join();
        }
        crate::sys::diagnostics::info("config_watcher", "Config watcher stopped");
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

/// Background watcher loop.
fn watcher_loop(
    files: Arc<Mutex<Vec<WatchedFile>>>,
    on_reload: Arc<Mutex<Option<ReloadCallback>>>,
    shutdown: Arc<AtomicBool>,
    history: Arc<Mutex<Vec<ReloadEvent>>>,
    config: WatcherConfig,
) {
    let poll_duration = Duration::from_millis(config.poll_interval_ms);
    let debounce_duration = Duration::from_millis(config.debounce_ms);

    while !shutdown.load(Ordering::Acquire) {
        let mut files_guard = files.lock().unwrap();
        let now = Instant::now();
        let mut reload_paths: Vec<PathBuf> = Vec::new();

        for watched in files_guard.iter_mut() {
            // Check if file exists and get mtime
            if let Ok(meta) = fs::metadata(&watched.path) {
                if let Ok(mtime) = meta.modified() {
                    let changed = match watched.last_mtime {
                        Some(last) => mtime != last,
                        None => true, // First check — file appeared
                    };

                    if changed {
                        watched.last_mtime = Some(mtime);
                        watched.last_change_at = Some(now);
                        watched.reload_pending = true;
                    } else if watched.reload_pending {
                        // Check if debounce period has elapsed
                        if let Some(changed_at) = watched.last_change_at {
                            if now.duration_since(changed_at) >= debounce_duration {
                                watched.reload_pending = false;
                                reload_paths.push(watched.path.clone());
                            }
                        }
                    }
                }
            } else {
                // File doesn't exist — reset mtime tracking
                watched.last_mtime = None;
                watched.reload_pending = false;
            }
        }

        drop(files_guard);

        // Execute reloads outside the lock
        for path in &reload_paths {
            let old_mtime = {
                let guard = files.lock().unwrap();
                guard
                    .iter()
                    .find(|f| &f.path == path)
                    .and_then(|f| f.last_mtime)
            };

            let new_mtime = fs::metadata(path).ok().and_then(|m| m.modified().ok());

            let reload_result = {
                let guard = on_reload.lock().unwrap();
                if let Some(ref cb) = *guard {
                    cb(path)
                } else {
                    Err("No reload callback".into())
                }
            };

            let event = ReloadEvent {
                path: path.clone(),
                detected_at: SystemTime::now(),
                success: reload_result.is_ok(),
                error: reload_result.err(),
                old_mtime,
                new_mtime,
            };

            if event.success {
                crate::sys::diagnostics::info(
                    "config_watcher",
                    &format!("Config reloaded successfully: {}", path.display()),
                );
            } else {
                crate::sys::diagnostics::error(
                    "config_watcher",
                    &format!(
                        "Config reload FAILED for {}: {}",
                        path.display(),
                        event.error.as_deref().unwrap_or("unknown error")
                    ),
                );
            }

            // Record history
            let mut hist = history.lock().unwrap();
            hist.push(event);
            if hist.len() > 100 {
                hist.remove(0);
            }
        }

        thread::sleep(poll_duration);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_watcher_disabled() {
        let config = WatcherConfig {
            enabled: false,
            ..Default::default()
        };

        let watcher = ConfigWatcher::start(
            vec![PathBuf::from("/nonexistent")],
            Box::new(|_| Ok(())),
            config,
        );

        assert!(watcher.is_ok());
        let w = watcher.unwrap();
        assert!(!w.is_running()); // Disabled watcher doesn't run
    }

    #[test]
    fn test_watch_and_unwatch() {
        // Create a temp file to watch
        let temp_dir = std::env::temp_dir().join("dologger_test_watcher");
        let _ = fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join("test_config.toml");
        let mut f = fs::File::create(&test_file).unwrap();
        f.write_all(b"[dologger]\nlevel = \"DEBUG\"\n").unwrap();
        drop(f);

        let config = WatcherConfig {
            enabled: false, // Don't start background thread for test
            ..Default::default()
        };

        let watcher = ConfigWatcher::start(vec![], Box::new(|_| Ok(())), config).unwrap();

        // Watch the test file
        watcher.watch(&test_file).unwrap();

        // Unwatch
        watcher.unwatch(&test_file);

        // Cleanup
        let _ = fs::remove_file(&test_file);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_reload_now_no_callback() {
        let config = WatcherConfig {
            enabled: false,
            ..Default::default()
        };

        let watcher = ConfigWatcher::start(vec![], Box::new(|_| Ok(())), config).unwrap();

        // reload_now without callback registered should error
        let result = watcher.reload_now(Path::new("/nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn test_watcher_start_no_paths() {
        let config = WatcherConfig::default();
        let result = ConfigWatcher::start(vec![], Box::new(|_| Ok(())), config);
        assert!(result.is_err());
    }
}
