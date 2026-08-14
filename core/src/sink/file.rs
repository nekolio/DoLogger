//! File Sink — writes formatted log records to files.
//!
//! # Implementation
//!
//! Basic file sink with buffered writes, supporting:
//! - Append mode to a specified file path
//! - Plain text output format
//! - Configurable flush interval
//! - Basic rotation by size (skeleton)
//!
//! # Planned Enhancements
//!
//! - io_uring/IOCP/kqueue async IO
//! - Compression (gzip/zstd)
//! - Full rotation policies (time-based, size-based)
//! - WORM mode with fsync + permission locking

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use crate::sink::{DurabilityLevel, Sink, SinkError, SinkResult};

/// File sink configuration.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct FileSinkConfig {
    /// Path to the output file
    pub path: PathBuf,
    /// Maximum file size before rotation (bytes, 0 = unlimited)
    pub max_size: u64,
    /// Whether to fsync after each write (default: false, AUDIT forces true)
    pub fsync_on_write: bool,
    /// Durability level for writes
    pub durability_level: DurabilityLevel,
    /// Buffer size for BufWriter (default: 64KB)
    pub buffer_size: usize,
}

impl Default for FileSinkConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("dologger_output.log"),
            max_size: 0,
            fsync_on_write: false,
            durability_level: DurabilityLevel::OsCache,
            buffer_size: 65536, // 64KB
        }
    }
}

impl FileSinkConfig {
    /// Validate that the configuration is internally consistent.
    pub fn validate(&self) -> Result<(), String> {
        // If durability_level is Media or MediaWithFua, fsync_on_write should be true
        if self.durability_level >= DurabilityLevel::Media && !self.fsync_on_write {
            return Err(format!(
                "fsync_on_write must be true when durability_level is {:?} (requires per-write fsync)",
                self.durability_level
            ));
        }
        // If fsync_on_write is true but durability_level is Unsafe, that's a mismatch
        if self.fsync_on_write && self.durability_level == DurabilityLevel::Unsafe {
            return Err("fsync_on_write=true is incompatible with DurabilityLevel::Unsafe".into());
        }
        Ok(())
    }
}

/// File sink — writes formatted log records to a file.
pub struct FileSink {
    config: FileSinkConfig,
    writer: Option<BufWriter<File>>,
    bytes_written: u64,
    is_open: bool,
}

impl FileSink {
    /// Create a new FileSink with the given config.
    pub fn new(config: FileSinkConfig) -> Self {
        Self {
            config,
            writer: None,
            bytes_written: 0,
            is_open: false,
        }
    }

    /// Create with a simple file path.
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self::new(FileSinkConfig {
            path: path.into(),
            ..Default::default()
        })
    }

    /// Check if rotation is needed and perform it.
    fn check_rotation(&mut self) -> SinkResult {
        if self.config.max_size > 0 && self.bytes_written >= self.config.max_size {
            self.rotate()?;
        }
        Ok(())
    }

    /// Rotate the current file (rename + create new).
    fn rotate(&mut self) -> SinkResult {
        // Close current writer
        if let Some(writer) = self.writer.take() {
            let mut file = writer
                .into_inner()
                .map_err(|e| SinkError::WriteFailed(format!("Flush during rotation: {e}")))?;
            file.flush()
                .map_err(|e| SinkError::WriteFailed(format!("Flush during rotation: {e}")))?;
        }

        // Rename existing file
        let rotated_path = self.config.path.with_extension(format!(
            "log.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));

        if self.config.path.exists() {
            std::fs::rename(&self.config.path, &rotated_path)
                .map_err(|e| SinkError::WriteFailed(format!("Rotation rename failed: {e}")))?;
        }

        // Open new file
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.path)
            .map_err(|e| SinkError::WriteFailed(format!("Cannot open: {e}")))?;

        self.writer = Some(BufWriter::with_capacity(self.config.buffer_size, file));
        self.bytes_written = 0;

        Ok(())
    }
}

impl Sink for FileSink {
    fn open(&mut self) -> SinkResult {
        // Create parent directory if needed
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
            .map_err(|e| SinkError::WriteFailed(format!("Cannot open: {e}")))?;

        self.writer = Some(BufWriter::with_capacity(self.config.buffer_size, file));
        self.is_open = true;
        self.bytes_written = 0;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        if !self.is_open {
            return Err(SinkError::Closed);
        }

        self.check_rotation()?;

        if let Some(ref mut writer) = self.writer {
            writer
                .write_all(formatted.as_bytes())
                .map_err(|e| SinkError::WriteFailed(format!("write: {e}")))?;
            writer
                .write_all(b"\n")
                .map_err(|e| SinkError::WriteFailed(format!("write newline: {e}")))?;

            self.bytes_written += formatted.len() as u64 + 1;

            // Durability enforcement
            if self.config.durability_level >= DurabilityLevel::Media {
                // Media and MediaWithFua: fsync per write.
                // MediaWithFua would additionally use the FUA (Force Unit Access)
                // flag on platforms that support it (e.g., Windows FILE_FLAG_WRITE_THROUGH,
                // Linux O_DIRECT|O_SYNC, or NVMe FUA bit).
                writer
                    .get_mut()
                    .sync_all()
                    .map_err(|e| SinkError::WriteFailed(format!("fsync: {e}")))?;
            } else if self.config.fsync_on_write {
                // Legacy path: fsync_on_write override at OsCache or Unsafe level
                writer
                    .get_mut()
                    .sync_all()
                    .map_err(|e| SinkError::WriteFailed(format!("fsync: {e}")))?;
            }
        }

        Ok(())
    }

    fn flush(&mut self) -> SinkResult {
        if let Some(ref mut writer) = self.writer {
            writer
                .flush()
                .map_err(|e| SinkError::WriteFailed(format!("flush: {e}")))?;
            if self.config.fsync_on_write {
                writer
                    .get_mut()
                    .sync_all()
                    .map_err(|e| SinkError::WriteFailed(format!("fsync: {e}")))?;
            }
        }
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        self.flush()?;
        // OsCache and above: fsync on close to ensure data reaches OS cache
        if self.config.durability_level >= DurabilityLevel::OsCache {
            if let Some(ref mut writer) = self.writer {
                writer
                    .get_mut()
                    .sync_all()
                    .map_err(|e| SinkError::WriteFailed(format!("fsync on close: {e}")))?;
            }
        }
        self.writer = None;
        self.is_open = false;
        Ok(())
    }
}
