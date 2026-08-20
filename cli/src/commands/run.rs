//! Run command with `--trace` support for `dologctl`.
//!
//! When `--trace` is set, the run command creates a minimal Engine,
//! submits a batch of test records, and reports per-record pipeline stage
//! timing with a summary at the end.

use dologger_core::config::{ConfigWatcher, DologgerConfig};
use dologger_core::record::{thread_id_u64, LogLevel};
use dologger_core::sink::ShmSinkConfig;
use dologger_core::sys::TimeSource;
use dologger_core::Engine;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::perf::format_ns;
use crate::output::{self, color};
use crate::{stderr, stdout};

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

fn green() -> &'static str {
    output::when_color(color::GREEN)
}
fn red() -> &'static str {
    output::when_color(color::RED)
}
fn cyan() -> &'static str {
    output::when_color(color::CYAN)
}
fn yellow() -> &'static str {
    output::when_color(color::YELLOW)
}
fn bold() -> &'static str {
    output::when_color(color::BOLD)
}
fn dim() -> &'static str {
    output::when_color(color::DIM)
}
fn bright_green() -> &'static str {
    output::when_color(color::BRIGHT_GREEN)
}
fn bright_cyan() -> &'static str {
    output::when_color(color::BRIGHT_CYAN)
}

/// Default number of trace records to submit.
const TRACE_RECORD_COUNT: usize = 10;

// ===========================================================================
// Public entry point
// ===========================================================================

