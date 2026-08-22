//! Operational control plane.
//!
//! Lightweight HTTP/JSON control API for runtime management. The server is
//! disabled unless a caller explicitly starts it, and all runtime counters are
//! supplied through [`ControlPlaneStats`] so `/status` reports live facts rather
//! than a hard-coded placeholder.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/status` | Engine status and atomic metrics |
//! | POST | `/level` | Set the shared log level |
//! | POST | `/reload` | Trigger the registered reload callback |
//! | POST | `/metrics/reset` | Reset process counters |

use std::io::{BufRead, BufReader, Read, Write};
use std::net::SocketAddr;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Shared engine log level.
pub type SharedLevel = Arc<Mutex<String>>;
/// Callback used to apply a validated runtime log level to the engine policy.
pub type LevelCb = Arc<Mutex<Option<Box<dyn Fn(&str) -> Result<(), String> + Send>>>>;
/// Config reload callback.
pub type ReloadCb = Arc<Mutex<Option<Box<dyn Fn() -> Result<(), String> + Send>>>>;

/// Maximum allowed Content-Length for HTTP body (64KB).
const MAX_CONTENT_LENGTH: usize = 65536;
/// Maximum line length accepted while parsing an HTTP request.
const MAX_REQUEST_LINE: usize = 4096;
/// Maximum number of status history entries retained by a caller.
const MAX_STATUS_HISTORY: usize = 64;

/// Live counters exposed by the control plane.
///
/// Every field is atomic so producers and the control-plane thread can update
/// metrics without taking a hot-path mutex. The control plane never owns an
/// engine or sink; it only observes this explicitly shared snapshot.
pub struct ControlPlaneStats {
    started_at: Instant,
    accepted: AtomicU64,
    processed: AtomicU64,
    dropped: AtomicU64,
    errors: AtomicU64,
    sink_errors: AtomicU64,
    dispatch_errors: AtomicU64,
    shm_drops: AtomicU64,
    reload_requests: AtomicU64,
    reloads: AtomicU64,
    active_connections: AtomicU64,
    ring_capacity: AtomicU64,
    ring_fill_permille: AtomicU64,
    plugins: AtomicU64,
    signature_enabled: AtomicBool,
    hot_reload_epoch: AtomicU64,
    profile: Mutex<String>,
    history: Mutex<Vec<StatusEvent>>,
}

