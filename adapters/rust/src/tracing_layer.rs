//! `tracing-subscriber` `Layer` adapter.
//!
//! Bridges `tracing` events into DoLogger. Install the layer on any
//! `tracing_subscriber` registry:
//!
//! ```rust,no_run
//! use tracing_subscriber::prelude::*;
//!
//! let handle = dologger_sdk::Logger::init_handle(None).expect("init");
//! tracing_subscriber::registry()
//!     .with(dologger_sdk::tracing_layer::TracingBridge::new(handle))
//!     .init();
//!
//! tracing::info!("hello from tracing");
//! ```

use dologger_core::record::LogLevel;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::LoggerHandle;

/// A `tracing_subscriber::Layer` that forwards every event to a DoLogger
/// [`LoggerHandle`].
pub struct TracingBridge {
    handle: LoggerHandle,
}

impl TracingBridge {
    /// Create a layer that forwards to `handle`.
    pub fn new(handle: LoggerHandle) -> Self {
        Self { handle }
    }

    /// The logger this layer forwards to.
    pub fn handle(&self) -> &LoggerHandle {
        &self.handle
    }
}

fn map_level(level: Level) -> LogLevel {
    match level {
        Level::TRACE => LogLevel::Trace,
        Level::DEBUG => LogLevel::Debug,
        Level::INFO => LogLevel::Info,
        Level::WARN => LogLevel::Warn,
        Level::ERROR => LogLevel::Error,
    }
}

/// Captures the event's `message` field, falling back to the target name.
struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

impl<S> Layer<S> for TracingBridge
where
    S: Subscriber + for<'a> LookupSpan<'a> + 'static,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor { message: None };
        event.record(&mut visitor);

        let msg = visitor
            .message
            .unwrap_or_else(|| event.metadata().target().to_string());
        self.handle.log(map_level(*event.metadata().level()), &msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    #[test]
    fn layer_logs() {
        let handle = crate::Logger::init_handle(None).expect("init handle");
        tracing_subscriber::registry()
            .with(TracingBridge::new(handle))
            .init();

        tracing::trace!("tracing trace");
        tracing::debug!("tracing debug");
        tracing::info!("tracing info");
        tracing::warn!("tracing warn");
        tracing::error!("tracing error");
    }
}
