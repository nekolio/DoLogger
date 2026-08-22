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
//! # Backends
//!
//! The watcher selects a platform-native kernel-event backend when available
//! and falls back to polling (mtime comparison) otherwise:
//! - Windows: `ReadDirectoryChangesW` (kernel event stream, ~1ms latency)
//! - Linux: `inotify` (kernel event stream, ~1ms latency)
//! - macOS / other: polling fallback (FSEvents native integration deferred;
//!   revisit once a macOS build target exists)
//!
//! The `WatcherBackend` enum drives dispatch; the polling loop and the native
//! loops share the same debounce + reload + history machinery so behaviour is
//! uniform regardless of backend.
//!
//! # Wiring status — v0.0.1
//!
//! **NOT wired into [`crate::Engine`] at v0.0.1** — this isolation is
//! deliberate. The engine does not reload its configuration automatically,
//! and nothing calls [`ConfigWatcher::start`] outside this module's tests.
//! The watcher is complete and tested, ready to wire behind an explicit,
//! default-off config gate in a later milestone; this note keeps the
//! isolation explicit so it is not mistaken for a live hot-reload path.

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
            // inotify is available on all Linux kernels >= 2.6.13 (2005).
            WatcherBackend::Inotify
        }
        #[cfg(windows)]
        {
            // ReadDirectoryChangesW is available on all supported Windows
            // versions.
            WatcherBackend::ReadDirectoryChanges
        }
        #[cfg(target_os = "macos")]
        {
            // FSEvents native integration is deferred; polling is the
            // portable fallback until a macOS build target exists.
            WatcherBackend::Polling
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
            .spawn(move || match config.backend {
                #[cfg(windows)]
                WatcherBackend::ReadDirectoryChanges => {
                    native_windows::watcher_loop_native_windows(
                        files_clone,
                        reload_clone,
                        shutdown_clone,
                        history_clone,
                        cfg,
                    )
                }
                #[cfg(target_os = "linux")]
                WatcherBackend::Inotify => native_inotify::watcher_loop_native_inotify(
                    files_clone,
                    reload_clone,
                    shutdown_clone,
                    history_clone,
                    cfg,
                ),
                // Polling, deferred backends, and any non-matching variant on
                // a given platform fall through to the mtime polling loop.
                _ => watcher_loop(
                    files_clone,
                    reload_clone,
                    shutdown_clone,
                    history_clone,
                    cfg,
                ),
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
        let now = Instant::now();
        {
            let mut files_guard = files.lock().unwrap();
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
                        }
                    }
                } else {
                    // File doesn't exist — reset mtime tracking
                    watched.last_mtime = None;
                    watched.reload_pending = false;
                }
            }
        }

        flush_pending_reloads(&files, &on_reload, &history, debounce_duration);

        thread::sleep(poll_duration);
    }
}