impl Default for ControlPlaneStats {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlPlaneStats {
    /// Create an empty metrics snapshot.
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            accepted: AtomicU64::new(0),
            processed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            sink_errors: AtomicU64::new(0),
            dispatch_errors: AtomicU64::new(0),
            shm_drops: AtomicU64::new(0),
            reload_requests: AtomicU64::new(0),
            reloads: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            ring_capacity: AtomicU64::new(0),
            ring_fill_permille: AtomicU64::new(0),
            plugins: AtomicU64::new(0),
            signature_enabled: AtomicBool::new(false),
            hot_reload_epoch: AtomicU64::new(0),
            profile: Mutex::new("unknown".to_string()),
            history: Mutex::new(Vec::new()),
        }
    }

    /// Set static engine facts used by `/status`.
    pub fn configure(&self, profile: &str, ring_capacity: usize, signature_enabled: bool) {
        *self
            .profile
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = profile.to_string();
        self.ring_capacity
            .store(ring_capacity as u64, Ordering::Release);
        self.signature_enabled
            .store(signature_enabled, Ordering::Release);
    }

    /// Set the number of loaded plugins.
    pub fn set_plugins(&self, plugins: usize) {
        self.plugins.store(plugins as u64, Ordering::Release);
    }

    /// Set the current ring fill ratio in permille, clamped to 1000.
    pub fn set_ring_fill(&self, ratio: f64) {
        let permille = (ratio.clamp(0.0, 1.0) * 1000.0).round() as u64;
        self.ring_fill_permille.store(permille, Ordering::Release);
    }

    /// Publish a new hot-reload epoch.
    pub fn set_hot_reload_epoch(&self, epoch: u64) {
        self.hot_reload_epoch.store(epoch, Ordering::Release);
    }

    /// Count a record accepted by the producer boundary.
    pub fn record_accepted(&self) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a record completed by the pipeline.
    pub fn record_processed(&self) {
        self.processed.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a record dropped by policy or backpressure.
    pub fn record_dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Record the result of one drained batch without repeated atomic loads.
    pub fn record_batch(&self, processed: u64, dropped: u64, errors: u64) {
        self.processed.fetch_add(processed, Ordering::Relaxed);
        self.dropped.fetch_add(dropped, Ordering::Relaxed);
        self.errors.fetch_add(errors, Ordering::Relaxed);
    }

    /// Set ring occupancy from exact used/capacity counts.
    pub fn set_ring_fill_counts(&self, used: usize, capacity: usize) {
        if capacity == 0 {
            self.set_ring_fill(0.0);
            return;
        }
        self.set_ring_fill(used as f64 / capacity as f64);
    }
    /// Count an internal processing or sink error.
    pub fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Count an error reported by a sink and include it in the total errors.
    pub fn record_sink_error(&self) {
        self.sink_errors.fetch_add(1, Ordering::Relaxed);
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Count an error reported while dispatching a record and include it in
    /// the total errors.
    pub fn record_dispatch_error(&self) {
        self.dispatch_errors.fetch_add(1, Ordering::Relaxed);
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a record dropped at the shared-memory boundary and include it in
    /// the total dropped records.
    pub fn record_shm_drop(&self) {
        self.shm_drops.fetch_add(1, Ordering::Relaxed);
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a successful or attempted reload request.
    pub fn record_reload_request(&self) {
        self.reload_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a configuration reload attempt owned by the engine.
    pub fn record_reload(&self) {
        self.reloads.fetch_add(1, Ordering::Relaxed);
    }

    /// Add one active connection while a request is being handled.
    fn connection_started(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Remove one active connection after request handling.
    fn connection_finished(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Return a consistent point-in-time snapshot of the counters.
    pub fn snapshot(&self, level: &str) -> StatusSnapshot {
        let profile = self
            .profile
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        StatusSnapshot {
            status: "ok".to_string(),
            level: level.to_string(),
            profile,
            uptime_seconds: self.started_at.elapsed().as_secs(),
            accepted: self.accepted.load(Ordering::Acquire),
            processed: self.processed.load(Ordering::Acquire),
            dropped: self.dropped.load(Ordering::Acquire),
            errors: self.errors.load(Ordering::Acquire),
            sink_errors: self.sink_errors.load(Ordering::Acquire),
            dispatch_errors: self.dispatch_errors.load(Ordering::Acquire),
            shm_drops: self.shm_drops.load(Ordering::Acquire),
            reload_requests: self.reload_requests.load(Ordering::Acquire),
            reloads: self.reloads.load(Ordering::Acquire),
            active_connections: self.active_connections.load(Ordering::Acquire),
            ring_capacity: self.ring_capacity.load(Ordering::Acquire),
            ring_fill_permille: self.ring_fill_permille.load(Ordering::Acquire),
            plugins: self.plugins.load(Ordering::Acquire),
            signature_enabled: self.signature_enabled.load(Ordering::Acquire),
            hot_reload_epoch: self.hot_reload_epoch.load(Ordering::Acquire),
        }
    }

    /// Render the current status without opening a listener.
    pub fn status_json(&self, level: &str) -> String {
        self.snapshot(level).to_json()
    }

    /// Clear the bounded status event history.
    pub fn clear_history(&self) {
        self.history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
    /// Reset counters while preserving static engine facts.
    pub fn reset_counters(&self) {
        self.accepted.store(0, Ordering::Release);
        self.processed.store(0, Ordering::Release);
        self.dropped.store(0, Ordering::Release);
        self.errors.store(0, Ordering::Release);
        self.sink_errors.store(0, Ordering::Release);
        self.dispatch_errors.store(0, Ordering::Release);
        self.shm_drops.store(0, Ordering::Release);
        self.reload_requests.store(0, Ordering::Release);
        self.reloads.store(0, Ordering::Release);
        self.push_history(StatusEvent::CountersReset);
    }

    fn push_history(&self, event: StatusEvent) {
        let mut history = self
            .history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        history.push(event);
        if history.len() > MAX_STATUS_HISTORY {
            let excess = history.len() - MAX_STATUS_HISTORY;
            history.drain(0..excess);
        }
    }

    /// Return a copy of the small in-process status history.
    pub fn history(&self) -> Vec<StatusEvent> {
        self.history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// A status event retained for diagnostics and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusEvent {
    /// The metrics counters were reset.
    CountersReset,
}

/// JSON-ready status snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    /// Overall server health.
    pub status: String,
    /// Current shared log level.
    pub level: String,
    /// Active performance profile.
    pub profile: String,
    /// Seconds since the stats object was created.
    pub uptime_seconds: u64,
    /// Records accepted at the producer boundary.
    pub accepted: u64,
    /// Records completed by the pipeline.
    pub processed: u64,
    /// Records dropped by policy or backpressure.
    pub dropped: u64,
    /// Internal processing or sink errors.
    pub errors: u64,
    /// Errors reported by configured sinks.
    pub sink_errors: u64,
    /// Errors raised while dispatching records to pipeline stages.
    pub dispatch_errors: u64,
    /// Records dropped at the shared-memory boundary.
    pub shm_drops: u64,
    /// Reload requests observed by the engine.
    pub reload_requests: u64,
    /// Configuration reload attempts applied by the engine.
    pub reloads: u64,
    /// Current number of handled connections.
    pub active_connections: u64,
    /// Configured ring capacity.
    pub ring_capacity: u64,
    /// Ring fill in permille.
    pub ring_fill_permille: u64,
    /// Number of loaded plugins.
    pub plugins: u64,
    /// Whether audit signatures are enabled.
    pub signature_enabled: bool,
    /// Current hot-reload epoch.
    pub hot_reload_epoch: u64,
}

/// Control plane configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneConfig {
    /// Whether the listener should be started by an owner.
    pub enabled: bool,
    /// Bind address (for example, `127.0.0.1:9090`).
    pub bind_addr: String,
    /// Enable mTLS (reserved until the TLS boundary is implemented).
    pub enable_tls: bool,
}

impl Default for ControlPlaneConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_addr: "127.0.0.1:9090".into(),
            enable_tls: false,
        }
    }
}

/// Operational control plane server.
pub struct ControlPlane {
    shutdown: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
    /// Shared log level for the engine.
    pub engine_level: SharedLevel,
    /// Optional callback that applies a level to the real engine policy.
    pub level_callback: LevelCb,
    /// Callback triggered on POST `/reload`.
    pub reload_callback: ReloadCb,
    /// Actual bound address (useful when binding to port 0).
    local_addr: String,
    /// Shared live metrics used by `/status`.
    pub stats: Arc<ControlPlaneStats>,
}

impl ControlPlane {
    /// Start the server with an empty metrics object.
    pub fn start(
        config: ControlPlaneConfig,
        engine_level: SharedLevel,
        reload_callback: ReloadCb,
    ) -> Result<Self, String> {
        Self::start_with_stats(
            config,
            engine_level,
            reload_callback,
            Arc::new(ControlPlaneStats::new()),
        )
    }

    /// Start the server with caller-owned live metrics.
    pub fn start_with_stats(
        config: ControlPlaneConfig,
        engine_level: SharedLevel,
        reload_callback: ReloadCb,
        stats: Arc<ControlPlaneStats>,
    ) -> Result<Self, String> {
        Self::start_with_handlers(
            config,
            engine_level,
            Arc::new(Mutex::new(None)),
            reload_callback,
            stats,
        )
    }

    /// Start the server with live engine mutation callbacks.
    ///
    /// The compatibility [`Self::start_with_stats`] constructor only updates
    /// the display level. Engine integrations should use this constructor so
    /// `/level` also updates the pre-filter policy used by the pipeline.
    pub fn start_with_handlers(
        config: ControlPlaneConfig,
        engine_level: SharedLevel,
        level_callback: LevelCb,
        reload_callback: ReloadCb,
        stats: Arc<ControlPlaneStats>,
    ) -> Result<Self, String> {
        if config.enable_tls {
            return Err("control plane TLS is not implemented; refusing insecure downgrade".into());
        }

        let bind_addr = config.bind_addr.clone();
        if !is_loopback_bind_address(&bind_addr) {
            return Err(
                "control plane requires a loopback bind address until TLS/authentication is implemented"
                    .into(),
            );
        }
        let listener = TcpListener::bind(&bind_addr)
            .map_err(|error| format!("Control plane bind {bind_addr}: {error}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| format!("Control plane address: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("Control plane nonblocking: {error}"))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown);
        let level = Arc::clone(&engine_level);
        let level_handler = Arc::clone(&level_callback);
        let callback = Arc::clone(&reload_callback);
        let thread_stats = Arc::clone(&stats);
        let listener_thread = thread::Builder::new()
            .name("dologger-control-plane".into())
            .spawn(move || {
                control_loop(
                    listener,
                    shutdown_flag,
                    level,
                    level_handler,
                    callback,
                    thread_stats,
                );
            })
            .map_err(|error| format!("Control plane thread: {error}"))?;

        crate::sys::diagnostics::info(
            "control_plane",
            &format!("Control plane listening on {bind_addr}"),
        );

        Ok(Self {
            shutdown,
            listener_thread: Some(listener_thread),
            engine_level,
            level_callback,
            reload_callback,
            stats,
            local_addr: local_addr.to_string(),
        })
    }

    /// Return the actual listener address.
    pub fn local_addr(&self) -> &str {
        &self.local_addr
    }

    /// Shutdown the control plane gracefully.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }
        crate::sys::diagnostics::info("control_plane", "Control plane stopped");
    }
}

impl Drop for ControlPlane {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn control_loop(
    listener: TcpListener,
    shutdown: Arc<AtomicBool>,
    engine_level: SharedLevel,
    level_callback: LevelCb,
    reload_callback: ReloadCb,
    stats: Arc<ControlPlaneStats>,
) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                stats.connection_started();
                handle_request(
                    stream,
                    Arc::clone(&engine_level),
                    Arc::clone(&level_callback),
                    Arc::clone(&reload_callback),
                    Arc::clone(&stats),
                );
                stats.connection_finished();
            }
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                stats.record_error();
                crate::sys::diagnostics::error(
                    "control_plane",
                    &format!("Control plane accept failed: {error}"),
                );
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[derive(Debug)]
struct ParsedRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
enum RequestError {
    BadRequest,
    PayloadTooLarge,
    HeaderTooLarge,
    UnsupportedMethod,
}

fn handle_request(
    mut stream: TcpStream,
    engine_level: SharedLevel,
    level_callback: LevelCb,
    reload_callback: ReloadCb,
    stats: Arc<ControlPlaneStats>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let parsed = {
        let mut reader = BufReader::new(&mut stream);
        read_request(&mut reader)
    };
    let request = match parsed {
        Ok(request) => request,
        Err(RequestError::PayloadTooLarge) => {
            send_error(
                &mut stream,
                413,
                crate::error::DO_LOG_ERR_BUFFER_TOO_SMALL,
                None,
            );
            stats.record_error();
            return;
        }
        Err(RequestError::HeaderTooLarge) => {
            send_error(
                &mut stream,
                431,
                crate::error::DO_LOG_ERR_BUFFER_TOO_SMALL,
                None,
            );
            stats.record_error();
            return;
        }
        Err(RequestError::UnsupportedMethod) => {
            send_error(
                &mut stream,
                405,
                crate::error::DO_LOG_ERR_NOT_SUPPORTED,
                None,
            );
            stats.record_error();
            return;
        }
        Err(RequestError::BadRequest) => {
            send_error(&mut stream, 400, crate::error::DO_LOG_ERR_INVALID_ARG, None);
            stats.record_error();
            return;
        }
    };

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/status") => {
            let level = engine_level
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let snapshot = stats.snapshot(&level);
            send_response(&mut stream, 200, &snapshot.to_json());
        }
        ("POST", "/level") => {
            let body = std::str::from_utf8(&request.body).unwrap_or_default();
            let Some(requested_level) = extract_json_string(body, "level") else {
                send_error(&mut stream, 400, crate::error::DO_LOG_ERR_INVALID_ARG, None);
                stats.record_error();
                return;
            };
            if !is_valid_level(requested_level) {
                send_error(
                    &mut stream,
                    400,
                    crate::error::DO_LOG_ERR_INVALID_ARG,
                    Some("level"),
                );
                stats.record_error();
                return;
            }
            let apply_result = level_callback
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(|callback| callback(requested_level));
            if let Some(Err(error)) = apply_result {
                stats.record_error();
                send_error(
                    &mut stream,
                    500,
                    crate::error::DO_LOG_ERR_INVALID_ARG,
                    Some(&error),
                );
                return;
            }
            *engine_level
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = requested_level.to_string();
            let response = format!(
                "{{\"status\":\"ok\",\"level\":\"{}\"}}",
                escape_json(requested_level)
            );
            send_response(&mut stream, 200, &response);
        }
        ("POST", "/reload") => {
            stats.record_reload_request();
            let result = reload_callback
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(|callback| callback());
            match result {
                None => send_response(
                    &mut stream,
                    200,
                    "{\"status\":\"ok\",\"message\":\"reload queued\"}",
                ),
                Some(Ok(())) => send_response(
                    &mut stream,
                    200,
                    "{\"status\":\"ok\",\"message\":\"reload queued\"}",
                ),
                Some(Err(error)) => {
                    stats.record_error();
                    send_error(
                        &mut stream,
                        500,
                        crate::error::DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED,
                        Some(&error),
                    );
                }
            }
        }
        ("POST", "/metrics/reset") => {
            if !request.body.is_empty() {
                send_error(&mut stream, 400, crate::error::DO_LOG_ERR_INVALID_ARG, None);
                stats.record_error();
                return;
            }
            stats.reset_counters();
            send_response(
                &mut stream,
                200,
                "{\"status\":\"ok\",\"message\":\"metrics reset\"}",
            );
        }
        ("GET", _) | ("POST", _) => {
            send_error(
                &mut stream,
                404,
                crate::error::DO_LOG_ERR_NOT_SUPPORTED,
                None,
            );
        }
        _ => {
            send_error(
                &mut stream,
                405,
                crate::error::DO_LOG_ERR_NOT_SUPPORTED,
                None,
            );
        }
    }
}

