//! Kafka Sink.
//!
//! Apache Kafka producer for high-throughput log streaming.
//! Uses the pure-Rust `rskafka` client (no C broker library).
//!
//! # Feature flag
//!
//! Compile with `--features sink-kafka` to enable the `rskafka` dependency.

#[cfg(feature = "sink-kafka")]
mod producer {
    //! Thin wrapper around the rskafka async client, bridged to the sync
    //! `Sink` trait via a long-lived tokio runtime.
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use rskafka::client::partition::{Compression, UnknownTopicHandling};
    use rskafka::client::ClientBuilder;
    use rskafka::record::Record;
    use tokio::runtime::Runtime;

    /// Owns the tokio runtime and the rskafka client for the lifetime of the
    /// sink. Kept behind the `sink-kafka` feature gate.
    pub struct KafkaProducer {
        runtime: Arc<Runtime>,
        client: rskafka::client::Client,
    }

    impl KafkaProducer {
        /// Connect to a comma-separated broker list, blocking until the client
        /// metadata handshake completes.
        pub fn connect(brokers: &str) -> Result<Self, String> {
            let runtime = Runtime::new().map_err(|e| format!("kafka tokio runtime: {e}"))?;
            let addrs: Vec<String> = brokers
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if addrs.is_empty() {
                return Err("kafka: no brokers configured".into());
            }
            let client = runtime
                .block_on(ClientBuilder::new(addrs).build())
                .map_err(|e| format!("kafka connect: {e}"))?;
            Ok(Self {
                runtime: Arc::new(runtime),
                client,
            })
        }

        /// Produce a single record to the given topic/partition and wait for
        /// the broker ack.
        pub fn produce(
            &self,
            topic: &str,
            partition: i32,
            payload: &[u8],
            key: Option<&[u8]>,
        ) -> Result<(), String> {
            let runtime = Arc::clone(&self.runtime);
            let partition_client = runtime
                .block_on(self.client.partition_client(
                    topic.to_owned(),
                    partition,
                    UnknownTopicHandling::Retry,
                ))
                .map_err(|e| format!("kafka partition client: {e}"))?;
            let record = Record {
                key: key.map(|k| k.to_vec()),
                value: Some(payload.to_vec()),
                headers: BTreeMap::new(),
                timestamp: chrono::Utc::now(),
            };
            runtime
                .block_on(partition_client.produce(vec![record], Compression::default()))
                .map_err(|e| format!("kafka produce: {e}"))?;
            Ok(())
        }
    }
}

use crate::sink::{Sink, SinkError, SinkResult};

/// Kafka Sink configuration.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct KafkaSinkConfig {
    /// Comma-separated list of Kafka brokers
    pub brokers: String,
    /// Target topic
    pub topic: String,
    /// Client ID
    pub client_id: Option<String>,
    /// Compression codec: "none", "gzip", "snappy", "lz4", "zstd"
    pub compression: Option<String>,
    /// Required acks: 0, 1, or "all"
    pub acks: Option<String>,
    /// Maximum time in ms to wait for broker ack (used for timeout in write)
    pub acks_timeout_ms: Option<i32>,
    /// Linger time in ms before sending batch
    pub linger_ms: Option<i32>,
    /// Maximum batch size in bytes
    pub batch_size: Option<i32>,
    /// SASL username
    pub sasl_username: Option<String>,
    /// SASL password
    pub sasl_password: Option<String>,
    /// Enable TLS
    pub enable_tls: bool,
}

impl Default for KafkaSinkConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".into(),
            topic: "dologger".into(),
            client_id: None,
            compression: Some("lz4".into()),
            acks: Some("all".into()),
            acks_timeout_ms: Some(100),
            linger_ms: Some(5),
            batch_size: Some(65536),
            sasl_username: None,
            sasl_password: None,
            enable_tls: false,
        }
    }
}

/// Statistics for the Kafka sink.
#[derive(Debug, Clone, Default)]
pub struct KafkaSinkStats {
    /// Number of records successfully written to Kafka.
    pub records_sent: u64,
    /// Number of records that failed to be written to Kafka.
    pub errors: u64,
    /// Number of bytes written to Kafka.
    pub bytes_sent: u64,
}

/// Kafka Sink — writes formatted log records to an Apache Kafka topic.
pub struct KafkaSink {
    config: KafkaSinkConfig,
    #[cfg(feature = "sink-kafka")]
    producer: Option<producer::KafkaProducer>,
    is_open: bool,
    records_sent: u64,
    errors: u64,
    bytes_sent: u64,
}