/// Run the engine in trace mode: submit records and print per-record
/// pipeline stage timing.
///
/// * `config_path` — optional path to a TOML configuration file.
/// * `shm_path` — optional shared-memory path; enables sink_shm (overriding
///   any `[shm]` path from the config, keeping other `[shm]` fields).
pub fn cmd_run_trace(config_path: Option<&str>, shm_path: Option<&str>) {
    let config = apply_shm_override(load_run_config(config_path), shm_path);

    let b = bold();
    let bc = bright_cyan();
    let d = dim();
    let g = green();
    let r = red();
    let c = cyan();
    let y = yellow();
    let bg = bright_green();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bc}DoLogger Engine — Trace Run{reset}");
    stdout!("{d}──────────────────────────────{reset}");
    stdout!("");

    // --- Initialize engine ---
    stdout!("{d}Initializing engine...{reset}");
    let mut engine = match Engine::init(config) {
        Ok(e) => e,
        Err(e) => {
            stderr!("{r}Error{reset} Engine initialization failed: {e}");
            std::process::exit(1);
        }
    };
    stdout!("{d}Engine ready.  Submitting {TRACE_RECORD_COUNT} trace records...{reset}");
    stdout!("");

    let ts = TimeSource::new();
    let tid = thread_id_u64();
    let pid = std::process::id();

    // Per-record timing data
    struct TraceEntry {
        index: usize,
        message: String,
        push_ns: f64,
        e2e_ns: f64,
    }

    let messages: [&str; 10] = [
        "Application started successfully",
        "Processing incoming request",
        "Database connection pool initialized (32 connections)",
        "Cache miss for key 'user:session:abc123'",
        "Request completed in 45ms — status 200 OK",
        "Rate limiter engaged for client 10.0.1.42",
        "Garbage collection pause — 12ms (threshold: 50ms)",
        "Health check passed — all subsystems GREEN",
        "Configuration hot-reload detected — applying changes",
        "Scheduled maintenance window starts in 300s",
    ];

    let mut entries: Vec<TraceEntry> = Vec::with_capacity(TRACE_RECORD_COUNT);

    for (i, msg) in messages.iter().enumerate().take(TRACE_RECORD_COUNT) {
        // Allocate and fill the record
        let record_ptr = match engine.pool.alloc() {
            Some(ptr) => ptr,
            None => {
                stderr!("{r}Error:{reset} Pool exhausted at record {i}");
                break;
            }
        };

        unsafe {
            let record = &mut *record_ptr;
            let id = ts.next_id();
            record.set_id(id.hi, id.lo);
            record.timestamp = ts.now_nanos();
            record.level = LogLevel::Info;
            record.message.set(msg);
            record.thread_id = tid as u32;
            record.process_id = pid;
            record.set_process_name("dologctl-trace");
            record.set_host_name("localhost");
            record.set_environment("trace");
        }

        // Push to ring buffer — measure submit latency
        let t0 = Instant::now();
        match engine.ring_buffer.try_push(record_ptr) {
            Ok(()) => {
                let push_ns = t0.elapsed().as_nanos() as f64;

                let drain_start = Instant::now();
                while !engine.ring_buffer.is_empty() {
                    if drain_start.elapsed().as_secs() > 5 {
                        stderr!("{y}Warning:{reset} Timeout waiting for pipeline drain");
                        break;
                    }
                    std::hint::spin_loop();
                }
                let drain_elapsed = drain_start.elapsed();
                let e2e_ns = push_ns + drain_elapsed.as_nanos() as f64;

                entries.push(TraceEntry {
                    index: i + 1,
                    message: msg.to_string(),
                    push_ns,
                    e2e_ns,
                });
            }
            Err(ptr) => {
                // SAFETY: ptr came from engine.pool.alloc() on this same pool
                unsafe {
                    engine.pool.free(ptr);
                }
                stderr!("{y}Warning:{reset} Ring buffer full at record {i} — skipped");
            }
        }
    }

    // Wait for final drain
    let drain_timeout = Instant::now();
    while !engine.ring_buffer.is_empty() {
        if drain_timeout.elapsed().as_secs() > 5 {
            stderr!("{y}Warning:{reset} Timeout waiting for final pipeline drain");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // --- Print per-record trace ---
    stdout!("");
    stdout!("{b}Per-Record Trace{reset}");
    stdout!("{d}────────────────{reset}");

    if entries.is_empty() {
        stdout!("{d}No records submitted.{reset}");
    } else {
        for entry in &entries {
            stdout!(
                "  [{idx:>2}] push={c}{push}{reset}  e2e={g}{e2e}{reset}  {msg}",
                idx = entry.index,
                push = format_ns(entry.push_ns),
                e2e = format_ns(entry.e2e_ns),
                msg = entry.message,
            );
        }
    }

    // --- Summary ---
    stdout!("");
    stdout!("{b}Trace Summary{reset}");
    stdout!("{d}─────────────{reset}");

    if entries.is_empty() {
        stdout!("  No records processed.");
    } else {
        let count = entries.len();
        let total_push: f64 = entries.iter().map(|e| e.push_ns).sum();
        let total_e2e: f64 = entries.iter().map(|e| e.e2e_ns).sum();
        let avg_push = total_push / count as f64;
        let avg_e2e = total_e2e / count as f64;
        let min_push = entries
            .iter()
            .map(|e| e.push_ns)
            .fold(f64::INFINITY, f64::min);
        let max_push = entries
            .iter()
            .map(|e| e.push_ns)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_e2e = entries
            .iter()
            .map(|e| e.e2e_ns)
            .fold(f64::INFINITY, f64::min);
        let max_e2e = entries
            .iter()
            .map(|e| e.e2e_ns)
            .fold(f64::NEG_INFINITY, f64::max);

        stdout!("  Records processed:  {count}");
        stdout!("  ── Submit (push → ring buffer) ──");
        stdout!(
            "    Min: {c}{}{reset}   Max: {y}{}{reset}   Avg: {bg}{}{reset}",
            format_ns(min_push),
            format_ns(max_push),
            format_ns(avg_push),
        );
        stdout!("  ── End-to-end (push → sink write) ──");
        stdout!(
            "    Min: {c}{}{reset}   Max: {y}{}{reset}   Avg: {bg}{}{reset}",
            format_ns(min_e2e),
            format_ns(max_e2e),
            format_ns(avg_e2e),
        );
    }

    // --- Shutdown ---
    stdout!("");
    stdout!("{d}Shutting down engine...{reset}");
    engine.shutdown();
    stdout!("{g}Engine shutdown complete.{reset}");
}

/// Flag flipped by the SIGINT / SIGTERM handler.  Static so the C-style
/// signal handler has a stable address; the engine run loop polls it.
static SHUTDOWN_FLAG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Run the engine in normal (steady-state) mode.
///
/// Loads the configuration, initialises the engine, then blocks until a
/// termination signal (SIGINT / SIGTERM on POSIX) is received.  Shutdown
/// is graceful: the pipeline drains, sinks close, and shared-memory
/// auto-cleanup runs if configured.
///
/// On non-Windows platforms we install a tiny `libc::signal` handler that
/// flips the static `SHUTDOWN_FLAG`.  On Windows, no extra dependency is
/// added for console-control handling — the OS default for Ctrl-C
/// terminates the process.
pub fn cmd_run(config_path: Option<&str>, shm_path: Option<&str>) {
    let b = bold();
    let bc = bright_cyan();
    let d = dim();
    let g = green();
    let r = red();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bc}DoLogger Engine{reset}");
    stdout!("{d}──────────────{reset}");
    stdout!("");

    // --- Load config & init engine ---
    let config = apply_shm_override(load_run_config(config_path), shm_path);
    stdout!("{d}Initializing engine...{reset}");
    let engine = match Engine::init(config) {
        Ok(e) => e,
        Err(e) => {
            stderr!("{r}Error:{reset} Engine initialization failed: {e}");
            std::process::exit(crate::EXIT_ERR);
        }
    };

    // Share the engine behind a mutex so the config watcher's background
    // thread can atomically hot-reload it when the config file changes.
    let engine = Arc::new(Mutex::new(engine));
    let _watcher = start_config_watcher(&engine, shm_path.map(|s| s.to_string()));
    stdout!("{g}Engine running.  Press Ctrl-C to stop.{reset}");
    stdout!("");

    // --- Wait for termination signal ---
    install_signal_handler();

    while !SHUTDOWN_FLAG.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // --- Graceful shutdown ---
    stdout!("");
    stdout!("{d}Shutdown signal received.  Draining pipeline...{reset}");
    engine.lock().unwrap().shutdown();
    stdout!("{g}Engine shutdown complete.{reset}");
}

