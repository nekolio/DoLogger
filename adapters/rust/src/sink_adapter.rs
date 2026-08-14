//! Closure-based sink adapter for the core [`Sink`] trait.
//!
//! The SDK does not own the core's output sinks (`FileSink`, `SyslogSink`,
//! `ShmSink`, …) — those live in `dologger_core::sink`. This module adapts a
//! user-provided closure into a full [`Sink`], so any backend (a socket, a
//! queue, a test recorder, a shm/syslog wrapper) can be plugged into the
//! pipeline without implementing the trait yourself:
//!
//! ```rust,no_run
//! use dologger_core::sink::Sink;
//!
//! let sink = dologger_sdk::sink_adapter::FnSink::new(|line: &str| {
//!     eprintln!("[sink] {line}");
//!     Ok(())
//! });
//! ```

use std::sync::Mutex;

use dologger_core::sink::{Sink, SinkResult};

struct FnSinkInner<W> {
    write: W,
    flush: Option<Box<dyn FnMut() -> SinkResult + Send>>,
    close: Option<Box<dyn FnMut() -> SinkResult + Send>>,
    closed: bool,
}

/// A [`Sink`] implemented by a single closure.
///
/// The core `Sink` trait requires `Send + Sync`, so the closures are guarded
/// by a `Mutex`. `open`/`flush`/`close` are no-ops by default (override via
/// [`FnSink::with_flush`], [`FnSink::with_close`]); [`FnSink::is_healthy`]
/// reports healthy until `close` runs.
pub struct FnSink<W> {
    inner: Mutex<FnSinkInner<W>>,
}

impl<W> FnSink<W>
where
    W: FnMut(&str) -> SinkResult + Send,
{
    /// Create a sink whose `write` calls `f` for each formatted record.
    pub fn new(write: W) -> Self {
        Self {
            inner: Mutex::new(FnSinkInner {
                write,
                flush: None,
                close: None,
                closed: false,
            }),
        }
    }

    /// Set a hook called by [`Sink::flush`].
    pub fn with_flush(self, flush: impl FnMut() -> SinkResult + Send + 'static) -> Self {
        self.inner.lock().unwrap().flush = Some(Box::new(flush));
        self
    }

    /// Set a hook called by [`Sink::close`].
    pub fn with_close(self, close: impl FnMut() -> SinkResult + Send + 'static) -> Self {
        self.inner.lock().unwrap().close = Some(Box::new(close));
        self
    }
}

impl<W> Sink for FnSink<W>
where
    W: FnMut(&str) -> SinkResult + Send,
{
    fn open(&mut self) -> SinkResult {
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        let mut inner = self.inner.lock().unwrap();
        (inner.write)(formatted)
    }

    fn flush(&mut self) -> SinkResult {
        let mut inner = self.inner.lock().unwrap();
        if let Some(ref mut flush) = inner.flush {
            flush()
        } else {
            Ok(())
        }
    }

    fn close(&mut self) -> SinkResult {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        if let Some(ref mut close) = inner.close {
            close()
        } else {
            Ok(())
        }
    }

    fn is_healthy(&self) -> bool {
        !self.inner.lock().unwrap().closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fn_sink_forwards_lines() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let lines = Arc::new(AtomicUsize::new(0));
        let lines2 = Arc::clone(&lines);

        let mut sink = FnSink::new(move |_line: &str| {
            lines2.fetch_add(1, Ordering::Relaxed);
            Ok(())
        });

        assert!(sink.open().is_ok());
        sink.write("hello").unwrap();
        sink.write("world").unwrap();
        sink.flush().unwrap();
        assert_eq!(lines.load(Ordering::Relaxed), 2);
        assert!(sink.is_healthy());

        sink.close().unwrap();
        assert!(!sink.is_healthy(), "closed sink reports unhealthy");
    }
}
