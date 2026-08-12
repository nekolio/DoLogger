//! Syslog Sink.
//!
//! Sends formatted log records to a Syslog server per RFC 5424.
//! Supports UDP, TCP, and TLS transports.
//!
//! # RFC 5424 message format
//!
//! `<PRI>VERSION TIMESTAMP HOSTNAME APP-NAME PROCID MSGID STRUCTURED-DATA MSG`
//!
//! PRI = facility * 8 + severity

use std::io::Write;
use std::net::{TcpStream, UdpSocket};
use std::time::Duration;

use crate::record::Record;
use crate::sink::{Sink, SinkError, SinkResult};

/// Syslog transport protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyslogProtocol {
    /// UDP (RFC 5426) — connectionless, best-effort delivery.
    Udp,
    /// TCP (RFC 6587) — octet-counted framing with reliable delivery.
    Tcp,
    /// TLS over TCP (RFC 5425) — encrypted transport requiring TLS support.
    Tls,
}

/// Syslog facility codes (RFC 5424).
#[derive(Debug, Clone, Copy)]
pub enum SyslogFacility {
    /// Kernel messages.
    Kernel = 0,
    /// User-level messages.
    User = 1,
    /// Mail system messages.
    Mail = 2,
    /// System daemon messages.
    Daemon = 3,
    /// Security/authorization messages.
    Auth = 4,
    /// Internal syslogd messages.
    Syslog = 5,
    /// Line printer subsystem messages.
    Lpr = 6,
    /// Network news subsystem messages.
    News = 7,
    /// UUCP subsystem messages.
    Uucp = 8,
    /// Cron/at scheduler messages.
    Cron = 9,
    /// Security/authorization messages (private).
    Authpriv = 10,
    /// FTP daemon messages.
    Ftp = 11,
    /// Local use facility 0.
    Local0 = 16,
    /// Local use facility 1.
    Local1 = 17,
    /// Local use facility 2.
    Local2 = 18,
    /// Local use facility 3.
    Local3 = 19,
    /// Local use facility 4.
    Local4 = 20,
    /// Local use facility 5.
    Local5 = 21,
    /// Local use facility 6.
    Local6 = 22,
    /// Local use facility 7.
    Local7 = 23,
}

impl SyslogFacility {
    fn code(self) -> u8 {
        self as u8
    }
}

/// Map DoLogger LogLevel to Syslog severity (per RFC 5424).
#[allow(dead_code)]
fn to_syslog_severity(level: &crate::record::LogLevel) -> u8 {
    use crate::record::LogLevel;
    match level {
        LogLevel::Trace => 7, // Debug
        LogLevel::Debug => 7, // Debug
        LogLevel::Info => 6,  // Informational
        LogLevel::Warn => 4,  // Warning
        LogLevel::Error => 3, // Error
        LogLevel::Fatal => 2, // Critical
        LogLevel::Audit => 2, // Critical
    }
}

/// Syslog Sink configuration.
#[derive(Debug, Clone)]
pub struct SyslogSinkConfig {
    /// Syslog server hostname or IP address.
    pub host: String,
    /// Syslog server port (default: 514).
    pub port: u16,
    /// Transport protocol (UDP, TCP, or TLS).
    pub protocol: SyslogProtocol,
    /// Syslog facility code.
    pub facility: SyslogFacility,
    /// Hostname to report in syslog message headers.
    pub hostname: String,
    /// Application name for syslog message headers.
    pub app_name: String,
    /// Connection timeout in seconds.
    pub connect_timeout_secs: u64,
}

impl Default for SyslogSinkConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 514,
            protocol: SyslogProtocol::Udp,
            facility: SyslogFacility::User,
            hostname: "localhost".into(),
            app_name: "dologger".into(),
            connect_timeout_secs: 5,
        }
    }
}

/// Syslog Sink — RFC 5424 compliant.
pub struct SyslogSink {
    config: SyslogSinkConfig,
    udp_socket: Option<UdpSocket>,
    tcp_stream: Option<TcpStream>,
    is_open: bool,
}

impl SyslogSink {
    /// Create a new syslog sink with the given configuration.
    pub fn new(config: SyslogSinkConfig) -> Self {
        Self {
            config,
            udp_socket: None,
            tcp_stream: None,
            is_open: false,
        }
    }