/// Start the config-file watcher for hot reload, if enabled in the engine
/// config. Returns `None` when `[watcher]` is disabled or there is no config
/// file path to watch; on a start failure it logs a warning and returns
/// `None` so the engine still runs without reload.
fn start_config_watcher(
    engine: &Arc<Mutex<Engine>>,
    shm_path: Option<String>,
) -> Option<ConfigWatcher> {
    // Read watcher settings and the active config path from the engine.
    let (watcher_config, watch_path) = {
        let guard = engine.lock().unwrap();
        let path = guard.config.config_path.clone()?;
        (guard.config.watcher.clone(), path)
    };
    if !watcher_config.enabled {
        return None;
    }

    let engine = Arc::clone(engine);
    let watch_for_reload = watch_path.clone();
    let callback = Box::new(move |_path: &Path| {
        // Re-read the active config file and reload. A transient bad edit
        // must NOT terminate the engine: return an error so the previous
        // config is preserved and the watcher keeps watching.
        let content = match std::fs::read_to_string(&watch_for_reload) {
            Ok(c) => c,
            Err(e) => {
                return Err(format!(
                    "Cannot read config '{}': {e}",
                    watch_for_reload.display()
                ))
            }
        };
        let (config, _warnings) =
            match DologgerConfig::parse(&content, Some(PathBuf::from(&watch_for_reload))) {
                Ok(v) => v,
                Err((code, msg)) => {
                    return Err(format!("Config parse failed (err 0x{code:x}): {msg}"))
                }
            };
        let config = apply_shm_override(config, shm_path.as_deref());
        let mut guard = engine.lock().unwrap();
        guard
            .reload_config(config)
            .map_err(|err| format!("Config reload failed (err 0x{:x})", err as u32))
    });

    match ConfigWatcher::start(vec![watch_path], callback, watcher_config) {
        Ok(watcher) => Some(watcher),
        Err(e) => {
            let y = yellow();
            let reset = output::when_color(color::RESET);
            stderr!("{y}Warning:{reset} Config watcher could not start: {e}");
            None
        }
    }
}