fn read_request(reader: &mut BufReader<&mut TcpStream>) -> Result<ParsedRequest, RequestError> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.len() > MAX_REQUEST_LINE {
        return Err(RequestError::BadRequest);
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() != 3 || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
        return Err(RequestError::BadRequest);
    }
    if !matches!(parts[0], "GET" | "POST") {
        return Err(RequestError::UnsupportedMethod);
    }
    if parts[1].len() > 256
        || !parts[1].starts_with('/')
        || parts[1].contains("..")
        || parts[1].contains('\0')
    {
        return Err(RequestError::BadRequest);
    }

    let mut content_length = None;
    let mut header_count = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return Err(RequestError::BadRequest);
        }
        if header.len() > MAX_REQUEST_LINE {
            return Err(RequestError::HeaderTooLarge);
        }
        if header.trim().is_empty() {
            break;
        }
        header_count += 1;
        if header_count > 64 {
            return Err(RequestError::HeaderTooLarge);
        }
        let Some((name, value)) = header.split_once(':') else {
            return Err(RequestError::BadRequest);
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "transfer-encoding" {
            return Err(RequestError::BadRequest);
        }
        if name == "content-length" {
            if content_length.is_some() || value.is_empty() {
                return Err(RequestError::BadRequest);
            }
            let length = value
                .parse::<usize>()
                .map_err(|_| RequestError::BadRequest)?;
            if length > MAX_CONTENT_LENGTH {
                return Err(RequestError::PayloadTooLarge);
            }
            content_length = Some(length);
        }
    }

    let length = content_length.unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 && reader.read_exact(&mut body).is_err() {
        return Err(RequestError::BadRequest);
    }
    Ok(ParsedRequest {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        body,
    })
}

