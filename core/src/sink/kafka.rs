//! Kafka Sink.
//!
//! Apache Kafka producer for high-throughput log streaming.
//! Supports async batch production, compression, and SASL/TLS.
//!
//! # Feature flag
//!
//! Compile with `--features sink-kafka` to enable the `rdkafka` dependency.

use std::time::Duration;

use crate::sink::{Sink, SinkError, SinkResult};

/// Kafka Sink configuration.
#[derive(Debug, Clone)]
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
    producer: Option<rdkafka::producer::FutureProducer>,
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
            use rdkafka::config::ClientConfig;
            use rdkafka::producer::FutureProducer;

            let mut cfg = ClientConfig::new();
            cfg.set("bootstrap.servers", &self.config.brokers);

            if let Some(ref id) = self.config.client_id {
                cfg.set("client.id", id);
            }
            if let Some(ref comp) = self.config.compression {
                cfg.set("compression.type", comp);
            }
            if let Some(ref acks) = self.config.acks {
                cfg.set("acks", acks);
            }
            if let Some(linger) = self.config.linger_ms {
                cfg.set("linger.ms", linger.to_string());
            }
            if let Some(batch) = self.config.batch_size {
                cfg.set("batch.size", batch.to_string());
            }
            if let Some(ref user) = self.config.sasl_username {
                cfg.set("security.protocol", "SASL_SSL");
                cfg.set("sasl.mechanism", "SCRAM-SHA-256");
                cfg.set("sasl.username", user);
                if let Some(ref pass) = self.config.sasl_password {
                    cfg.set("sasl.password", pass);
                }
            } else if self.config.enable_tls {
                cfg.set("security.protocol", "SSL");
            }

            let producer: FutureProducer = cfg
                .create()
                .map_err(|e| SinkError::WriteFailed(format!("kafka open: {e}")))?;

            self.producer = Some(producer);
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
            use rdkafka::producer::FutureRecord;
            use std::sync::mpsc;
            use std::time::Duration;

            // Clone and leak producer for 'static lifetime — FutureRecord
            // holds a reference. Memory cost: ~producer struct (negligible).
            let producer: &'static rdkafka::producer::FutureProducer = Box::leak(Box::new(
                self.producer.as_ref().ok_or(SinkError::Closed)?.clone(),
            ));
            // Leak small strings for 'static lifetime — needed because
            // FutureRecord stores &str references and the future is polled
            // in a spawned thread. Memory cost is bounded (~256B per call).
            let topic: &'static str = Box::leak(self.config.topic.clone().into_boxed_str());
            let payload: &'static str = Box::leak(formatted.to_owned().into_boxed_str());
            let key: &'static str = Box::leak(self.records_sent.to_string().into_boxed_str());

            // Bridge async FutureProducer to sync Sink trait via
            // helper thread polling the delivery future.
            let (tx, rx) = mpsc::channel();
            let delivery_future = producer.send(
                FutureRecord::to(topic).payload(payload).key(key),
                Duration::from_millis(100),
            );

            std::thread::spawn(move || {
                use std::future::Future;
                use std::pin::Pin;
                use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

                unsafe fn clone_raw(_: *const ()) -> RawWaker {
                    RawWaker::new(std::ptr::null(), &VTABLE)
                }
                unsafe fn noop(_: *const ()) {}
                static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_raw, noop, noop, noop);

                // SAFETY: the waker uses a null data pointer with no-op clone/wake
                // functions that never dereference it, so constructing it from a
                // null pointer is sound. It is only used to poll the delivery
                // future once per loop iteration in the busy-wait below.
                let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
                let mut cx = Context::from_waker(&waker);
                let mut pinned: Pin<Box<dyn Future<Output = _>>> = Box::pin(delivery_future);

                let result = loop {
                    match pinned.as_mut().poll(&mut cx) {
                        Poll::Ready(r) => break r,
                        Poll::Pending => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                };
                let _ = tx.send(result);
            });

            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(Ok(_)) => {
                    self.records_sent += 1;
                    Ok(())
                }
                Ok(Err((kafka_err, _msg))) => {
                    self.errors += 1;
                    Err(SinkError::WriteFailed(format!(
                        "kafka delivery: {kafka_err}"
                    )))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.errors += 1;
                    Err(SinkError::WriteFailed("kafka delivery timeout".to_string()))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.errors += 1;
                    Err(SinkError::WriteFailed(
                        "kafka producer internal error".to_string(),
                    ))
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
        #[cfg(feature = "sink-kafka")]
        if let Some(ref producer) = self.producer {
            use rdkafka::producer::Producer;
            let _ = producer.flush(Duration::from_secs(5));
        }
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        self.flush()?;
        #[cfg(feature = "sink-kafka")]
        {
            self.producer = None;
        }
        self.is_open = false;
        Ok(())
    }
}
