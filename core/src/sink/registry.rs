//! Sink registry — maps `[sinks.*]` TOML configuration to sink instances.
//!
//! The registry is the single place that knows how to construct a [`SinkRef`]
//! from a declarative configuration. The `[sinks.<name>]` tables in a
//! `dologger.toml` deserialise into [`SinkKindConfig`], and [`build_sinks`]
//! materialises them.
//!
//! # TOML shape
//!
//! Each `[sinks.<name>]` table carries a `type` tag plus that sink's config
//! fields. Unknown fields are rejected (deny by default), so a typo surfaces
//! as a config warning rather than a silently ignored setting.
//!
//! ```toml
//! [sinks.stdout]
//! type = "console"
//!
//! [sinks.applog]
//! type = "file"
//! path = "/var/log/app.log"
//! durability_level = "os_cache"
//! ```
//!
//! The shared-memory sink is intentionally absent: it consumes raw SIF bytes
//! through its own `write(&[u8])` API and is wired separately, not through the
//! formatted-string `Sink` trait this registry constructs.

use crate::sink::{
    ConsoleSink, ConsoleSinkConfig, FanoutSink, FileSink, FileSinkConfig, SecuritySink,
    SecuritySinkConfig, SinkRef, SyslogSink, SyslogSinkConfig, WormSink, WormSinkConfig,
};
#[cfg(feature = "sink-kafka")]
use crate::sink::{KafkaSink, KafkaSinkConfig};
#[cfg(feature = "sink-otel")]
use crate::sink::{OtelSink, OtelSinkConfig};
#[cfg(feature = "sink-sqlite")]
use crate::sink::{SqliteSink, SqliteSinkConfig};
#[cfg(feature = "sink-webhook")]
use crate::sink::{WebhookSink, WebhookSinkConfig};

/// One configured sink, discriminated by the `type` field.
///
/// The `[sinks.<name>]` table's remaining fields deserialise into the
/// matching `*SinkConfig`, whose missing fields fall back to defaults.
/// Feature-gated sinks are only constructible when their feature is enabled.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SinkKindConfig {
    /// Plain-text console output (stdout/stderr).
    Console(ConsoleSinkConfig),
    /// Buffered file output with optional rotation.
    File(FileSinkConfig),
    /// Write-once-read-many audit file.
    Worm(WormSinkConfig),
    /// Hardened, plugin-bypass security audit file.
    Security(SecuritySinkConfig),
    /// RFC 5424 syslog over UDP/TCP/TLS.
    Syslog(SyslogSinkConfig),
    /// Apache Kafka producer.
    #[cfg(feature = "sink-kafka")]
    Kafka(KafkaSinkConfig),
    /// Local SQLite database.
    #[cfg(feature = "sink-sqlite")]
    Sqlite(SqliteSinkConfig),
    /// HTTP webhook with retry/backoff.
    #[cfg(feature = "sink-webhook")]
    Webhook(WebhookSinkConfig),
    /// OpenTelemetry OTLP/HTTP exporter.
    #[cfg(feature = "sink-otel")]
    Otel(OtelSinkConfig),
}

impl SinkKindConfig {
    /// The default sink kind: console to stdout.
    pub fn console() -> Self {
        Self::Console(ConsoleSinkConfig::default())
    }
}

/// Construct a single sink instance from its configuration.
pub fn build_sink(kind: &SinkKindConfig) -> Result<SinkRef, String> {
    let sink = match kind {
        SinkKindConfig::Console(c) => SinkRef::new(if c.use_stderr {
            ConsoleSink::stderr()
        } else {
            ConsoleSink::new()
        }),
        SinkKindConfig::File(c) => SinkRef::new(FileSink::new(c.clone())),
        SinkKindConfig::Worm(c) => SinkRef::new(WormSink::new(c.clone())),
        SinkKindConfig::Security(c) => SinkRef::new(SecuritySink::new(c.clone())),
        SinkKindConfig::Syslog(c) => SinkRef::new(SyslogSink::new(c.clone())),
        #[cfg(feature = "sink-kafka")]
        SinkKindConfig::Kafka(c) => SinkRef::new(KafkaSink::new(c.clone())),
        #[cfg(feature = "sink-sqlite")]
        SinkKindConfig::Sqlite(c) => SinkRef::new(SqliteSink::new(c.clone())),
        #[cfg(feature = "sink-webhook")]
        SinkKindConfig::Webhook(c) => SinkRef::new(WebhookSink::new(c.clone())),
        #[cfg(feature = "sink-otel")]
        SinkKindConfig::Otel(c) => SinkRef::new(OtelSink::new(c.clone())),
    };
    Ok(sink)
}

/// Construct all configured sinks.
///
/// The config layer guarantees the list is never empty (the console default is
/// applied when `[sinks.*]` is absent or empty), so the returned vector has at
/// least one entry in normal operation.
pub fn build_sinks(configs: &[SinkKindConfig]) -> Result<Vec<SinkRef>, String> {
    configs.iter().map(build_sink).collect()
}

/// Wrap a list of configured sinks in a [`FanoutSink`].
pub fn build_fanout(configs: &[SinkKindConfig]) -> Result<FanoutSink, String> {
    Ok(FanoutSink::new(build_sinks(configs)?))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::Sink;

    #[test]
    fn deserialize_console_from_toml() {
        let value: SinkKindConfig = toml::from_str("type = \"console\"\n").expect("console parses");
        assert!(matches!(value, SinkKindConfig::Console(_)));
    }

    #[test]
    fn deserialize_console_stderr() {
        let value: SinkKindConfig =
            toml::from_str("type = \"console\"\nuse_stderr = true\n").expect("parses");
        match value {
            SinkKindConfig::Console(c) => assert!(c.use_stderr),
            other => panic!("expected console, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_file_with_defaults() {
        let value: SinkKindConfig =
            toml::from_str("type = \"file\"\npath = \"/tmp/x.log\"\n").expect("parses");
        match value {
            SinkKindConfig::File(c) => {
                assert_eq!(c.path, std::path::PathBuf::from("/tmp/x.log"));
                assert_eq!(c.buffer_size, 65536, "missing fields fall back to defaults");
            }
            other => panic!("expected file, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_worm_durability() {
        let value: SinkKindConfig =
            toml::from_str("type = \"worm\"\ndurability = \"media_with_fua\"\n").expect("parses");
        match value {
            SinkKindConfig::Worm(c) => {
                assert_eq!(c.durability, crate::sink::WormDurability::MediaWithFua);
            }
            other => panic!("expected worm, got {other:?}"),
        }
    }

    #[test]
    fn unknown_type_is_rejected() {
        let result: Result<SinkKindConfig, _> = toml::from_str("type = \"bogus\"\n");
        assert!(result.is_err(), "unknown sink type must be rejected");
    }

    #[test]
    fn default_console_kind() {
        assert!(matches!(
            SinkKindConfig::console(),
            SinkKindConfig::Console(_)
        ));
    }

    #[test]
    fn build_sinks_constructs_and_opens() {
        let configs = vec![
            SinkKindConfig::console(),
            SinkKindConfig::File(FileSinkConfig {
                path: std::path::PathBuf::from("/tmp/dologger_test_sink.log"),
                ..Default::default()
            }),
        ];
        let mut fanout = build_fanout(&configs).expect("sinks build");
        fanout.open().expect("fanout opens");
        fanout.write("probe").expect("write succeeds");
        fanout.close().expect("fanout closes");
    }
}