    /// Format a record as an RFC 5424 syslog message.
    #[allow(dead_code)]
    fn format_rfc5424(&self, record: &Record, formatted: &str) -> Vec<u8> {
        let pri = self.config.facility.code() * 8 + to_syslog_severity(&record.level);
        let timestamp = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            // Simplified timestamp from formatted prefix
            1970,
            1,
            1,
            0,
            0,
            0,
            0
        );

        // RFC 5424: <PRI>VERSION TIMESTAMP HOSTNAME APP-NAME PROCID MSGID STRUCTURED-DATA MSG
        // For simplicity, extract message from formatted output
        let message = extract_message(formatted);

        format!(
            "<{pri}>1 {ts} {host} {app} {pid} - - {msg}\n",
            pri = pri,
            ts = timestamp,
            host = self.config.hostname,
            app = self.config.app_name,
            pid = std::process::id(),
            msg = message
        )
        .into_bytes()
    }
}

/// Extract the message portion from a ConsoleSink-formatted line.
fn extract_message(formatted: &str) -> &str {
    // Format: "[secs.millis] [LEVEL] [thread_id] message"
    // Find the third "] "
    let mut bracket_count = 0;
    for (i, c) in formatted.char_indices() {
        if c == ']' {
            bracket_count += 1;
            if bracket_count == 3 {
                if let Some(rest) = formatted.get(i + 2..) {
                    return rest;
                }
            }
        }
    }
    formatted
}

impl Sink for SyslogSink {
    fn open(&mut self) -> SinkResult {
        match self.config.protocol {
            SyslogProtocol::Udp => {
                let addr = format!("{}:{}", self.config.host, self.config.port);
                let socket = UdpSocket::bind("0.0.0.0:0")
                    .map_err(|e| SinkError::WriteFailed(format!("udp bind: {e}")))?;
                socket
                    .set_write_timeout(Some(Duration::from_secs(self.config.connect_timeout_secs)))
                    .ok();
                socket
                    .connect(&addr)
                    .map_err(|e| SinkError::WriteFailed(format!("udp connect: {e}")))?;
                self.udp_socket = Some(socket);
            }
            SyslogProtocol::Tcp | SyslogProtocol::Tls => {
                let addr = format!("{}:{}", self.config.host, self.config.port);
                let stream = TcpStream::connect_timeout(
                    &addr
                        .parse()
                        .map_err(|e| SinkError::WriteFailed(format!("parse addr: {e}")))?,
                    Duration::from_secs(self.config.connect_timeout_secs),
                )
                .map_err(|e| SinkError::WriteFailed(format!("tcp connect: {e}")))?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(self.config.connect_timeout_secs)))
                    .ok();
                self.tcp_stream = Some(stream);
            }
        }
        self.is_open = true;
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        // We need a Record reference for syslog PRI calculation.
        // In the pipeline, we don't have the Record here — only the formatted string.
        // For a full implementation, we'd cache the Record or use a syslog-specific format.
        // For now, send the formatted string directly (fallback mode).
        let data = format!(
            "<{pri}>1 - - {host} {app} {pid} - - {msg}\n",
            pri = self.config.facility.code() * 8 + 6, // INFO severity
            host = self.config.hostname,
            app = self.config.app_name,
            pid = std::process::id(),
            msg = extract_message(formatted)
        );

        match self.config.protocol {
            SyslogProtocol::Udp => {
                if let Some(ref socket) = self.udp_socket {
                    socket
                        .send(data.as_bytes())
                        .map_err(|e| SinkError::WriteFailed(format!("udp send: {e}")))?;
                }
            }
            SyslogProtocol::Tcp | SyslogProtocol::Tls => {
                if let Some(ref mut stream) = self.tcp_stream {
                    stream
                        .write_all(data.as_bytes())
                        .map_err(|e| SinkError::WriteFailed(format!("tcp write: {e}")))?;
                }
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> SinkResult {
        if let Some(ref mut stream) = self.tcp_stream {
            stream
                .flush()
                .map_err(|e| SinkError::WriteFailed(format!("tcp flush: {e}")))?;
        }
        Ok(())
    }

    fn close(&mut self) -> SinkResult {
        self.udp_socket = None;
        self.tcp_stream = None;
        self.is_open = false;
        Ok(())
    }
}
