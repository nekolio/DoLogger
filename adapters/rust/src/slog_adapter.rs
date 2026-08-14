//! `slog` Drain adapter.
//!
//! Bridges a `slog::Logger` into DoLogger. Attach the drain with
//! `slog::Logger::root(dologger_sdk::slog_adapter::SlogBridge::new(handle), o!())`.
//!
//! ```rust,no_run
//! let handle = dologger_sdk::Logger::init_handle(None).expect("init");
//! let slogger = slog::Logger::root(
//!     dologger_sdk::slog_adapter::SlogBridge::new(handle),
//!     slog::o!(),
//! );
//! slog::info!(slogger, "hello from slog");
//! ```

use dologger_core::record::LogLevel;

use crate::LoggerHandle;

/// A `slog::Drain` that forwards every record to a DoLogger [`LoggerHandle`].
pub struct SlogBridge {
    handle: LoggerHandle,
}

// `SlogBridge::log` takes `&self`, performs only self-contained operations
// (no borrows held across a potential panic), and forwards into the lock-free
// ring buffer. `slog::Logger<D>` requires its drain to be unwind-safe.
// `UnwindSafe`/`RefUnwindSafe` are auto traits that may be implemented when no
// reference is held across a panic — true here.
impl std::panic::UnwindSafe for SlogBridge {}
impl std::panic::RefUnwindSafe for SlogBridge {}

impl SlogBridge {
    /// Create a drain that forwards to `handle`.
    pub fn new(handle: LoggerHandle) -> Self {
        Self { handle }
    }

    /// The logger this drain forwards to.
    pub fn handle(&self) -> &LoggerHandle {
        &self.handle
    }
}

fn map_level(level: slog::Level) -> LogLevel {
    match level {
        slog::Level::Trace => LogLevel::Trace,
        slog::Level::Debug => LogLevel::Debug,
        slog::Level::Info => LogLevel::Info,
        slog::Level::Warning => LogLevel::Warn,
        slog::Level::Error => LogLevel::Error,
        slog::Level::Critical => LogLevel::Fatal,
    }
}

impl slog::Drain for SlogBridge {
    type Ok = ();
    type Err = slog::Never;

    fn log(
        &self,
        record: &slog::Record,
        _values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        self.handle
            .log(map_level(record.level()), &format!("{}", record.msg()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_logs() {
        let handle = crate::Logger::init_handle(None).expect("init handle");
        let logger = slog::Logger::root(SlogBridge::new(handle), slog::o!());
        slog::info!(logger, "slog hello");
        slog::warn!(logger, "slog warn");
        slog::error!(logger, "slog error");
    }
}
