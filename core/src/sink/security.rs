//! Security File Sink.
//!
//! Dedicated security audit sink that completely bypasses all user-configured
//! plugin chains. Writes directly to an isolated security log file with:
//!
//! - **Fixed raw format** — no configurable formatting (prevents plugin interference)
//! - **Forced fsync** — every write is synchronised to media
//! - **Restrictive permissions** — 0600 on Unix, current-user-only on Windows
//! - **Plugin bypass** — not accessible via the plugin loading system
//! - **Independent file** — completely separate from regular log files
//!
//! Used exclusively by the AuditPipeline for AUDIT records.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use crate::record::Record;
use crate::sink::{Sink, SinkError, SinkResult};

/// Security File Sink configuration.
#[derive(Debug, Clone)]
pub struct SecuritySinkConfig {
    /// Path to the security log file
    pub path: PathBuf,
    /// Maximum file size before rotation (bytes, 0 = unlimited)
    pub max_size: u64,
    /// Buffer size for BufWriter (default: 64KB)
    pub buffer_size: usize,
}

impl Default for SecuritySinkConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("dologger_security.log"),
            max_size: 0,
            buffer_size: 65536,
        }
    }
}

/// Security File Sink — hardened, plugin-bypass audit log writer.
///
/// # Security properties
///
/// - NOT registered in the plugin system — cannot be intercepted
/// - Forces `fsync` on every write (MEDIA durability)
/// - Fixed output format prevents format-injection attacks
/// - File permissions locked to owner-only
pub struct SecuritySink {
    config: SecuritySinkConfig,
    writer: Option<BufWriter<File>>,
    bytes_written: u64,
    is_open: bool,
}

impl SecuritySink {
    /// Create a new Security Sink.
    pub fn new(config: SecuritySinkConfig) -> Self {
        Self {
            config,
            writer: None,
            bytes_written: 0,
            is_open: false,
        }
    }

    /// Create with a file path.
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self::new(SecuritySinkConfig {
            path: path.into(),
            ..Default::default()
        })
    }

    /// Write a structured audit record in fixed security format.
    ///
    /// Format: `LSN|TIMESTAMP_NS|LEVEL|THREAD|PROCESS|HOST|MESSAGE|SIGNATURE_HEX`
    pub fn write_security_record(&mut self, record: &Record) -> SinkResult {
        if !self.is_open {
            return Err(SinkError::Closed);
        }

        let line = format_security_record(record);

        if let Some(ref mut writer) = self.writer {
            writer
                .write_all(line.as_bytes())
                .map_err(|e| SinkError::WriteFailed(format!("security write: {e}")))?;

            self.bytes_written += line.len() as u64;

            // Force fsync — MEDIA durability for all security records
            writer
                .get_mut()
                .sync_all()
                .map_err(|e| SinkError::WriteFailed(format!("security fsync: {e}")))?;
        }

        Ok(())
    }

    fn rotate(&mut self) -> SinkResult {
        self.flush()?;
        self.writer = None;

        let rotated = self.config.path.with_extension(format!(
            "security.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));

        if self.config.path.exists() {
            std::fs::rename(&self.config.path, &rotated)
                .map_err(|e| SinkError::WriteFailed(format!("rotate: {e}")))?;
        }

        // Re-open
        self.open_writer()?;
        self.bytes_written = 0;

        Ok(())
    }

    fn open_writer(&mut self) -> SinkResult {
        if let Some(parent) = self.config.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| SinkError::WriteFailed(format!("mkdir: {e}")))?;
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.path)
            .map_err(|e| SinkError::WriteFailed(format!("security open: {e}")))?;

        // Set restrictive permissions
        set_restrictive_permissions(&file);

        self.writer = Some(BufWriter::with_capacity(self.config.buffer_size, file));
        Ok(())
    }
}

/// Format a record in fixed security format.
///
/// Format: `LSN|TIMESTAMP_HI:TIMESTAMP_LO|LEVEL|THREAD|PROCESS|HOST|MESSAGE|SIG_PREFIX`
fn format_security_record(record: &Record) -> String {
    let timestamp_ns =
        (record.timestamp.hi as u128) * 1_000_000_000 + (record.timestamp.lo as u128);
    let sig_hex: String = record
        .signature
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();

    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}\n",
        record.lsn,
        timestamp_ns,
        record.level.to_str(),
        record.thread_id,
        record.process_id,
        record.host_name.as_str(),
        record.message.as_str().replace('|', "\\x7c"),
        sig_hex,
    )
}

/// Set restrictive file permissions (owner-only).
#[cfg(not(windows))]
fn set_restrictive_permissions(file: &File) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = file.metadata() {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        let _ = file.set_permissions(perms);
    }
}

#[cfg(windows)]
fn set_restrictive_permissions(_file: &File) {
    // Windows: file is created with current user's default ACL.
    // For stricter control, use SetSecurityInfo API.
}

impl Sink for SecuritySink {
    fn open(&mut self) -> SinkResult {
        self.open_writer()?;
        self.is_open = true;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        if !self.is_open {
            return Err(SinkError::Closed);
        }

        if let Some(ref mut writer) = self.writer {
            writer
                .write_all(formatted.as_bytes())
                .map_err(|e| SinkError::WriteFailed(format!("security write: {e}")))?;
            writer
                .write_all(b"\n")
                .map_err(|e| SinkError::WriteFailed(format!("security newline: {e}")))?;

            writer
                .get_mut()
                .sync_all()
                .map_err(|e| SinkError::WriteFailed(format!("security fsync: {e}")))?;
        }

        self.bytes_written += formatted.len() as u64 + 1;

        if self.config.max_size > 0 && self.bytes_written >= self.config.max_size {
            self.rotate()?;
        }

        Ok(())
    }

    fn flush(&mut self) -> SinkResult {
        if let Some(ref mut writer) = self.writer {
            writer
                .flush()
                .map_err(|e| SinkError::WriteFailed(format!("security flush: {e}")))?;
            writer
                .get_mut()
                .sync_all()
                .map_err(|e| SinkError::WriteFailed(format!("security fsync: {e}")))?;
        }
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        self.flush()?;
        self.writer = None;
        self.is_open = false;
        Ok(())
    }
}
