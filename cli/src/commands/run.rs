//! Run command with `--trace` support for `dologctl`.
//!
//! When `--trace` is set, the run command creates a minimal Engine,
//! submits a batch of test records, and reports per-record pipeline stage
//! timing with a summary at the end.

use std::path::PathBuf;
use std::time::Instant;

use dologger_core::config::DologgerConfig;
use dologger_core::record::{thread_id_u64, LogLevel};
use dologger_core::sys::TimeSource;
use dologger_core::Engine;

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
pub fn cmd_run_trace(config_path: Option<&str>) {
    let config = load_run_config(config_path);

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
            record.id = ts.next_id();
            record.timestamp = ts.now_utc();
            record.level = LogLevel::Info;
            record.message.set(msg);
            record.thread_id = tid;
            record.process_id = pid;
            record.process_name.set("dologctl-trace");
            record.host_name.set("localhost");
            record.environment.set("trace");
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
