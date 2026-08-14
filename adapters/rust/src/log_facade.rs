//! `log` crate facade adapter.
//!
//! Bridges the de-facto standard `log` facade into DoLogger. Call
//! [`install_log`] once with a shared [`LoggerHandle`]; every `log::info!`,
//! `log::error!`, etc. macro then routes into the DoLogger engine.
//!
//! ```rust,no_run
//! let handle = dologger_sdk::Logger::init_handle(None).expect("init");
//! dologger_sdk::log_facade::install_log(handle).expect("install");
//! log::info!("hello from the log facade");
//! ```

use dologger_core::record::LogLevel;
use log::{Level, Log, Metadata, Record};

use crate::LoggerHandle;

/// A `log::Log` implementation that forwards every record to a DoLogger
/// [`LoggerHandle`].
pub struct LogBridge {
    handle: LoggerHandle,
}

impl LogBridge {
    /// Create a facade that forwards to `handle`.
    pub fn new(handle: LoggerHandle) -> Self {
        Self { handle }
    }

    /// The logger this facade forwards to.
    pub fn handle(&self) -> &LoggerHandle {
        &self.handle
    }
}

fn map_level(level: Level) -> LogLevel {
    match level {
        Level::Trace => LogLevel::Trace,
        Level::Debug => LogLevel::Debug,
        Level::Info => LogLevel::Info,
        Level::Warn => LogLevel::Warn,
        Level::Error => LogLevel::Error,
    }
}

impl Log for LogBridge {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        // Filtering is handled by the `log` facade's own max-level + the
        // DoLogger pipeline's drop-level policy; accept everything here.
        true
    }

    fn log(&self, record: &Record) {
        self.handle
            .log(map_level(record.level()), &record.args().to_string());
    }

    fn flush(&self) {
        // The DoLogger pipeline flushes asynchronously; nothing to do.
    }
}

/// Install `handle` as the global `log` logger, capturing every `log::*!`
/// macro call. Idempotent-in-failure: returns `SetLoggerError` if a logger is
/// already installed.
pub fn install_log(handle: LoggerHandle) -> Result<(), log::SetLoggerError> {
    log::set_boxed_logger(Box::new(LogBridge::new(handle)))?;
    log::set_max_level(log::LevelFilter::Trace);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_logs() {
        let handle = crate::Logger::init_handle(None).expect("init handle");
        let bridge = LogBridge::new(handle);
        log::set_boxed_logger(Box::new(bridge)).expect("set logger");
        log::set_max_level(log::LevelFilter::Trace);

        log::trace!("facade trace");
        log::debug!("facade debug");
        log::info!("facade info");
        log::warn!("facade warn");
        log::error!("facade error");
    }
}