/// Fire reload callbacks for every watched file whose debounce window has
/// elapsed. Shared by the polling and native loops so reload semantics
/// (keep-old-config-on-failure, history, sysmon logging) are uniform.
///
/// Files are collected under the files lock, then reloaded outside the lock
/// so a slow callback never blocks change detection.
fn flush_pending_reloads(
    files: &Arc<Mutex<Vec<WatchedFile>>>,
    on_reload: &Arc<Mutex<Option<ReloadCallback>>>,
    history: &Arc<Mutex<Vec<ReloadEvent>>>,
    debounce_duration: Duration,
) {
    let now = Instant::now();

    // Collect paths whose debounce has elapsed, under the files lock.
    let mut reload_paths: Vec<PathBuf> = {
        let mut files_guard = files.lock().unwrap();
        let mut ready: Vec<PathBuf> = Vec::new();
        for watched in files_guard.iter_mut() {
            if watched.reload_pending {
                if let Some(changed_at) = watched.last_change_at {
                    if now.duration_since(changed_at) >= debounce_duration {
                        watched.reload_pending = false;
                        ready.push(watched.path.clone());
                    }
                }
            }
        }
        ready
    };

    // Execute reloads outside the lock
    for path in reload_paths.drain(..) {
        let old_mtime = {
            let guard = files.lock().unwrap();
            guard
                .iter()
                .find(|f| f.path == path)
                .and_then(|f| f.last_mtime)
        };

        let new_mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());

        let reload_result = {
            let guard = on_reload.lock().unwrap();
            if let Some(ref cb) = *guard {
                cb(&path)
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
}

// ---------------------------------------------------------------------------
// Windows native backend — ReadDirectoryChangesW
// ---------------------------------------------------------------------------

/// Windows kernel-event watcher via `ReadDirectoryChangesW`.
///
/// Each watched file's parent directory is opened with
/// `FILE_FLAG_OVERLAPPED | FILE_FLAG_BACKUP_SEMANTICS` and a change-notify
/// read is queued against a manual-reset event. The loop waits on all
/// directory events with a short timeout so shutdown stays responsive, parses
/// the returned `FILE_NOTIFY_INFORMATION` stream, and marks any watched file
/// whose name matches as changed. Debounce + reload reuse the shared
/// [`super::flush_pending_reloads`] machinery, so failure semantics and
/// history are identical to the polling path.
#[cfg(windows)]
mod native_windows {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    #[allow(clippy::upper_case_acronyms)]
    type HANDLE = *mut core::ffi::c_void;
    #[allow(clippy::upper_case_acronyms)]
    type DWORD = u32;
    #[allow(clippy::upper_case_acronyms)]
    type BOOL = i32;
    #[allow(clippy::upper_case_acronyms)]
    type LPVOID = *mut core::ffi::c_void;

    const GENERIC_READ: DWORD = 0x8000_0000;
    const FILE_SHARE_READ: DWORD = 0x1;
    const FILE_SHARE_WRITE: DWORD = 0x2;
    const FILE_SHARE_DELETE: DWORD = 0x4;
    const OPEN_EXISTING: DWORD = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: DWORD = 0x0200_0000;
    const FILE_FLAG_OVERLAPPED: DWORD = 0x4000_0000;
    const FILE_NOTIFY_CHANGE_FILE_NAME: DWORD = 0x1;
    const FILE_NOTIFY_CHANGE_LAST_WRITE: DWORD = 0x10;
    const FILE_NOTIFY_CHANGE_CREATION: DWORD = 0x40;
    const ERROR_IO_PENDING: DWORD = 997;
    const WAIT_TIMEOUT: DWORD = 258;
    const DIRECTORY_BUF_SIZE: usize = 64 * 1024;

    /// Overlapped I/O structure — must be exactly the Win32 `OVERLAPPED`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        event: HANDLE,
    }

    /// Directory notify buffer, 8-byte aligned for `FILE_NOTIFY_INFORMATION`.
    #[repr(C, align(8))]
    struct NotifyBuf([u8; DIRECTORY_BUF_SIZE]);

    /// First field of `FILE_NOTIFY_INFORMATION`; the file name is a trailing
    /// variable-length UTF-16 array accessed via raw pointer arithmetic.
    #[repr(C)]
    struct FileNotifyInformation {
        next_entry_offset: DWORD,
        action: DWORD,
        file_name_length: DWORD,
        file_name: [u16; 1],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: DWORD,
            share_mode: DWORD,
            security_attributes: LPVOID,
            creation_disposition: DWORD,
            flags_and_attributes: DWORD,
            template_file: HANDLE,
        ) -> HANDLE;
        fn CreateEventW(
            security_attributes: LPVOID,
            manual_reset: BOOL,
            initial_state: BOOL,
            name: *const u16,
        ) -> HANDLE;
        fn ReadDirectoryChangesW(
            directory: HANDLE,
            buffer: LPVOID,
            buffer_length: DWORD,
            watch_subtree: BOOL,
            notify_filter: DWORD,
            bytes_returned: *mut DWORD,
            overlapped: *mut Overlapped,
            completion_routine: LPVOID,
        ) -> BOOL;
        fn GetOverlappedResult(
            file: HANDLE,
            overlapped: *mut Overlapped,
            bytes_transferred: *mut DWORD,
            wait: BOOL,
        ) -> BOOL;
        fn WaitForMultipleObjects(
            count: DWORD,
            handles: *const HANDLE,
            wait_all: BOOL,
            milliseconds: DWORD,
        ) -> DWORD;
        fn GetLastError() -> DWORD;
        fn CancelIoEx(file: HANDLE, overlapped: *mut Overlapped) -> BOOL;
        fn CloseHandle(hObject: isize) -> i32;
        fn SetEvent(event: HANDLE) -> BOOL;
        fn ResetEvent(event: HANDLE) -> BOOL;
    }

    /// State for one watched directory: its notify handle, event, overlapped
    /// struct, and buffer. The overlapped struct has a stable address for the
    /// lifetime of the watch so it can be reused across reissues.
    struct DirWatch {
        handle: HANDLE,
        event: HANDLE,
        overlapped: Overlapped,
        buffer: NotifyBuf,
    }

    impl DirWatch {
        /// Queue a change-notify read. Returns false if the directory cannot
        /// be watched (unlikely after a successful open).
        fn issue_read(&mut self) -> bool {
            // The OVERLAPPED must keep a stable address for the lifetime of
            // the pending read, so the OS's pointer to it stays valid. It is
            // stored in `self`, not on the stack.
            self.overlapped.event = self.event;
            let mut bytes: DWORD = 0;
            // SAFETY: `self` owns a valid directory handle and the aligned
            // buffer; `self.overlapped` has its event set and a stable address.
            let rc = unsafe {
                ReadDirectoryChangesW(
                    self.handle,
                    self.buffer.0.as_mut_ptr() as LPVOID,
                    DIRECTORY_BUF_SIZE as DWORD,
                    0, // watch subtree only — files directly in this directory
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_CREATION,
                    &mut bytes,
                    &mut self.overlapped,
                    core::ptr::null_mut(),
                )
            };
            if rc != 0 {
                true
            } else {
                // SAFETY: `GetLastError` has no caller invariants.
                let last_error = unsafe { GetLastError() };
                last_error == ERROR_IO_PENDING
            }
        }

        /// Collect the completed read's byte count, if the I/O completed.
        fn completed_bytes(&mut self) -> Option<DWORD> {
            let mut bytes: DWORD = 0;
            // SAFETY: `self.handle` and `self.overlapped` are the values
            // registered for the outstanding read; `wait` is 0 (non-blocking).
            let ok =
                unsafe { GetOverlappedResult(self.handle, &mut self.overlapped, &mut bytes, 0) };
            if ok != 0 {
                Some(bytes)
            } else {
                None
            }
        }
    }

    pub(super) fn watcher_loop_native_windows(
        files: Arc<Mutex<Vec<WatchedFile>>>,
        on_reload: Arc<Mutex<Option<ReloadCallback>>>,
        shutdown: Arc<AtomicBool>,
        history: Arc<Mutex<Vec<ReloadEvent>>>,
        config: WatcherConfig,
    ) {
        let debounce = Duration::from_millis(config.debounce_ms);
        // Short wait keeps the loop responsive to shutdown.
        let timeout_ms: DWORD = 100;

        // Collect the set of unique parent directories being watched.
        let dirs: Vec<PathBuf> = {
            let guard = files.lock().unwrap();
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for f in guard.iter() {
                if let Some(dir) = f.path.parent() {
                    if seen.insert(dir.to_path_buf()) {
                        out.push(dir.to_path_buf());
                    }
                }
            }
            out
        };

        let mut watches: Vec<DirWatch> = Vec::new();
        // SAFETY: only Win32 API calls with correctly-typed pointers; all
        // returned handles are stored for release, and buffers are aligned.
        unsafe {
            for dir in &dirs {
                let handle = open_directory(dir);
                if handle.is_null() {
                    continue;
                }
                let event = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
                if event.is_null() {
                    CloseHandle(handle as isize);
                    continue;
                }
                let mut watch = DirWatch {
                    handle,
                    event,
                    overlapped: Overlapped {
                        internal: 0,
                        internal_high: 0,
                        offset: 0,
                        offset_high: 0,
                        event,
                    },
                    buffer: NotifyBuf([0u8; DIRECTORY_BUF_SIZE]),
                };
                if !watch.issue_read() {
                    CloseHandle(event as isize);
                    CloseHandle(handle as isize);
                    continue;
                }
                watches.push(watch);
            }
        }

        // If no directory opened, degrade to the polling loop for this run so
        // change detection is never silently disabled.
        if watches.is_empty() {
            super::watcher_loop(files, on_reload, shutdown, history, config);
            return;
        }

        let handles: Vec<HANDLE> = watches.iter().map(|w| w.event).collect();

        loop {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            // SAFETY: `handles` is a stable Vec of valid event handles and
            // `timeout_ms` is finite, so the call returns promptly.
            let wait_rc = unsafe {
                WaitForMultipleObjects(handles.len() as DWORD, handles.as_ptr(), 0, timeout_ms)
            };

            if wait_rc == WAIT_TIMEOUT {
                // No event in this window; give pending debounce a chance to
                // elapse so a lone change still triggers a reload.
                flush_pending_reloads(&files, &on_reload, &history, debounce);
                continue;
            }

            if wait_rc < watches.len() as DWORD {
                let idx = wait_rc as usize;
                // SAFETY: each signalled watch has a valid event handle.
                unsafe { ResetEvent(watches[idx].event) };
                let bytes = watches[idx].completed_bytes();
                if let Some(bytes) = bytes {
                    // `bytes` is the number of bytes the OS wrote into the
                    // watch's own aligned buffer.
                    process_notify(&watches[idx], &files, bytes);
                }
                // Reissue the read so the directory stays armed regardless of
                // parse outcome (handles ERROR_NOTIFY_ENUM_DIR / overflow).
                watches[idx].issue_read();
            }

            flush_pending_reloads(&files, &on_reload, &history, debounce);
        }

        // SAFETY: cancel outstanding I/O then release every handle.
        unsafe {
            for w in &watches {
                CancelIoEx(w.handle, core::ptr::null_mut());
                SetEvent(w.event);
                CloseHandle(w.handle as isize);
                CloseHandle(w.event as isize);
            }
        }
    }

    /// Open a directory handle for change notification. Requires
    /// `FILE_FLAG_BACKUP_SEMANTICS` to open a directory with `CreateFileW`.
    fn open_directory(dir: &Path) -> HANDLE {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 path; all other arguments
        // are either valid flags or null.
        unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                core::ptr::null_mut(),
            )
        }
    }

    /// Walk a `FILE_NOTIFY_INFORMATION` stream and mark every watched file in
    /// this directory whose name appears as changed.
    fn process_notify(watch: &DirWatch, files: &Arc<Mutex<Vec<WatchedFile>>>, bytes: DWORD) {
        const HEADER: usize = 12; // next_entry_offset + action + file_name_length
        let base = watch.buffer.0.as_ptr();
        let mut offset: usize = 0;
        while offset + HEADER <= bytes as usize {
            // SAFETY: `offset + HEADER` is within `bytes`, which the OS wrote
            // into `watch.buffer`; the record is therefore in-bounds and the
            // first three fields (all u32) are aligned.
            let rec = unsafe { &*((base.add(offset)) as *const FileNotifyInformation) };
            let name_bytes = rec.file_name_length as usize;
            // Bounds-check before dereferencing the variable-length name.
            if offset + HEADER + name_bytes > bytes as usize {
                break;
            }
            let name_len = name_bytes / 2;
            // SAFETY: the name slice of `name_len` u16s lies within the
            // verified `bytes` extent of the aligned buffer.
            let name = unsafe {
                OsString::from_wide(core::slice::from_raw_parts(
                    rec.file_name.as_ptr(),
                    name_len,
                ))
            };

            {
                let mut guard = files.lock().unwrap();
                for f in guard.iter_mut() {
                    if let Some(fname) = f.path.file_name() {
                        if fname == name.as_os_str() {
                            f.last_change_at = Some(Instant::now());
                            f.reload_pending = true;
                            f.last_mtime =
                                fs::metadata(&f.path).ok().and_then(|m| m.modified().ok());
                        }
                    }
                }
            }

            if rec.next_entry_offset == 0 {
                break;
            }
            offset += rec.next_entry_offset as usize;
        }
    }
}

