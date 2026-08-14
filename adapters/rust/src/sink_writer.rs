//! `std::io::Write` sink adapter.
//!
//! Lets any crate that writes to an `io::Write` (print macros, `eprintln!`
//! redirection, serialization writers, etc.) route its bytes into DoLogger.
//! Each `write` call emits one log record at the configured level, trimmed of
//! a trailing newline so `writeln!`-style output does not double-space.

use std::io;

use dologger_core::record::LogLevel;

use crate::LoggerHandle;

/// An `io::Write` that forwards each chunk to a DoLogger [`LoggerHandle`].
#[derive(Clone)]
pub struct LoggerWriter {
    handle: LoggerHandle,
    level: LogLevel,
}

impl LoggerWriter {
    /// Create a writer that emits every chunk at `level`.
    pub fn new(handle: LoggerHandle, level: LogLevel) -> Self {
        Self { handle, level }
    }

    /// The logger this writer forwards to.
    pub fn handle(&self) -> &LoggerHandle {
        &self.handle
    }

    /// The level at which this writer logs.
    pub fn level(&self) -> LogLevel {
        self.level
    }
}

impl io::Write for LoggerWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = std::str::from_utf8(buf).unwrap_or("<non-UTF-8 bytes>");
        // Trim a trailing newline (and surrounding whitespace) so `writeln!`
        // style output maps to a single clean record.
        self.handle.log(self.level, text.trim());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl std::fmt::Debug for LoggerWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggerWriter")
            .field("level", &self.level)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn writer_forwards_records() {
        let handle = crate::Logger::init_handle(None).expect("init handle");
        let mut writer = LoggerWriter::new(handle, LogLevel::Info);

        writer.write_all(b"hello dologger\n").unwrap();
        writer.flush().unwrap();
    }
}
