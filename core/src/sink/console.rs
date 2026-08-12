//! Sink trait and built-in Sink implementations.
//!
//! # Sink Trait
//!
//! A Sink is the final stage in the DoLogger pipeline — it receives
//! formatted log records and writes them to an output destination.
//!
//! # M1 Built-in Sinks
//!
//! - **ConsoleSink**: Writes plain text logs to stdout/stderr.
//! - M2+: File, Callback, Kafka, Syslog, Webhook, SQLite, etc.

use crate::record::Record;
use crate::sys::io;

/// Result type for Sink operations.
pub type SinkResult = Result<(), SinkError>;

/// Error type for Sink operations.
#[derive(Debug)]
pub enum SinkError {
    /// Write operation failed
    WriteFailed(String),
    /// Sink is closed
    Closed,
}

impl std::fmt::Display for SinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WriteFailed(msg) => write!(f, "Sink write failed: {msg}"),
            Self::Closed => write!(f, "Sink is closed"),
        }
    }
}

/// Durability level for sink writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurabilityLevel {
    /// No durability guarantee — data may be lost on crash
    Unsafe = 0,
    /// OS cache flush only (fsync on close)
    OsCache = 1,
    /// Media flush (fsync/fdatasync per write)
    Media = 2,
    /// Media flush with Force Unit Access (fsync + FUA flag)
    MediaWithFua = 3,
}

/// The Sink trait — implemented by all output destinations.
///
/// # Lifecycle
///
/// 1. `open()` — Called once when the sink is initialized
/// 2. `write()` — Called for each record (or `write_batch()` if implemented)
/// 3. `flush()` — Called periodically and during shutdown
/// 4. `close()` — Called once during shutdown
pub trait Sink: Send + Sync {
    /// Open/initialize the sink. Called once before any writes.
    fn open(&mut self) -> SinkResult;

    /// Write a single formatted record to the sink.
    fn write(&mut self, formatted: &str) -> SinkResult;

    /// Write a batch of formatted records.
    ///
    /// Default implementation calls `write()` for each record.
    /// Sinks that support batch I/O (e.g., `writev`, Kafka batch produce)
    /// should override this for better performance.
    fn write_batch(&mut self, formatted: &[String]) -> SinkResult {
        for s in formatted {
            self.write(s)?;
        }
        Ok(())
    }

    /// Flush any buffered data to the underlying output.
    fn flush(&mut self) -> SinkResult;

    /// Close the sink, releasing any resources.
    fn close(&mut self) -> SinkResult;

    /// Check if the sink is healthy.
    fn is_healthy(&self) -> bool {
        true
    }
}

// ===========================================================================
// Console Sink
// ===========================================================================

/// Console sink — writes log records to stdout or stderr.
///
/// # Format
///
/// Outputs plain text logs in the format:
/// `[YYYY-MM-DD HH:MM:SS.mmm] [LEVEL] [thread_id] message`
pub struct ConsoleSink {
    /// Write to stderr instead of stdout
    use_stderr: bool,
    /// Whether the sink is open
    is_open: bool,
    /// Total records written
    records_written: u64,
}

impl ConsoleSink {
    /// Create a new Console sink writing to stdout.
    pub fn new() -> Self {
        Self {
            use_stderr: false,
            is_open: false,
            records_written: 0,
        }
    }

    /// Create a new Console sink writing to stderr.
    pub fn stderr() -> Self {
        Self {
            use_stderr: true,
            is_open: false,
            records_written: 0,
        }
    }

    /// Format a record as a plain text line.
    pub fn format_record(record: &Record) -> String {
        // Convert 128-bit timestamp to total milliseconds since epoch
        let total_ms = record.timestamp.hi * 1000 + record.timestamp.lo / 1_000_000;
        let secs = total_ms / 1000;
        let millis = total_ms % 1000;

        let level_str = record.level.to_str();
        let thread_id = record.thread_id;
        let message = record.message.as_str();

        format!("[{secs}.{millis:03}] [{level_str}] [{thread_id}] {message}")
    }
}

impl Default for ConsoleSink {
    fn default() -> Self {
        Self::new()
    }
}

impl Sink for ConsoleSink {
    fn open(&mut self) -> SinkResult {
        self.is_open = true;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        if !self.is_open {
            return Err(SinkError::Closed);
        }

        // All I/O goes through platform-native syscalls, not libc stdio.
        // M2: direct write/WriteFile; M3: io_uring/IOCP/kqueue.
        if self.use_stderr {
            io::stderr_line(formatted);
        } else {
            io::stdout_line(formatted);
        }

        self.records_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> SinkResult {
        // Direct syscalls are unbuffered — flush is a no-op in M2.
        // M3 with io_uring/IOCP will submit the completion queue here.
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        self.is_open = false;
        Ok(())
    }
}

// ===========================================================================
// Sink wrapper for pipeline use
// ===========================================================================

/// Type-erased Sink wrapper for the pipeline.
///
/// Uses a `Box<dyn Sink>` to allow runtime configuration of the sink type.
pub struct SinkRef {
    inner: Box<dyn Sink>,
}

impl SinkRef {
    /// Create a new SinkRef from any Sink implementation.
    pub fn new(sink: impl Sink + 'static) -> Self {
        Self {
            inner: Box::new(sink),
        }
    }

    /// Format a record using the console sink's default format
    /// (In M2+, this will use the configured Formatter plugin)
    pub fn format_record(record: &Record) -> String {
        ConsoleSink::format_record(record)
    }

    /// Write a formatted record
    pub fn write(&mut self, formatted: &str) -> SinkResult {
        self.inner.write(formatted)
    }

    /// Write a batch of formatted records
    pub fn write_batch(&mut self, formatted: &[String]) -> SinkResult {
        self.inner.write_batch(formatted)
    }

    /// Flush buffered data
    pub fn flush(&mut self) -> SinkResult {
        self.inner.flush()
    }

    /// Open the sink
    pub fn open(&mut self) -> SinkResult {
        self.inner.open()
    }

    /// Close the sink
    pub fn close(&mut self) -> SinkResult {
        self.inner.close()
    }
}
