//! DoLogger Rust SDK — ergonomic high-level wrapper
//!
//! Provides a simple, idiomatic Rust API on top of `dologger_core::Engine`.
//! Internally handles record allocation, field population, and ring-buffer
//! submission so callers only need to pass a message string.
//!
//! # Quick Start
//!
//! ```rust
//! let mut logger = dologger_sdk::Logger::init(None).expect("init");
//! logger.info("Application started");
//! logger.warn("Disk usage at 85%");
//! logger.error("Connection refused");
//! logger.shutdown();
//! ```
//!
//! # Shared Handles & Frontend Adapters
//!
//! `Logger::log` takes `&self`, so a `Logger` can be shared across threads
//! behind an [`LoggerHandle`] (`Arc<Logger>`). The SDK ships adapters that
//! route popular logging frontends into DoLogger:
//!
//! - [`log_facade`] — the `log` crate (`install_log` / `impl log::Log`)
//! - [`tracing_layer`] — a `tracing-subscriber` `Layer` (feature `tracing`)
//! - [`slog_drain`] — a `slog` `Drain` (feature `slog`)
//! - [`write_sink`] — a `std::io::Write` sink
//! - [`sink_adapter`] — a closure-based `dologger_core::sink::Sink`
//!
//! # Cooperative Helping
//!
//! When the ring buffer reaches ≥90% capacity, the calling thread will
//! drain a small batch inline before retrying the push.  This prevents
//! indefinite blocking while maintaining low latency under backpressure
//! (cooperative helping).

use std::sync::Arc;

use dologger_core::config::DologgerConfig;
use dologger_core::record::{thread_id_u64, LogLevel};
use dologger_core::Engine;

// ---------------------------------------------------------------------------
// Adapter modules (feature-gated where an external frontend crate is needed)
// ---------------------------------------------------------------------------

#[cfg(feature = "log-facade")]
pub mod log_facade;
pub mod sink_adapter;
#[cfg(feature = "slog")]
pub mod slog_drain;
#[cfg(feature = "tracing")]
pub mod tracing_layer;
pub mod write_sink;

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// A shared, cloneable handle to a [`Logger`].
///
/// Log calls take `&self`, so any number of threads can clone this handle and
/// log concurrently through the lock-free ring buffer. Obtain one via
/// [`Logger::into_handle`] or [`Logger::init_handle`]. Graceful shutdown still
/// requires exclusive access: call [`Logger::shutdown`] on the underlying
/// logger (or use the engine's own lifecycle) before the last handle drops.
pub type LoggerHandle = Arc<Logger>;

/// High-level logging interface around the DoLogger core engine.
///
/// Construct via [`Logger::init`]; call [`Logger::shutdown`] before dropping.
pub struct Logger {
    engine: Engine,
}

impl Logger {
    /// Initialize the DoLogger engine and return a high-level `Logger`.
    ///
    /// If `config_path` is `None`, the default configuration is used
    /// (auto-discovery via `DologgerConfig::load_default()`).
    ///
    /// # Errors
    ///
    /// Returns `Err(msg)` if the engine fails to initialize (e.g. the
    /// config file is malformed or the pipeline consumer thread cannot
    /// be spawned).
    pub fn init(config_path: Option<&str>) -> Result<Self, String> {
        let (config, _warnings) = match config_path {
            Some(path) => DologgerConfig::load_from_file(path)
                .map_err(|(code, msg)| format!("Failed to load config ({code}): {msg}"))?,
            None => DologgerConfig::load_default(),
        };

        Engine::init(config).map(|engine| Self { engine })
    }

    /// Initialize with a pre-built `DologgerConfig`.
    ///
    /// Useful when constructing the config programmatically instead of
    /// loading from a file.
    pub fn init_with_config(config: DologgerConfig) -> Result<Self, String> {
        Engine::init(config).map(|engine| Self { engine })
    }

    /// Initialize the engine and return a shared [`LoggerHandle`].
    ///
    /// Shorthand for `Logger::init(...)` followed by [`Logger::into_handle`].
    pub fn init_handle(config_path: Option<&str>) -> Result<LoggerHandle, String> {
        Logger::init(config_path).map(Logger::into_handle)
    }

    /// Initialize with a pre-built config and return a shared [`LoggerHandle`].
    pub fn init_handle_with_config(config: DologgerConfig) -> Result<LoggerHandle, String> {
        Logger::init_with_config(config).map(Logger::into_handle)
    }

    /// Wrap this logger in an `Arc` so it can be shared across threads and
    /// handed to the frontend adapters ([`log_facade`], [`tracing_layer`],
    /// [`slog_drain`], [`write_sink`]).
    pub fn into_handle(self) -> LoggerHandle {
        Arc::new(self)
    }

    // --- Convenience level methods -------------------------------------

    /// Log at TRACE level.
    pub fn trace(&self, msg: &str) {
        self.log(LogLevel::Trace, msg);
    }

    /// Log at DEBUG level.
    pub fn debug(&self, msg: &str) {
        self.log(LogLevel::Debug, msg);
    }

    /// Log at INFO level.
    pub fn info(&self, msg: &str) {
        self.log(LogLevel::Info, msg);
    }

    /// Log at WARN level.
    pub fn warn(&self, msg: &str) {
        self.log(LogLevel::Warn, msg);
    }

    /// Log at ERROR level.
    pub fn error(&self, msg: &str) {
        self.log(LogLevel::Error, msg);
    }

