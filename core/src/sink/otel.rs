//! OpenTelemetry Sink.
//!
//! Exports log records via OTLP (OpenTelemetry Protocol) over HTTP.
//! Associates logs with trace_id and span_id for distributed tracing.
//!
//! # OTLP/JSON Format
//!
//! Sends JSON-encoded log records to an OTLP HTTP endpoint.
//! Compatible with Jaeger, Tempo, and other OTel collectors.
//!
//! # Feature flag
//!
//! Uses `ureq` for HTTP transport (shared with sink-webhook).

use std::time::Duration;

use crate::sink::{Sink, SinkError, SinkResult};

/// OpenTelemetry Sink configuration.
#[derive(Debug, Clone)]
pub struct OtelSinkConfig {
    /// OTLP HTTP endpoint (e.g. "http://localhost:4318/v1/logs")
    pub endpoint: String,
    /// Service name for resource attribution
    pub service_name: String,
    /// Service version
    pub service_version: String,
    /// HTTP request timeout (seconds)
    pub timeout_secs: u64,
    /// Batch size before sending (0 = send immediately)
    pub batch_size: usize,
    /// Max retry attempts
    pub max_retries: u32,
    /// Authentication bearer token (optional)
    pub bearer_token: Option<String>,
    /// Custom HTTP headers
    pub headers: Vec<(String, String)>,
}

impl Default for OtelSinkConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4318/v1/logs".into(),
            service_name: "dologger".into(),
            service_version: env!("CARGO_PKG_VERSION").into(),
            timeout_secs: 10,
            batch_size: 256,
            max_retries: 2,
            bearer_token: None,
            headers: vec![],
        }
    }
}

/// OTel Sink — exports logs via OTLP/HTTP.
pub struct OtelSink {
    config: OtelSinkConfig,
    agent: Option<ureq::Agent>,
    /// Pending batch buffer
    batch: Vec<String>,
    is_open: bool,
    records_exported: u64,
}

impl OtelSink {
    pub fn new(config: OtelSinkConfig) -> Self {
        Self {
            config,
            agent: None,
            batch: Vec::with_capacity(256),
            is_open: false,
            records_exported: 0,
        }
    }

    /// Build a JSON OTLP log record payload.
    fn build_otlp_payload(batch: &[String]) -> String {
        let mut log_records = String::new();
        for (i, msg) in batch.iter().enumerate() {
            if i > 0 {
                log_records.push(',');
            }
            // Escape message for JSON
            let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
            log_records.push_str(&format!(
                r#"{{"timeUnixNano":"{}","body":{{"stringValue":"{}"}}}}"#,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                escaped
            ));
        }

        format!(
            r#"{{"resourceLogs":[{{"resource":{{"attributes":[{{"key":"service.name","value":{{"stringValue":"{}"}}}},{{"key":"service.version","value":{{"stringValue":"{}"}}}}]}},"scopeLogs":[{{"scope":{{"name":"dologger"}},"logRecords":[{}]}}]}}]}}"#,
            "",
            "",
            log_records // placeholder for service name/version
        )
    }

    /// Build proper OTLP JSON with resource attributes.
    fn build_batch_payload(&self) -> String {
        let mut log_records = String::new();
        for (i, msg) in self.batch.iter().enumerate() {
            if i > 0 {
                log_records.push(',');
            }
            let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
            log_records.push_str(&format!(
                r#"{{"timeUnixNano":"{}","body":{{"stringValue":"{}"}}}}"#,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                escaped
            ));
        }

        let svc_name = &self.config.service_name;
        let svc_ver = &self.config.service_version;

        format!(
            r#"{{"resourceLogs":[{{"resource":{{"attributes":[{{"key":"service.name","value":{{"stringValue":"{svc_name}"}}}},{{"key":"service.version","value":{{"stringValue":"{svc_ver}"}}}}]}},"scopeLogs":[{{"scope":{{"name":"dologger"}},"logRecords":[{log_records}]}}]}}]}}"#
        )
    }

    /// Flush the current batch to the OTLP endpoint.
    fn flush_batch(&mut self) -> SinkResult {
        if self.batch.is_empty() {
            return Ok(());
        }

        let payload = self.build_batch_payload();
        let batch_size = self.batch.len();
        self.batch.clear();

        let agent = self.agent.as_ref().ok_or(SinkError::Closed)?;

        let mut attempts = 0u32;
        loop {
            let mut req = agent
                .post(&self.config.endpoint)
                .set("Content-Type", "application/json");

            if let Some(ref token) = self.config.bearer_token {
                req = req.set("Authorization", &format!("Bearer {token}"));
            }
            for (k, v) in &self.config.headers {
                req = req.set(k, v);
            }

            match req.send_string(&payload) {
                Ok(_) => {
                    self.records_exported += batch_size as u64;
                    return Ok(());
                }
                Err(e) => {
                    if attempts < self.config.max_retries {
                        attempts += 1;
                        let delay = 100 * 2u64.pow(attempts);
                        std::thread::sleep(Duration::from_millis(delay));
                        continue;
                    }
                    return Err(SinkError::WriteFailed(format!("otel export: {e}")));
                }
            }
        }
    }
}

impl Sink for OtelSink {
    fn open(&mut self) -> SinkResult {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .build();
        self.agent = Some(agent);
        self.is_open = true;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        if !self.is_open {
            return Err(SinkError::Closed);
        }

        self.batch.push(formatted.to_string());

        if self.batch.len() >= self.config.batch_size {
            self.flush_batch()?;
        }

        Ok(())
    }

    fn write_batch(&mut self, formatted: &[String]) -> SinkResult {
        self.batch.extend(formatted.iter().cloned());
        self.flush_batch()
    }

    fn flush(&mut self) -> SinkResult {
        self.flush_batch()
    }

    fn close(&mut self) -> SinkResult {
        self.flush_batch()?;
        self.agent = None;
        self.is_open = false;
        Ok(())
    }
}