fn send_error(stream: &mut TcpStream, status: u16, code: i32, detail: Option<&str>) {
    let descriptor = crate::error::error_descriptor(code);
    let detail = detail.map(escape_json).unwrap_or_default();
    let body = format!(
        "{{\"error_code\":{},\"error_key\":\"{}\",\"message\":\"{}\"{} }}",
        code,
        escape_json(descriptor.key),
        escape_json(descriptor.default_message),
        if detail.is_empty() {
            String::new()
        } else {
            format!(",\"detail\":\"{}\"", detail)
        }
    );
    send_response(stream, status, &body);
}
impl StatusSnapshot {
    /// Return accepted plus processed records for coarse throughput reporting.
    pub fn total_records(&self) -> u64 {
        self.accepted.saturating_add(self.processed)
    }

    /// Whether the snapshot is healthy enough for a liveness response.
    pub fn is_healthy(&self) -> bool {
        self.status == "ok" && self.errors == 0
    }

    /// Return the ring fill as a fraction in the range 0.0..=1.0.
    pub fn ring_fill_ratio(&self) -> f64 {
        self.ring_fill_permille as f64 / 1000.0
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"status\":\"{}\",\"level\":\"{}\",\"profile\":\"{}\",\"uptime_seconds\":{},\"accepted\":{},\"processed\":{},\"dropped\":{},\"errors\":{},\"sink_errors\":{},\"dispatch_errors\":{},\"shm_drops\":{},\"reload_requests\":{},\"reloads\":{},\"active_connections\":{},\"ring_capacity\":{},\"ring_fill_permille\":{},\"plugins\":{},\"signature_enabled\":{},\"hot_reload_epoch\":{}}}",
            escape_json(&self.status),
            escape_json(&self.level),
            escape_json(&self.profile),
            self.uptime_seconds,
            self.accepted,
            self.processed,
            self.dropped,
            self.errors,
            self.sink_errors,
            self.dispatch_errors,
            self.shm_drops,
            self.reload_requests,
            self.reloads,
            self.active_connections,
            self.ring_capacity,
            self.ring_fill_permille,
            self.plugins,
            self.signature_enabled,
            self.hot_reload_epoch,
        )
    }
}

