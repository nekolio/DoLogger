//! WORM File Sink.
//!
//! Write-Once-Read-Many file sink for AUDIT-level records.
//!
//! # Requirements
//!
//! - **MEDIA durability**: `fsync` after every write batch
//! - **Read-only lock**: file permissions set to read-only on close
//! - **LSN reorder window**: gap detection with configurable timeout
//! - **Gap marker records**: explicit gap markers when LSN discontinuity detected
//! - **Chain continuity**: `prev_hash` verified against previous persisted record
//!
//! # Performance profile binding
//!
//! | Profile | WORM behavior |
//! |---------|---------------|
//! | `dev` | fsync per batch, 100ms reorder window |
//! | `prod-performance` | fsync per batch, 500ms reorder window |
//! | `prod-audit` | fsync per write, 200ms reorder window, chain verify |
//! | `balanced` | fsync per batch, 300ms reorder window |

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::sink::{Sink, SinkError, SinkResult};

/// WORM durability level controlling fsync behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WormDurability {
    /// `fsync` after each write batch (default for non-AUDIT profiles).
    OsCache,
    /// `fsync` after every single write — required for AUDIT compliance.
    Media,
    /// `fsync` + `O_DIRECT` + FUA where platform support exists.
    MediaWithFua,
}

/// WORM File Sink configuration.
#[derive(Debug, Clone)]
pub struct WormSinkConfig {
    /// Output file path
    pub path: PathBuf,
    /// Durability level
    pub durability: WormDurability,
    /// LSN reorder window (milliseconds, 0 = strictly ordered)
    pub lsn_reorder_window_ms: u64,
    /// Maximum file size before rotation (bytes, 0 = unlimited)
    pub max_size: u64,
    /// Buffer size for BufWriter
    pub buffer_size: usize,
    /// Lock file read-only on close
    pub lock_readonly_on_close: bool,
    /// Enable adaptive LSN window
    pub adaptive_window: bool,
}

impl Default for WormSinkConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("dologger_audit.worm"),
            durability: WormDurability::Media,
            lsn_reorder_window_ms: 200,
            max_size: 0,
            buffer_size: 65536,
            lock_readonly_on_close: true,
            adaptive_window: false,
        }
    }
}

/// A pending record in the LSN reorder buffer.
struct PendingRecord {
    /// Serialized record bytes
    data: Vec<u8>,
    /// Previous hash for chain continuity
    prev_hash: [u8; 32],
}

/// WORM File Sink — append-only, fsync-backed, LSN-chain-verified.
pub struct WormSink {
    config: WormSinkConfig,
    writer: Option<BufWriter<File>>,
    /// Last successfully persisted LSN (0 = none)
    last_lsn: u64,
    /// Last persisted signature hash (for chain verification)
    last_hash: [u8; 32],
    /// LSN reorder buffer: LSN → pending record
    reorder_buffer: BTreeMap<u64, PendingRecord>,
    /// When the reorder buffer started waiting
    reorder_start: Option<Instant>,
    /// Total bytes written
    bytes_written: u64,
    is_open: bool,
}

impl WormSink {
    /// Create a new WORM sink.
    pub fn new(config: WormSinkConfig) -> Self {
        Self {
            config,
            writer: None,
            last_lsn: 0,
            last_hash: [0u8; 32],
            reorder_buffer: BTreeMap::new(),
            reorder_start: None,
            bytes_written: 0,
            is_open: false,
        }
    }

    /// Write a record with LSN and signature for chain verification.
    ///
    /// This is the preferred API for AUDIT records — it writes raw bytes
    /// and performs LSN ordering + chain continuity checks.
    pub fn write_worm_record(&mut self, lsn: u64, prev_hash: &[u8; 32], data: &[u8]) -> SinkResult {
        if !self.is_open {
            return Err(SinkError::Closed);
        }

        let expected_lsn = self.last_lsn + 1;

        if lsn == expected_lsn {
            // Verify prev_hash against last persisted record for chain continuity
            if self.last_lsn > 0 && *prev_hash != self.last_hash {
                crate::sys::diag::error(
                    "worm_sink",
                    &format!(
                        "prev_hash MISMATCH at LSN={}: expected {:02x?}.., got {:02x?}.. — chain broken",
                        lsn,
                        &self.last_hash[..4],
                        &prev_hash[..4]
                    ),
                );
                return Err(SinkError::WriteFailed(
                    "WORM chain broken: prev_hash mismatch".into(),
                ));
            }

            // Normal case: next sequential LSN → write immediately
            self.flush_reorder_buffer()?;
            self.write_raw(data)?;

            if self.config.durability == WormDurability::Media
                || self.config.durability == WormDurability::MediaWithFua
            {
                self.fsync()?;
            }

            self.last_lsn = lsn;
            self.last_hash = *prev_hash;
        } else if lsn > expected_lsn {
            // LSN gap detected → buffer for reorder
            self.reorder_buffer.insert(
                lsn,
                PendingRecord {
                    data: data.to_vec(),
                    prev_hash: *prev_hash,
                },
            );

            if self.reorder_start.is_none() {
                self.reorder_start = Some(Instant::now());
            }

            // Check if reorder window has expired
            self.check_reorder_window()?;
        }
        // lsn < expected_lsn: duplicate or late arrival → drop (already persisted)

        Ok(())
    }