// ---------------------------------------------------------------------------
// Linux native backend — inotify
// ---------------------------------------------------------------------------

/// Linux kernel-event watcher via `inotify`.
///
/// Each watched file is registered with `inotify_add_watch` for
/// modify/close-write/moved-to events. The loop polls the inotify descriptor
/// with a short timeout so shutdown stays responsive, parses the returned
/// `inotify_event` stream (matched by watch descriptor), and marks the
/// corresponding watched file as changed. Debounce + reload reuse the shared
/// [`super::flush_pending_reloads`] machinery.
#[cfg(target_os = "linux")]
mod native_inotify {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    /// A single file registered with the inotify descriptor.
    struct FileWatch {
        wd: i32,
        path: PathBuf,
    }

    /// `inotify_event` header. The file name is a trailing flexible array;
    /// not used here because events are matched by watch descriptor.
    #[repr(C)]
    struct InotifyEvent {
        wd: i32,
        mask: u32,
        cookie: u32,
        len: u32,
    }

    /// Read buffer, 4-byte aligned so `inotify_event` fields are aligned.
    #[repr(C, align(4))]
    struct InotifyBuf([u8; 4096]);

    pub(super) fn watcher_loop_native_inotify(
        files: Arc<Mutex<Vec<WatchedFile>>>,
        on_reload: Arc<Mutex<Option<ReloadCallback>>>,
        shutdown: Arc<AtomicBool>,
        history: Arc<Mutex<Vec<ReloadEvent>>>,
        config: WatcherConfig,
    ) {
        let debounce = Duration::from_millis(config.debounce_ms);

        // SAFETY: `inotify_init1` returns a valid fd or -1; non-blocking mode
        // is set so a poll/read cycle never blocks the loop.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK) };
        if fd < 0 {
            // Descriptor unavailable — degrade to the polling loop so change
            // detection is never silently disabled.
            super::watcher_loop(files, on_reload, shutdown, history, config);
            return;
        }

        // Register a watch on each watched file.
        let mut watches: Vec<FileWatch> = Vec::new();
        {
            let guard = files.lock().unwrap();
            for f in guard.iter() {
                let Ok(c_path) = CString::new(f.path.as_os_str().as_bytes()) else {
                    continue; // path contains an interior NUL — cannot watch
                };
                // SAFETY: `c_path` is a NUL-terminated byte path and `fd` is a
                // valid inotify descriptor.
                let wd = unsafe {
                    libc::inotify_add_watch(
                        fd,
                        c_path.as_ptr(),
                        libc::IN_MODIFY | libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO,
                    )
                };
                if wd >= 0 {
                    watches.push(FileWatch {
                        wd,
                        path: f.path.clone(),
                    });
                }
            }
        }

        if watches.is_empty() {
            // SAFETY: `fd` was successfully opened and no longer referenced.
            unsafe { libc::close(fd) };
            super::watcher_loop(files, on_reload, shutdown, history, config);
            return;
        }

        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN as i16,
            revents: 0,
        };
        let mut buf = InotifyBuf([0u8; 4096]);

