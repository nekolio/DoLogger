//! Operational control plane.
//!
//! Lightweight HTTP/JSON control API for runtime management.
//! Supports SetLevel, GetStatus, and ReloadConfig operations.
//! Planned upgrade path: full gRPC with mTLS/JWT.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | /status | Engine status + metrics |
//! | POST | /level | Set log level for a domain |
//! | POST | /reload | Trigger config reload |

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Type alias for shared engine log level.
type SharedLevel = std::sync::Arc<std::sync::Mutex<String>>;
/// Type alias for the config reload callback.
type ReloadCb =
    std::sync::Arc<std::sync::Mutex<Option<Box<dyn Fn() -> Result<(), String> + Send>>>>;

/// Maximum allowed Content-Length for HTTP body (64KB).
/// Prevents OOM DoS from unbounded request bodies.
const MAX_CONTENT_LENGTH: usize = 65536;

/// Control plane configuration.
#[derive(Debug, Clone)]
pub struct ControlPlaneConfig {
    /// Bind address (e.g., "127.0.0.1:9090")
    pub bind_addr: String,
    /// Enable mTLS (planned — requires cert paths)
    pub enable_tls: bool,
}

impl Default for ControlPlaneConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:9090".into(),
            enable_tls: false,
        }
    }
}

/// Operational control plane server.
pub struct ControlPlane {
    shutdown: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
    /// Shared log level for the engine (read/write via /status and /level)
    pub engine_level: SharedLevel,
    /// Callback triggered on POST /reload (e.g., config reload)
    pub reload_callback: ReloadCb,
}

impl ControlPlane {
    /// Start the control plane server.
    ///
    /// `engine_level` — shared log level (mutable via POST /level).
    /// `reload_callback` — optional callback invoked on POST /reload.
    pub fn start(
        config: ControlPlaneConfig,
        engine_level: SharedLevel,
        reload_callback: ReloadCb,
    ) -> Result<Self, String> {
        let bind_addr = config.bind_addr.clone();
        let listener = TcpListener::bind(&bind_addr)
            .map_err(|e| format!("Control plane bind {bind_addr}: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("Control plane nonblocking: {e}"))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown);
        let addr_for_thread = bind_addr.clone();
        let level = Arc::clone(&engine_level);
        let cb = Arc::clone(&reload_callback);

        let listener_thread = thread::Builder::new()
            .name("dologger-control-plane".into())
            .spawn(move || {
                control_loop(listener, shutdown_flag, &addr_for_thread, level, cb);
            })
            .map_err(|e| format!("Control plane thread: {e}"))?;

        crate::sys::diag::info(
            "control_plane",
            &format!("Control plane listening on {bind_addr}"),
        );

        Ok(Self {
            shutdown,
            listener_thread: Some(listener_thread),
            engine_level,
            reload_callback,
        })
    }

    /// Shutdown the control plane gracefully.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }
        crate::sys::diag::info("control_plane", "Control plane stopped");
    }
}

/// Simple HTTP/JSON control loop.
fn control_loop(
    listener: TcpListener,
    shutdown: Arc<AtomicBool>,
    _addr: &str,
    engine_level: SharedLevel,
    reload_callback: ReloadCb,
) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                let level = Arc::clone(&engine_level);
                let cb = Arc::clone(&reload_callback);
                handle_request(stream, level, cb);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Handle a single HTTP request.
fn handle_request(mut stream: TcpStream, engine_level: SharedLevel, reload_callback: ReloadCb) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        send_response(&mut stream, 400, r#"{"error":"bad request"}"#);
        return;
    }

    let method = parts[0];
    let path = parts[1];

    // Read headers
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            break;
        }
        if header.trim().is_empty() {
            break;
        }
        if header.to_lowercase().starts_with("content-length:") {
            content_length = header
                .split(':')
                .nth(1)
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0);
        }
    }

    // SAFETY: Enforce a Content-Length limit of 64KB to prevent OOM DoS
    // from an attacker sending a crafted Content-Length header with
    // an unbounded value. Any body larger than MAX_CONTENT_LENGTH is
    // rejected with 413 Payload Too Large.
    if content_length > MAX_CONTENT_LENGTH {
        send_response(&mut stream, 413, r#"{"error":"payload too large"}"#);
        return;
    }

    // Read body
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    match (method, path) {
        ("GET", "/status") => {
            let level = {
                let guard = engine_level.lock().unwrap();
                guard.clone()
            };
            let status = format!(
                r#"{{"status":"ok","level":"{}","profile":"prod-performance","plugins":0,"signature_enabled":false}}"#,
                level
            );
            send_response(&mut stream, 200, &status);
        }
        ("POST", "/level") => {
            let body_str = String::from_utf8_lossy(&body);
            // Simple JSON parsing: extract "level" field
            let requested_level = body_str
                .split("\"level\"")
                .nth(1)
                .and_then(|s| s.split('"').nth(2))
                .unwrap_or("INFO");
            // Actually update the shared level
            {
                let mut guard = engine_level.lock().unwrap();
                *guard = requested_level.to_string();
            }
            crate::sys::diag::info(
                "control_plane",
                &format!("Log level set to: {}", requested_level),
            );
            let resp = format!(r#"{{"status":"ok","level":"{}"}}"#, requested_level);
            send_response(&mut stream, 200, &resp);
        }
        ("POST", "/reload") => {
            // Invoke the reload callback if present
            let reload_result = {
                let guard = reload_callback.lock().unwrap();
                if let Some(ref cb) = *guard {
                    match cb() {
                        Ok(()) => Some("ok".to_string()),
                        Err(e) => Some(e),
                    }
                } else {
                    None
                }
            };

            match reload_result {
                None => {
                    crate::sys::diag::info(
                        "control_plane",
                        "Config reload requested (no callback)",
                    );
                    send_response(
                        &mut stream,
                        200,
                        r#"{"status":"ok","message":"config reload initiated (no-op: no callback registered)"}"#,
                    );
                }
                Some(ref err) if err != "ok" => {
                    crate::sys::diag::warn(
                        "control_plane",
                        &format!("Config reload failed: {}", err),
                    );
                    let resp = format!(
                        r#"{{"status":"error","message":"config reload failed: {}"}}"#,
                        err
                    );
                    send_response(&mut stream, 500, &resp);
                }
                Some(_) => {
                    crate::sys::diag::info("control_plane", "Config reload succeeded");
                    send_response(
                        &mut stream,
                        200,
                        r#"{"status":"ok","message":"config reload initiated"}"#,
                    );
                }
            }
        }
        _ => {
            send_response(&mut stream, 404, r#"{"error":"not found"}"#);
        }
    }
}

fn send_response(stream: &mut TcpStream, status: u16, body: &str) {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}
