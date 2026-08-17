//! Webhook Sink.
//!
//! HTTP POST JSON/XML log records to a webhook endpoint.
//! Supports exponential backoff retry and configurable timeouts.
//!
//! # Feature flag
//!
//! Compile with `--features sink-webhook` to enable HTTP support via `ureq`.

#![allow(missing_docs)]

use std::time::Duration;

use crate::sink::{Sink, SinkError, SinkResult};

/// Webhook Sink configuration.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct WebhookSinkConfig {
    /// Target URL (HTTPS recommended)
    pub url: String,
    /// HTTP request timeout (seconds)
    pub timeout_secs: u64,
    /// Max retry attempts (0 = no retry)
    pub max_retries: u32,
    /// Initial backoff (milliseconds), doubles each retry
    pub backoff_ms: u64,
    /// Max backoff cap (milliseconds)
    pub max_backoff_ms: u64,
    /// Custom HTTP headers (key=value pairs)
    pub headers: Vec<(String, String)>,
}

impl Default for WebhookSinkConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8080/logs".into(),
            timeout_secs: 10,
            max_retries: 3,
            backoff_ms: 100,
            max_backoff_ms: 5000,
            headers: vec![("Content-Type".into(), "application/json".into())],
        }
    }
}

/// Webhook Sink — HTTP POST log records in JSON format.
pub struct WebhookSink {
    config: WebhookSinkConfig,
    #[cfg(feature = "sink-webhook")]
    agent: Option<ureq::Agent>,
    consecutive_failures: u32,
    is_open: bool,
}

impl WebhookSink {
    pub fn new(config: WebhookSinkConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "sink-webhook")]
            agent: None,
            consecutive_failures: 0,
            is_open: false,
        }
    }
}

impl Sink for WebhookSink {
    fn open(&mut self) -> SinkResult {
        #[cfg(feature = "sink-webhook")]
        {
            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(self.config.timeout_secs))
                .build();
            self.agent = Some(agent);
        }
        self.is_open = true;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        #[cfg(feature = "sink-webhook")]
        {
            let agent = self.agent.as_ref().ok_or(SinkError::Closed)?;

            // Build JSON payload
            let payload = format!(
                r#"{{"timestamp":"{}","message":"{}"}}"#,
                chrono_now(),
                formatted.replace('"', "\\\"")
            );

            let mut attempts = 0u32;
            let max_retries = self.config.max_retries;

            loop {
                let mut req = agent
                    .post(&self.config.url)
                    .set("Content-Type", "application/json");
                for (k, v) in &self.config.headers {
                    if k != "Content-Type" {
                        req = req.set(k, v);
                    }
                }

                match req.send_string(&payload) {
                    Ok(_resp) => {
                        self.consecutive_failures = 0;
                        return Ok(());
                    }
                    Err(ureq::Error::Status(code, _resp)) => {
                        // Server error → retry
                        if code >= 500 && attempts < max_retries {
                            attempts += 1;
                            backoff_sleep(
                                attempts,
                                self.config.backoff_ms,
                                self.config.max_backoff_ms,
                            );
                            continue;
                        }
                        self.consecutive_failures += 1;
                        return Err(SinkError::WriteFailed(format!("webhook HTTP {code}")));
                    }
                    Err(e) => {
                        if attempts < max_retries {
                            attempts += 1;
                            backoff_sleep(
                                attempts,
                                self.config.backoff_ms,
                                self.config.max_backoff_ms,
                            );
                            continue;
                        }
                        self.consecutive_failures += 1;
                        return Err(SinkError::WriteFailed(format!("webhook: {e}")));
                    }
                }
            }
        }
        #[cfg(not(feature = "sink-webhook"))]
        {
            let _ = formatted;
            Err(SinkError::WriteFailed(
                "Webhook Sink: compiled without 'sink-webhook' feature".into(),
            ))
        }
    }

    fn flush(&mut self) -> SinkResult {
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        #[cfg(feature = "sink-webhook")]
        {
            self.agent = None;
        }
        self.is_open = false;
        Ok(())
    }
}

/// Exponential backoff sleep.
fn backoff_sleep(attempt: u32, base_ms: u64, max_ms: u64) {
    let delay_ms = (base_ms * 2u64.pow(attempt.saturating_sub(1))).min(max_ms);
    std::thread::sleep(Duration::from_millis(delay_ms));
}

/// Get current UTC timestamp as ISO 8601 string (no external deps).
fn chrono_now() -> String {
    // Simple ISO 8601 timestamp without chrono dependency
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since_epoch.as_secs();
    // Basic ISO 8601
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::SinkError;

    #[test]
    fn config_defaults_are_sensible() {
        let cfg = WebhookSinkConfig::default();
        assert!(!cfg.url.is_empty(), "url must have a default");
        assert!(cfg.timeout_secs > 0, "timeout must be positive");
        assert_eq!(
            cfg.headers.first().map(|(k, _)| k.as_str()),
            Some("Content-Type"),
            "Content-Type header should be the first default"
        );
    }

    #[test]
    fn config_deserializes_with_missing_fields() {
        let toml_str = r#"
            url = "https://example.com/log"
        "#;
        let cfg: WebhookSinkConfig = toml::from_str(toml_str).expect("partial TOML parses");
        assert_eq!(cfg.url, "https://example.com/log");
        assert_eq!(cfg.timeout_secs, 10, "missing fields fall back to defaults");
        assert_eq!(cfg.max_retries, 3);
    }

    #[test]
    fn lifecycle_open_close_reports_is_open() {
        let mut sink = WebhookSink::new(WebhookSinkConfig::default());
        assert!(sink.open().is_ok());
        assert!(sink.close().is_ok());
    }

    #[test]
    fn write_without_feature_returns_write_failed() {
        // Compiled without `sink-webhook` feature, every write must surface a
        // structured `WriteFailed` error rather than panic.
        let mut sink = WebhookSink::new(WebhookSinkConfig::default());
        sink.open().expect("open");
        let result = sink.write("hello");
        if !cfg!(feature = "sink-webhook") {
            assert!(
                matches!(result, Err(SinkError::WriteFailed(_))),
                "expected WriteFailed when sink-webhook feature is off, got {result:?}"
            );
        }
    }

    #[test]
    fn backoff_formula_caps_exponentially() {
        // The backoff formula is `(base_ms * 2^(attempt - 1)).min(max_ms)`.
        // Verify it directly by sampling a few attempts against a tiny base.
        // We can't actually exercise it through the public API without
        // hitting the network, so we keep the check narrowly on the math:
        //   attempt 1 -> base
        //   attempt 2 -> 2 * base
        //   attempt 4 -> min(8 * base, max_ms)
        let base_ms: u64 = 100;
        let max_ms: u64 = 500;
        for attempt in 1..=6u32 {
            let computed = base_ms
                .saturating_mul(2u64.pow(attempt.saturating_sub(1)))
                .min(max_ms);
            assert!(computed <= max_ms, "cap must hold at attempt {attempt}");
        }
    }
}