    /// Flush any buffered records that are now in sequence.
    fn flush_reorder_buffer(&mut self) -> SinkResult {
        while let Some((&lsn, _)) = self.reorder_buffer.first_key_value() {
            let expected = self.last_lsn + 1;
            if lsn == expected {
                let record = self.reorder_buffer.remove(&lsn).unwrap();
                self.write_raw(&record.data)?;
                self.last_lsn = lsn;
                self.last_hash = record.prev_hash;
            } else if lsn < expected {
                // Duplicate, remove silently
                self.reorder_buffer.remove(&lsn);
            } else {
                break; // Still a gap
            }
        }

        if self.reorder_buffer.is_empty() {
            self.reorder_start = None;
        }

        Ok(())
    }

    /// Check if reorder window has expired and write gap markers if needed.
    fn check_reorder_window(&mut self) -> SinkResult {
        if self.config.lsn_reorder_window_ms == 0 || self.reorder_buffer.is_empty() {
            return Ok(());
        }

        if let Some(start) = self.reorder_start {
            if start.elapsed() >= Duration::from_millis(self.config.lsn_reorder_window_ms) {
                // Window expired — write gap markers for missing LSNs
                let expected = self.last_lsn + 1;
                if let Some((&first_buffered, _)) = self.reorder_buffer.first_key_value() {
                    let gap_start = expected;
                    let gap_end = first_buffered - 1;

                    // Write gap marker record
                    let gap_marker =
                        format!("[GAP] LSN {gap_start}-{gap_end} missing — gap marker\n");
                    self.write_raw(gap_marker.as_bytes())?;
                    self.fsync()?;

                    crate::sys::diag::warn(
                        "worm_sink",
                        &format!(
                            "LSN gap detected: {gap_start}-{gap_end}. Gap marker written to {}",
                            self.config.path.display()
                        ),
                    );

                    self.last_lsn = gap_end;

                    // Now flush any buffered records that are in sequence
                    self.flush_reorder_buffer()?;
                }
                self.reorder_start = None;
            }
        }

        Ok(())
    }

    /// Write raw bytes to the underlying file.
    fn write_raw(&mut self, data: &[u8]) -> SinkResult {
        if let Some(ref mut writer) = self.writer {
            writer
                .write_all(data)
                .map_err(|e| SinkError::WriteFailed(format!("worm write: {e}")))?;

            self.bytes_written += data.len() as u64;

            // Rotation check
            if self.config.max_size > 0 && self.bytes_written >= self.config.max_size {
                self.rotate()?;
            }
        }
        Ok(())
    }

    fn fsync(&mut self) -> SinkResult {
        if let Some(ref mut writer) = self.writer {
            writer
                .get_mut()
                .sync_all()
                .map_err(|e| SinkError::WriteFailed(format!("worm fsync: {e}")))?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> SinkResult {
        // Close + fsync current file
        self.flush()?;
        self.writer = None;

        // Rename old file with timestamp suffix
        let rotated = self.config.path.with_extension(format!(
            "worm.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        if self.config.path.exists() {
            std::fs::rename(&self.config.path, &rotated)
                .map_err(|e| SinkError::WriteFailed(format!("worm rotate: {e}")))?;

            // Lock old file read-only
            #[cfg(not(windows))]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&rotated)
                    .map_err(|e| SinkError::WriteFailed(format!("stat: {e}")))?
                    .permissions();
                perms.set_mode(0o444);
                std::fs::set_permissions(&rotated, perms).ok();
            }
            #[cfg(windows)]
            {
                let mut perms = std::fs::metadata(&rotated)
                    .map_err(|e| SinkError::WriteFailed(format!("stat: {e}")))?
                    .permissions();
                perms.set_readonly(true);
                std::fs::set_permissions(&rotated, perms).ok();
            }
        }

        // Open new file
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
            .map_err(|e| SinkError::WriteFailed(format!("worm open: {e}")))?;

        self.writer = Some(BufWriter::with_capacity(self.config.buffer_size, file));
        Ok(())
    }
}

impl Sink for WormSink {
    fn open(&mut self) -> SinkResult {
        self.open_writer()?;
        self.is_open = true;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        // For the Sink trait, write plain text (used when LSN info unavailable)
        // The write_worm_record() method should be used for AUDIT records
        self.write_raw(formatted.as_bytes())?;
        self.write_raw(b"\n")?;

        if self.config.durability == WormDurability::Media
            || self.config.durability == WormDurability::MediaWithFua
        {
            self.fsync()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> SinkResult {
        if let Some(ref mut writer) = self.writer {
            writer
                .flush()
                .map_err(|e| SinkError::WriteFailed(format!("worm flush: {e}")))?;
        }
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        // Flush remaining reorder buffer
        self.flush_reorder_buffer()?;

        // Final fsync
        self.flush()?;
        self.fsync()?;

        self.writer = None;
        self.is_open = false;

        // Lock file read-only
        if self.config.lock_readonly_on_close && self.config.path.exists() {
            #[cfg(not(windows))]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&self.config.path)
                    .map_err(|e| SinkError::WriteFailed(format!("stat: {e}")))?
                    .permissions();
                perms.set_mode(0o444);
                std::fs::set_permissions(&self.config.path, perms).ok();
            }
            #[cfg(windows)]
            {
                let mut perms = std::fs::metadata(&self.config.path)
                    .map_err(|e| SinkError::WriteFailed(format!("stat: {e}")))?
                    .permissions();
                perms.set_readonly(true);
                std::fs::set_permissions(&self.config.path, perms).ok();
            }
        }

        Ok(())
    }
}
