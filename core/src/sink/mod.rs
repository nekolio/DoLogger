//! Output sinks for log records.
//!
//! Contains the Sink trait, all concrete sink implementations (console,
//! file, callback, Kafka, syslog, webhook, SQLite, WORM, security, OTel,
//! shared memory), and the shared-memory infrastructure.

pub mod callback;
pub mod console;
pub mod fanout;
pub mod file;
#[cfg(feature = "sink-kafka")]
pub mod kafka;
#[cfg(feature = "sink-otel")]
pub mod open_telemetry;
pub mod registry;
pub mod security;
pub mod shm;
#[cfg(feature = "sink-sqlite")]
pub mod sqlite;
pub mod syslog;
#[cfg(feature = "sink-webhook")]
pub mod webhook;
pub mod worm;

pub use callback::{CallbackSink, LogCallback};
pub use console::{
    ConsoleSink, ConsoleSinkConfig, DurabilityLevel, Sink, SinkError, SinkRef, SinkResult,
};
pub use fanout::FanoutSink;
pub use file::{FileSink, FileSinkConfig};
#[cfg(feature = "sink-kafka")]
pub use kafka::{KafkaSink, KafkaSinkConfig, KafkaSinkStats};
#[cfg(feature = "sink-otel")]
pub use open_telemetry::{OtelSink, OtelSinkConfig};
pub use registry::{build_fanout, build_sink, build_sinks, SinkKindConfig};
pub use security::{SecuritySink, SecuritySinkConfig};
pub use shm::{ShmFullPolicy, ShmSink, ShmSinkConfig, ShmSinkStats};
#[cfg(feature = "sink-sqlite")]
pub use sqlite::{SqliteSink, SqliteSinkConfig};
pub use syslog::{SyslogFacility, SyslogProtocol, SyslogSink, SyslogSinkConfig};
#[cfg(feature = "sink-webhook")]
pub use webhook::{WebhookSink, WebhookSinkConfig};
pub use worm::{WormDurability, WormSink, WormSinkConfig};
