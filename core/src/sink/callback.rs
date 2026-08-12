//! Callback Sink — passes formatted log data to a host-registered callback.
//!
//! # Use Case
//!
//! When the host application wants to receive log data directly
//! (e.g., for in-process forwarding to another system) instead of
//! writing to a file or network.
//!
//! # Safety
//!
//! The callback is called from the pipeline consumer thread.
//! It MUST NOT block for extended periods — blocking the callback
//! blocks the entire pipeline.

use crate::sink::{Sink, SinkError, SinkResult};

/// Type alias for the callback function.
///
/// Takes a formatted log line (`&str`) and returns `Ok(())` or an error.
pub type LogCallback = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Callback Sink — invokes a host-provided callback for each log line.
pub struct CallbackSink {
    /// The callback function
    callback: Option<LogCallback>,
    /// Whether the sink is open
    is_open: bool,
    /// Total callbacks invoked
    callbacks_invoked: u64,
}

impl CallbackSink {
    /// Create a new CallbackSink with the given callback.
    pub fn new(callback: LogCallback) -> Self {
        Self {
            callback: Some(callback),
            is_open: false,
            callbacks_invoked: 0,
        }
    }
}

impl Sink for CallbackSink {
    fn open(&mut self) -> SinkResult {
        if self.callback.is_none() {
            return Err(SinkError::WriteFailed("No callback registered".into()));
        }
        self.is_open = true;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        if !self.is_open {
            return Err(SinkError::Closed);
        }

        if let Some(ref cb) = self.callback {
            cb(formatted).map_err(|e| SinkError::WriteFailed(format!("callback: {e}")))?;
            self.callbacks_invoked += 1;
            Ok(())
        } else {
            Err(SinkError::WriteFailed("Callback was consumed".into()))
        }
    }

    fn flush(&mut self) -> SinkResult {
        Ok(()) // No buffering for callbacks
    }

    fn close(&mut self) -> SinkResult {
        self.is_open = false;
        Ok(())
    }
}