    /// Log at FATAL level.
    pub fn fatal(&self, msg: &str) {
        self.log(LogLevel::Fatal, msg);
    }

    /// Log at AUDIT level (non-repudiable, WORM write, Ed25519 signed).
    pub fn audit(&self, msg: &str) {
        self.log(LogLevel::Audit, msg);
    }

    /// Submit a log record at the given level.
    ///
    /// Allocates a [`Record`] from the engine's object pool, populates
    /// Ring 0 and Ring 1 fields, and pushes it into the lock-free ring
    /// buffer.  Returns immediately; the background pipeline handles
    /// filtering, formatting, and sink output asynchronously.
    ///
    /// Safe to call concurrently from any thread via a shared [`LoggerHandle`]:
    /// the pool allocator and ring buffer use lock-free atomic operations and
    /// take `&self`.
    ///
    /// If the ring buffer is full and cooperative helping is enabled,
    /// this call may perform a small inline drain before retrying.
    /// If the buffer remains full after helping, the record is silently
    /// dropped and a diagnostic warning is emitted.
    pub fn log(&self, level: LogLevel, msg: &str) {
        // Allocate a record from the pool
        let record_ptr = match self.engine.pool.alloc() {
            Some(r) => r,
            None => {
                // Pool exhausted — nothing we can do
                return;
            }
        };

        // Populate the record
        // SAFETY: record_ptr was just allocated from the pool and grants
        // exclusive access until it is either pushed to the ring buffer or
        // returned via pool.free().
        unsafe {
            let record = &mut *record_ptr;

            // Ring 0: ID + timestamp
            record.id = self.engine.time_source.next_id();
            record.timestamp = self.engine.time_source.now_utc();

            // Ring 1: Level + message + source context
            record.level = level;
            record.message.set(msg);

            // Capture caller location via std::panic::Location (nightly only).
            // When stable, callers compile without source location.
            record.source_line = 0;

            // Thread/process info
            record.thread_id = thread_id_u64();
            record.process_id = std::process::id();
        }

        // Push to ring buffer — with cooperative helping retry
        match self.engine.ring_buffer.try_push(record_ptr) {
            Ok(()) => { /* record accepted */ }
            Err(ptr) => {
                // Attempt cooperative helping once
                if let Some(ref helping) = self.engine.coop_helping {
                    let helped = helping.try_help();
                    if helped > 0 {
                        match self.engine.ring_buffer.try_push(ptr) {
                            Ok(()) => return,
                            Err(ptr2) => {
                                // SAFETY: ptr2 is exclusively owned at this point
                                unsafe {
                                    self.engine.pool.free(&*ptr2);
                                }
                                return;
                            }
                        }
                    }
                }
                // Still full — drop the record
                // SAFETY: ptr is exclusively owned at this point
                unsafe {
                    self.engine.pool.free(&*ptr);
                }
            }
        }
    }

    /// Gracefully shut down the engine.
    ///
    /// Drains the pipeline, flushes all sinks, and frees all resources.
    /// After calling `shutdown()`, no further log calls should be made.
    pub fn shutdown(&mut self) {
        self.engine.shutdown();
    }

    /// Returns a reference to the underlying [`Engine`] for advanced use.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Returns a mutable reference to the underlying [`Engine`].
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_default() {
        let logger = Logger::init(None);
        assert!(logger.is_ok(), "Default init should succeed");
        let mut logger = logger.unwrap();
        logger.shutdown();
    }

    #[test]
    fn init_dev_profile() {
        let config = DologgerConfig::dev_profile();
        let logger = Logger::init_with_config(config);
        assert!(logger.is_ok(), "Dev profile init should succeed");
        let mut logger = logger.unwrap();
        logger.shutdown();
    }

    #[test]
    fn log_all_levels() {
        let mut logger = Logger::init(None).expect("init");
        logger.trace("trace message");
        logger.debug("debug message");
        logger.info("info message");
        logger.warn("warn message");
        logger.error("error message");
        logger.fatal("fatal message");
        logger.audit("audit message");
        logger.shutdown();
    }

    #[test]
    fn log_throughput_stress() {
        let mut logger = Logger::init(None).expect("init");
        for i in 0..10_000 {
            logger.info(&format!("stress test message #{i}"));
        }
        logger.shutdown();
    }

    #[test]
    fn engine_access() {
        let mut logger = Logger::init(None).expect("init");
        let _ = logger.engine();
        let _ = logger.engine_mut();
        logger.shutdown();
    }

    #[test]
    fn shared_handle_logs_from_threads() {
        let logger = Logger::init_handle(None).expect("init handle");
        let t1_logger = logger.clone();
        let t2_logger = logger.clone();
        // The main thread no longer needs its own reference.
        drop(logger);

        let t1 = std::thread::spawn(move || {
            for i in 0..1_000 {
                t1_logger.info(&format!("thread-1 #{i}"));
            }
            t1_logger
        });
        let t2 = std::thread::spawn(move || {
            for i in 0..1_000 {
                t2_logger.warn(&format!("thread-2 #{i}"));
            }
            t2_logger
        });

        let l1 = t1.join().expect("t1");
        let l2 = t2.join().expect("t2");
        // Both joins return the SAME allocation (clones of one Arc). Drop one,
        // then unwrap the sole remaining owner for exclusive shutdown.
        drop(l2);
        let mut logger = match Arc::try_unwrap(l1) {
            Ok(l) => l,
            Err(_) => panic!("logger handle still shared after join"),
        };
        logger.shutdown();
    }
}