/// Install an OS-native SIGINT/SIGTERM handler that flips
/// `SHUTDOWN_FLAG` on receipt.  POSIX only — Windows does not install
/// a handler here to avoid pulling `windows-sys` into the CLI
/// dependency graph.
#[cfg(not(windows))]
fn install_signal_handler() {
    // SAFETY: signal handlers may only call async-signal-safe operations.
    // `AtomicBool::store` is documented by the standard library as safe
    // to call from a C-style signal handler on all supported platforms.
    unsafe extern "C" fn handler(_sig: libc::c_int) {
        SHUTDOWN_FLAG.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    // Install handlers for SIGINT (Ctrl-C) and SIGTERM (service stop).
    unsafe {
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
    }
}

#[cfg(windows)]
fn install_signal_handler() {
    // No-op: Windows OS default for Ctrl-C is to terminate the process.
    // We deliberately do not link `windows-sys` here to keep the CLI
    // dependency graph minimal.
}

// ===========================================================================
// Config loading
// ===========================================================================

/// Load the run configuration from the given path or auto-detect defaults.
fn load_run_config(config_path: Option<&str>) -> DologgerConfig {
    let y = yellow();
    let r = red();
    let reset = output::when_color(color::RESET);

    if let Some(path) = config_path {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                stdout!("Configuration file: {path}");
                match DologgerConfig::parse(&content, Some(PathBuf::from(path))) {
                    Ok((config, warnings)) => {
                        for w in &warnings {
                            stderr!("{y}Warning:{reset} {w}");
                        }
                        config
                    }
                    Err((code, msg)) => {
                        stderr!("{r}Error{reset} (code {code}): {msg}");
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                stderr!("{r}Error:{reset} Cannot read config file '{path}': {e}");
                std::process::exit(1);
            }
        }
    } else {
        let candidates = ["dologger.toml", ".dologger.toml"];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                match std::fs::read_to_string(c) {
                    Ok(content) => {
                        stdout!("Configuration file: {c} (auto-detected)");
                        match DologgerConfig::parse(&content, Some(PathBuf::from(c))) {
                            Ok((config, warnings)) => {
                                for w in &warnings {
                                    stderr!("{y}Warning:{reset} {w}");
                                }
                                return config;
                            }
                            Err((code, msg)) => {
                                stderr!("{r}Error{reset} (code {code}): {msg}");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        stderr!("{r}Error:{reset} Cannot read '{c}': {e}");
                        std::process::exit(1);
                    }
                }
            }
        }

        // No config found — use a trace-friendly dev profile
        stdout!("No configuration file found. Using dev profile defaults.");
        DologgerConfig {
            ring_buffer_size: 65536,
            ..DologgerConfig::dev_profile()
        }
    }
}

/// Apply the `--shm <path>` CLI override: enables sink_shm and overrides the
/// shared-memory path, keeping any other `[shm]` fields from the TOML config
/// (or defaults when the section is absent).
fn apply_shm_override(config: DologgerConfig, shm_path: Option<&str>) -> DologgerConfig {
    match shm_path {
        Some(path) => DologgerConfig {
            shm: Some(ShmSinkConfig {
                path: path.to_string(),
                ..config.shm.clone().unwrap_or_default()
            }),
            ..config
        },
        None => config,
    }
}