fn is_valid_level(level: &str) -> bool {
    crate::record::LogLevel::parse_canonical(level).is_some()
}

fn is_loopback_bind_address(bind_addr: &str) -> bool {
    let Ok(address) = bind_addr.parse::<SocketAddr>() else {
        return bind_addr.strip_prefix("localhost:").is_some();
    };
    address.ip().is_loopback()
}

fn extract_json_string<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let start = body.find(&needle)? + needle.len();
    let remainder = body[start..].trim_start();
    let remainder = remainder.strip_prefix(':')?.trim_start();
    let remainder = remainder.strip_prefix('"')?;
    let end = remainder.find('"')?;
    Some(&remainder[..end])
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
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
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_snapshot_contains_live_counters() {
        let stats = ControlPlaneStats::new();
        stats.configure("balanced", 65536, true);
        stats.set_plugins(3);
        stats.set_ring_fill(0.25);
        stats.set_hot_reload_epoch(7);
        stats.record_accepted();
        stats.record_processed();
        stats.record_dropped();
        stats.record_error();
        stats.record_reload();

        let snapshot = stats.snapshot("WARN");
        assert_eq!(snapshot.profile, "balanced");
        assert_eq!(snapshot.accepted, 1);
        assert_eq!(snapshot.processed, 1);
        assert_eq!(snapshot.dropped, 1);
        assert_eq!(snapshot.errors, 1);
        assert_eq!(snapshot.reloads, 1);
        assert_eq!(snapshot.ring_fill_permille, 250);
        assert_eq!(snapshot.hot_reload_epoch, 7);
        assert!(snapshot.to_json().contains("\"signature_enabled\":true"));
    }

    #[test]
    fn fine_grained_metrics_update_totals_and_json() {
        let stats = ControlPlaneStats::new();
        stats.record_sink_error();
        stats.record_dispatch_error();
        stats.record_shm_drop();

        let snapshot = stats.snapshot("INFO");
        assert_eq!(snapshot.errors, 2);
        assert_eq!(snapshot.sink_errors, 1);
        assert_eq!(snapshot.dispatch_errors, 1);
        assert_eq!(snapshot.dropped, 1);
        assert_eq!(snapshot.shm_drops, 1);

        let json = snapshot.to_json();
        assert!(json.contains("\"sink_errors\":1"));
        assert!(json.contains("\"dispatch_errors\":1"));
        assert!(json.contains("\"shm_drops\":1"));
    }

    #[test]
    fn batch_updates_and_exact_fill_are_consistent() {
        let stats = ControlPlaneStats::new();
        stats.configure("prod", 100, false);
        stats.record_batch(8, 2, 1);
        stats.set_ring_fill_counts(75, 100);
        let snapshot = stats.snapshot("INFO");
        assert_eq!(snapshot.processed, 8);
        assert_eq!(snapshot.dropped, 2);
        assert_eq!(snapshot.errors, 1);
        assert_eq!(snapshot.ring_fill_permille, 750);
        assert!(stats.status_json("INFO").contains("\"processed\":8"));
        stats.clear_history();
        assert!(stats.history().is_empty());
    }
    #[test]
    fn reset_preserves_static_facts() {
        let stats = ControlPlaneStats::new();
        stats.configure("dev", 1024, false);
        stats.record_accepted();
        stats.record_sink_error();
        stats.record_dispatch_error();
        stats.record_shm_drop();
        stats.reset_counters();
        let snapshot = stats.snapshot("INFO");
        assert_eq!(snapshot.accepted, 0);
        assert_eq!(snapshot.errors, 0);
        assert_eq!(snapshot.sink_errors, 0);
        assert_eq!(snapshot.dispatch_errors, 0);
        assert_eq!(snapshot.dropped, 0);
        assert_eq!(snapshot.shm_drops, 0);
        assert_eq!(snapshot.profile, "dev");
        assert_eq!(stats.history(), vec![StatusEvent::CountersReset]);
    }

    #[test]
    fn live_status_endpoint_returns_metrics() {
        let stats = Arc::new(ControlPlaneStats::new());
        stats.configure("test", 4096, false);
        stats.record_accepted();
        let level = Arc::new(Mutex::new("INFO".to_string()));
        let callback: ReloadCb = Arc::new(Mutex::new(None));
        let mut control_plane = ControlPlane::start_with_stats(
            ControlPlaneConfig {
                enabled: true,
                bind_addr: "127.0.0.1:0".to_string(),
                enable_tls: false,
            },
            level,
            callback,
            Arc::clone(&stats),
        )
        .expect("control plane starts");
        let mut stream = TcpStream::connect(control_plane.local_addr()).expect("connect");
        stream
            .write_all(b"GET /status HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        control_plane.shutdown();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"accepted\":1"));
        assert!(response.contains("\"ring_capacity\":4096"));
    }

    #[test]
    fn level_endpoint_invokes_engine_policy_callback() {
        let stats = Arc::new(ControlPlaneStats::new());
        let level = Arc::new(Mutex::new("INFO".to_string()));
        let applied = Arc::new(std::sync::atomic::AtomicU8::new(u8::MAX));
        let applied_copy = Arc::clone(&applied);
        let level_callback: LevelCb = Arc::new(Mutex::new(Some(Box::new(move |value| {
            let parsed = crate::record::LogLevel::parse_canonical(value)
                .ok_or_else(|| "invalid level".to_string())?;
            applied_copy.store(parsed as u8, Ordering::Release);
            Ok(())
        }))));
        let reload_callback: ReloadCb = Arc::new(Mutex::new(None));
        let mut control_plane = ControlPlane::start_with_handlers(
            ControlPlaneConfig {
                enabled: true,
                bind_addr: "127.0.0.1:0".to_string(),
                enable_tls: false,
            },
            Arc::clone(&level),
            level_callback,
            reload_callback,
            stats,
        )
        .expect("control plane starts");

        let body = br#"{"level":"ERROR"}"#;
        let request = format!(
            "POST /level HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).expect("request body is utf8")
        );
        let mut stream = TcpStream::connect(control_plane.local_addr()).expect("connect");
        stream.write_all(request.as_bytes()).expect("request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        control_plane.shutdown();

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(
            applied.load(Ordering::Acquire),
            crate::record::LogLevel::Error as u8
        );
        assert_eq!(
            level
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_str(),
            "ERROR"
        );
    }

    #[test]
    fn remote_control_plane_bind_is_rejected_without_transport_security() {
        let result = ControlPlane::start_with_stats(
            ControlPlaneConfig {
                enabled: true,
                bind_addr: "0.0.0.0:0".to_string(),
                enable_tls: false,
            },
            Arc::new(Mutex::new("INFO".to_string())),
            Arc::new(Mutex::new(None)),
            Arc::new(ControlPlaneStats::new()),
        );
        let error = result.err().expect("remote bind must be refused");
        assert!(error.contains("loopback"));
    }

    #[test]
    fn snapshot_health_and_fill_helpers_are_deterministic() {
        let stats = ControlPlaneStats::new();
        stats.configure("test", 10, false);
        stats.record_accepted();
        stats.set_ring_fill_counts(5, 10);
        let snapshot = stats.snapshot("INFO");
        assert_eq!(snapshot.total_records(), 1);
        assert!(snapshot.is_healthy());
        assert!((snapshot.ring_fill_ratio() - 0.5).abs() < f64::EPSILON);
        stats.record_error();
        assert!(!stats.snapshot("INFO").is_healthy());
    }
    #[test]
    fn json_helpers_reject_invalid_levels_and_escape_strings() {
        assert!(is_valid_level("INFO"));
        assert!(!is_valid_level("info"));
        assert_eq!(escape_json("a\"b\n"), "a\\\"b\\n");
        assert_eq!(
            extract_json_string(r#"{"level":"WARN"}"#, "level"),
            Some("WARN")
        );
    }
}