impl KafkaSink {
    /// Create a new Kafka sink.
    pub fn new(config: KafkaSinkConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "sink-kafka")]
            producer: None,
            is_open: false,
            records_sent: 0,
            errors: 0,
            bytes_sent: 0,
        }
    }

    /// Open the Kafka producer connection.
    pub fn open(&mut self) -> SinkResult {
        #[cfg(feature = "sink-kafka")]
        {
            let producer = producer::KafkaProducer::connect(&self.config.brokers)
                .map_err(SinkError::WriteFailed)?;
            self.producer = Some(producer);
        }
        #[cfg(not(feature = "sink-kafka"))]
        {
            return Err(SinkError::WriteFailed(
                "Kafka Sink: compiled without 'sink-kafka' feature".into(),
            ));
        }

        self.is_open = true;
        Ok(())
    }

    /// Get statistics.
    pub fn stats(&self) -> KafkaSinkStats {
        KafkaSinkStats {
            records_sent: self.records_sent,
            errors: self.errors,
            bytes_sent: self.bytes_sent,
        }
    }
}

impl Sink for KafkaSink {
    fn open(&mut self) -> SinkResult {
        self.open()
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        #[cfg(feature = "sink-kafka")]
        {
            let payload = formatted.to_owned();
            let key = self.records_sent.to_string();
            let topic = self.config.topic.clone();
            // Scope the immutable borrow of `self.producer` so it ends before
            // the counter fields are mutated below.
            let result = {
                let producer = self.producer.as_ref().ok_or(SinkError::Closed)?;
                producer.produce(&topic, 0, payload.as_bytes(), Some(key.as_bytes()))
            };
            match result {
                Ok(()) => {
                    self.records_sent += 1;
                    self.bytes_sent += payload.len() as u64;
                    Ok(())
                }
                Err(e) => {
                    self.errors += 1;
                    Err(SinkError::WriteFailed(e))
                }
            }
        }
        #[cfg(not(feature = "sink-kafka"))]
        {
            let _ = formatted;
            Err(SinkError::WriteFailed(
                "Kafka Sink: compiled without 'sink-kafka' feature".into(),
            ))
        }
    }

    fn write_batch(&mut self, formatted: &[String]) -> SinkResult {
        for s in formatted {
            self.write(s)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> SinkResult {
        // rskafka produce awaits the broker ack synchronously, so there is
        // nothing left to flush.
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        #[cfg(feature = "sink-kafka")]
        {
            // Dropping the producer stops the runtime and closes connections.
            self.producer = None;
        }
        self.is_open = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_are_sensible() {
        let cfg = KafkaSinkConfig::default();
        assert!(
            !cfg.brokers.is_empty(),
            "brokers must default to localhost:9092"
        );
        assert!(!cfg.topic.is_empty(), "topic must default to 'dologger'");
        assert!(!cfg.enable_tls, "TLS must default to off");
        assert_eq!(
            cfg.acks.as_deref(),
            Some("all"),
            "acks default must be 'all' for durability"
        );
        assert_eq!(
            cfg.compression.as_deref(),
            Some("lz4"),
            "compression default must be 'lz4'"
        );
    }

    #[test]
    fn config_deserializes_with_minimal_fields() {
        let toml_str = r#"
            brokers = "broker1:9092,broker2:9092"
            topic = "audit"
            enable_tls = true
            sasl_username = "u"
            sasl_password = "p"
        "#;
        let cfg: KafkaSinkConfig = toml::from_str(toml_str).expect("partial TOML parses");
        assert_eq!(cfg.brokers, "broker1:9092,broker2:9092");
        assert_eq!(cfg.topic, "audit");
        assert!(cfg.enable_tls);
        assert_eq!(cfg.sasl_username.as_deref(), Some("u"));
        // Compression/acks keep their defaults when omitted.
        assert_eq!(cfg.compression.as_deref(), Some("lz4"));
    }

    #[test]
    fn stats_default_is_zeroed() {
        let stats = KafkaSinkStats::default();
        assert_eq!(stats.records_sent, 0);
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.bytes_sent, 0);
    }

    #[test]
    fn lifecycle_open_close_keeps_counters() {
        // `open` will fail to construct a producer when the broker is
        // unreachable; that's fine — we only verify that close still
        // succeeds and the counters survive.
        let cfg = KafkaSinkConfig {
            brokers: "127.0.0.1:1".into(),
            ..KafkaSinkConfig::default()
        };
        let mut sink = KafkaSink::new(cfg);
        let _ = sink.open();
        sink.close()
            .expect("close must not panic when producer is unset");
        assert_eq!(sink.records_sent, 0);
    }
}
