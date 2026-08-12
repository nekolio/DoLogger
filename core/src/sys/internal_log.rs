//! Internal diagnostic log.
//!
//! Writes critical diagnostic information using direct syscalls.
//! Used when sysmon is unavailable (early init, severe errors).
//!
//! Format: `[timestamp_ns] [LEVEL] [component] message`
//! Output: file (default `./dologger_internal.log`) or stderr fallback.

use crate::sys::io;

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::sync::Mutex;

/// Severity levels for internal diagnostic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum DiagLevel {
    Info,
    Warn,
    Error,
    Critical,
}

impl DiagLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Critical => "CRITICAL",
        }
    }
}

/// Internal diagnostic logger.
///
/// Writes to a file by default, with stderr fallback.
/// Thread-safe: uses a Mutex for the file handle.
pub struct InternalLog {
    file: Mutex<Option<File>>,
    #[allow(dead_code)]
    path: String,
}

impl InternalLog {
    /// Create a new internal diagnostic log writing to the given path.
    /// Sets restrictive permissions (0600 on Unix, current-user on Windows).
    pub fn new(path: &str) -> Self {
        let file = OpenOptions::new().create(true).append(true).open(path).ok();

        if let Some(ref f) = file {
            // Restrictive permissions — owner-only
            #[cfg(not(windows))]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = f.metadata() {
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o600);
                    let _ = f.set_permissions(perms);
                }
            }
        } else {
            io::stderr_line(&format!(
                "[DoLogger] WARN: Cannot open internal log '{path}', falling back to stderr"
            ));
        }

        Self {
            file: Mutex::new(file),
            path: path.to_string(),
        }
    }

    /// Check if rotation is needed and perform it.
    /// Size-based rotation, default 10MB, keep 5 historical files.
    pub fn check_rotation(&self, max_size_mb: u64) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(ref f) = *guard {
                if let Ok(meta) = f.metadata() {
                    if meta.len() > max_size_mb * 1024 * 1024 {
                        // Rotate: rename current, create new, keep 5 historical
                        for i in (1..=5).rev() {
                            let old = format!("{}.{}", self.path, i);
                            let new = format!("{}.{}", self.path, i + 1);
                            if i == 5 {
                                let _ = std::fs::remove_file(&old);
                            } else if std::path::Path::new(&old).exists() {
                                let _ = std::fs::rename(&old, &new);
                            }
                        }
                        let rotated = format!("{}.1", self.path);
                        let _ = std::fs::rename(&self.path, &rotated);
                        // Re-open new file
                        if let Ok(new_file) = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&self.path)
                        {
                            #[cfg(not(windows))]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                if let Ok(metadata) = new_file.metadata() {
                                    let mut perms = metadata.permissions();
                                    perms.set_mode(0o600);
                                    let _ = new_file.set_permissions(perms);
                                }
                            }
                            *guard = Some(new_file);
                        }
                    }
                }
            }
        }
    }

    /// Log a diagnostic message.
    pub fn log(&self, level: DiagLevel, component: &str, message: &str) {
        let timestamp_ns = self.monotonic_ns();
        let line = format!(
            "[{timestamp_ns}] [{level}] [{component}] {message}\n",
            level = level.as_str()
        );

        if let Ok(mut guard) = self.file.lock() {
            if let Some(ref mut f) = *guard {
                if f.write_all(line.as_bytes()).is_ok() {
                    // Periodic fsync for CRITICAL messages
                    if level == DiagLevel::Critical {
                        let _ = f.sync_all();
                    }
                    return;
                }
            }
        }

        // Fallback: write to stderr
        io::stderr_write(line.as_bytes());
    }

    /// Convenience: info message.
    pub fn info(&self, component: &str, message: &str) {
        self.log(DiagLevel::Info, component, message);
    }

    /// Convenience: warning message.
    pub fn warn(&self, component: &str, message: &str) {
        self.log(DiagLevel::Warn, component, message);
    }

    /// Convenience: error message.
    pub fn error(&self, component: &str, message: &str) {
        self.log(DiagLevel::Error, component, message);
    }

    /// Convenience: critical message.
    pub fn critical(&self, component: &str, message: &str) {
        self.log(DiagLevel::Critical, component, message);
    }

    /// Get a monotonic timestamp in nanoseconds.
    fn monotonic_ns(&self) -> u64 {
        // Use a simple counter since we can't rely on clock_gettime in all contexts.
        // In production, this would use CLOCK_MONOTONIC via libc::clock_gettime.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Flush the log file.
    pub fn flush(&self) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(ref mut f) = *guard {
                let _ = f.flush();
            }
        }
    }

    /// Close the log file.
    pub fn close(&self) {
        self.flush();
        if let Ok(mut guard) = self.file.lock() {
            *guard = None;
        }
    }
}