        loop {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            // SAFETY: `pollfd` points to a valid stack struct; timeout is 100ms.
            let rc = unsafe { libc::poll(&mut pollfd, 1, 100) };
            if rc < 0 {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            if rc == 0 || pollfd.revents & libc::POLLIN as i16 == 0 {
                flush_pending_reloads(&files, &on_reload, &history, debounce);
                continue;
            }
            // Drain all pending events for this poll.
            loop {
                // SAFETY: `buf` is a writable aligned buffer of `buf.len()`
                // bytes; `fd` is non-blocking so this returns promptly.
                let n =
                    unsafe { libc::read(fd, buf.0.as_mut_ptr() as *mut libc::c_void, buf.0.len()) };
                if n <= 0 {
                    break;
                }
                process_inotify(&buf.0[..n as usize], &watches, &files);
            }
            flush_pending_reloads(&files, &on_reload, &history, debounce);
        }

        // SAFETY: all watches are removed and the descriptor closed; no other
        // reference to `fd` remains.
        unsafe {
            for w in &watches {
                libc::inotify_rm_watch(fd, w.wd);
            }
            libc::close(fd);
        }
    }

    /// Walk an `inotify_event` stream and mark every watched file whose watch
    /// descriptor appears as changed.
    fn process_inotify(data: &[u8], watches: &[FileWatch], files: &Arc<Mutex<Vec<WatchedFile>>>) {
        let mut offset = 0usize;
        while offset + std::mem::size_of::<InotifyEvent>() <= data.len() {
            // SAFETY: `offset` stays within `data` and the buffer is 4-byte
            // aligned, so the header fields are valid.
            let ev = unsafe { &*(data.as_ptr().add(offset) as *const InotifyEvent) };
            if let Some(w) = watches.iter().find(|w| w.wd == ev.wd) {
                let mut guard = files.lock().unwrap();
                for f in guard.iter_mut() {
                    if f.path == w.path {
                        f.last_change_at = Some(Instant::now());
                        f.reload_pending = true;
                        f.last_mtime = fs::metadata(&f.path).ok().and_then(|m| m.modified().ok());
                    }
                }
            }
            let name_len = ev.len as usize;
            offset += std::mem::size_of::<InotifyEvent>() + name_len;
        }
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

    /// End-to-end check of the Windows native `ReadDirectoryChangesW` path:
    /// modify a watched file and assert the reload callback fires.
    #[cfg(windows)]
    #[test]
    fn test_native_windows_backend_reload() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let temp_dir = std::env::temp_dir().join("dologger_test_rdcw");
        let _ = fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join("config.toml");
        fs::write(&test_file, "[dologger]\nlevel = \"DEBUG\"\n").unwrap();

        let reload_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&reload_count);
        let config = WatcherConfig {
            backend: WatcherBackend::ReadDirectoryChanges,
            poll_interval_ms: 1000,
            debounce_ms: 200,
            enabled: true,
        };

        let watcher = ConfigWatcher::start(
            vec![test_file.clone()],
            Box::new(move |_| {
                count_clone.fetch_add(1, AtomicOrdering::SeqCst);
                Ok(())
            }),
            config,
        )
        .unwrap();

        // Wait for the native thread to arm its directory watch. RDCW only
        // reports changes after the read is armed, so a write before this
        // point would be missed.
        thread::sleep(Duration::from_millis(300));

        // Trigger a change.
        fs::write(&test_file, "[dologger]\nlevel = \"INFO\"\n").unwrap();

        // Wait for the reload callback to fire.
        let deadline = Instant::now() + Duration::from_secs(5);
        while reload_count.load(AtomicOrdering::SeqCst) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }

        let mut watcher = watcher;
        watcher.shutdown();

        assert!(
            reload_count.load(AtomicOrdering::SeqCst) > 0,
            "native RDCW reload never fired"
        );

        let _ = fs::remove_file(&test_file);
        let _ = fs::remove_dir(&temp_dir);
    }
}
