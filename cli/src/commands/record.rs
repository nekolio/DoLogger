//! SIF recording and replay commands for `dologctl`.
//!
//! SIF (Standard Intermediate Format) record generation, replay,
//! and recording session control for offline analysis and testing.
//!
//! # Commands
//!
//! | Command        | Description |
//! |----------------|-------------|
//! | `record`       | Generate synthetic test records, write SIF file with framing |
//! | `replay`       | Read SIF file, print record summaries with configurable speed |
//! | `record-stop`  | Manage recording PID file in temp directory |

use std::fs;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dologger_core::record::wire::{decode_any, DecodeOptions};
use dologger_core::record::{LogLevel, Record};
use dologger_core::sif::encode_record;

use crate::output::{self, color, OutputFormat};
use crate::{stderr, stdout, EXIT_ERR};

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

fn green() -> &'static str {
    output::when_color(color::GREEN)
}
fn red() -> &'static str {
    output::when_color(color::RED)
}
fn yellow() -> &'static str {
    output::when_color(color::YELLOW)
}
fn cyan() -> &'static str {
    output::when_color(color::CYAN)
}
fn magenta() -> &'static str {
    output::when_color(color::MAGENTA)
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
fn bright_magenta() -> &'static str {
    output::when_color(color::BRIGHT_MAGENTA)
}

// ---------------------------------------------------------------------------
// SIF record generation
// ---------------------------------------------------------------------------

/// Default records per second when generating synthetic data.
const DEFAULT_RECORDS_PER_SEC: u64 = 100;

/// Generate a synthetic SIF record with the given LSN, timestamp, level, and message.
///
/// Uses the canonical [`dologger_core::sif::encode_record`] FlatBuffer encoding —
/// the same wire format the core shm sink emits and `dologctl replay`/`verify-log`
/// consume.
fn generate_sif_record(lsn: u64, timestamp_ms: u64, level: u8, message: &str) -> Vec<u8> {
    let mut rec = Record::new(0);
    rec.lsn = lsn;
    // Timestamp: u64 nanoseconds since UNIX epoch.
    rec.timestamp = timestamp_ms * 1_000_000;
    rec.level = LogLevel::from_u8(level).unwrap_or(LogLevel::Info);
    rec.thread_id = 1;
    rec.process_id = std::process::id();
    rec.message.set(message);
    encode_record(&rec)
}

// ===========================================================================
// cmd_record — generate synthetic test records
// ===========================================================================

/// `dologctl record <domain> --output <path> [--duration <seconds>]`
pub fn cmd_record(domain: &str, output: &str, duration: u64, format: OutputFormat) {
    let total_records = duration * DEFAULT_RECORDS_PER_SEC;

    if format == OutputFormat::Json {
        cmd_record_json(domain, output, duration, total_records);
        return;
    }

    let b = bold();
    let bm = bright_magenta();
    let d = dim();
    let c = cyan();
    let bg = bright_green();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bm}SIF Record Generator{reset}");
    stdout!("{d}─────────────────────{reset}");
    stdout!("  Domain:    {domain}");
    stdout!("  Output:    {output}");
    stdout!("  Duration:  {duration}s");
    stdout!("");

    stdout!("  Generating {c}{total_records}{reset} synthetic records...");

    let output_path = std::path::Path::new(output);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                stderr!(
                    "{r}Error:{reset} Cannot create output directory: {e}",
                    r = red(),
                    reset = reset
                );
                std::process::exit(1);
            }
        }
    }

    let file = match fs::File::create(output) {
        Ok(f) => f,
        Err(e) => {
            stderr!(
                "{r}Error:{reset} Cannot create output file '{output}': {e}",
                r = red(),
                reset = reset
            );
            std::process::exit(1);
        }
    };

    let mut writer = BufWriter::with_capacity(65536, file);
    let mut total_bytes: u64 = 0;
    let levels: [(&str, u8); 7] = [
        ("TRACE", 0),
        ("DEBUG", 1),
        ("INFO", 2),
        ("WARN", 3),
        ("ERROR", 4),
        ("FATAL", 5),
        ("AUDIT", 6),
    ];

    let start = Instant::now();

    for i in 0..total_records {
        let lsn = i + 1;
        let timestamp_ms = i * (duration * 1000) / total_records;
        let (level_name, level_code) = levels[(i as usize) % levels.len()];
        let message = format!(
            "[{domain}] [{level_name}] Synthetic record #{lsn}: test message for verification and replay"
        );

        let sif = generate_sif_record(lsn, timestamp_ms, level_code, &message);

        let frame_len = sif.len() as u32;
        let len_bytes = frame_len.to_le_bytes();
        writer.write_all(&len_bytes).ok();
        writer.write_all(&sif).ok();
        total_bytes += 4 + sif.len() as u64;

        if i > 0 && i % 1000 == 0 {
            let pct = i as f64 / total_records as f64 * 100.0;
            stdout!("    {d}[{pct:.0}%] {i}/{total_records} records...{reset}");
        }
    }

    writer.flush().ok();

    let elapsed = start.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    let throughput = total_records as f64 / elapsed_secs;

    stdout!("");
    stdout!("{b}Generation Complete{reset}");
    stdout!("{d}──────────────────{reset}");
    stdout!("  Records:        {c}{total_records}{reset}");
    stdout!(
        "  Total bytes:    {} ({:.1} KiB)",
        total_bytes,
        total_bytes as f64 / 1024.0
    );
    stdout!("  Wall time:      {:.3}s", elapsed_secs);
    stdout!("  Throughput:     {bg}{throughput:.0}{reset} rec/s");
    stdout!(
        "  Avg record:     {:.1} bytes",
        total_bytes as f64 / total_records as f64
    );
    stdout!("  Output file:    {d}{output}{reset}");
}

fn cmd_record_json(domain: &str, output: &str, duration: u64, total_records: u64) {
    let output_path = std::path::Path::new(output);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                let obj = serde_json::json!({"status": "error", "error_code": EXIT_ERR, "message": format!("Cannot create directory: {e}")});
                output::stdout_line(&obj.to_string());
                std::process::exit(EXIT_ERR);
            }
        }
    }

    let file = match fs::File::create(output) {
        Ok(f) => f,
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_ERR, "message": format!("Cannot create file: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_ERR);
        }
    };

    let mut writer = BufWriter::with_capacity(65536, file);
    let mut total_bytes: u64 = 0;
    let levels: [(&str, u8); 7] = [
        ("TRACE", 0),
        ("DEBUG", 1),
        ("INFO", 2),
        ("WARN", 3),
        ("ERROR", 4),
        ("FATAL", 5),
        ("AUDIT", 6),
    ];

    let start = Instant::now();
    for i in 0..total_records {
        let lsn = i + 1;
        let timestamp_ms = i * (duration * 1000) / total_records;
        let (level_name, level_code) = levels[(i as usize) % levels.len()];
        let message = format!(
            "[{domain}] [{level_name}] Synthetic record #{lsn}: test message for verification and replay"
        );
        let sif = generate_sif_record(lsn, timestamp_ms, level_code, &message);
        let frame_len = sif.len() as u32;
        writer.write_all(&frame_len.to_le_bytes()).ok();
        writer.write_all(&sif).ok();
        total_bytes += 4 + sif.len() as u64;
    }
    writer.flush().ok();

    let elapsed = start.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    let throughput = total_records as f64 / elapsed_secs;

    let obj = serde_json::json!({
        "status": "ok",
        "domain": domain,
        "output_file": output,
        "duration_secs": duration,
        "total_records": total_records,
        "total_bytes": total_bytes,
        "wall_time_secs": elapsed_secs,
        "throughput_rec_per_sec": throughput
    });
    output::stdout_line(&obj.to_string());
}

// ===========================================================================
// cmd_replay — read SIF file and print record summaries
// ===========================================================================

/// `dologctl replay <input> [--speed <speed>]`
pub fn cmd_replay(input: &str, speed: &str, format: OutputFormat) {
    if format == OutputFormat::Json {
        cmd_replay_json(input, speed);
        return;
    }

    let real_time = speed == "1" || speed.eq_ignore_ascii_case("realtime");

    let b = bold();
    let bc = bright_cyan();
    let d = dim();
    let c = cyan();
    let bg = bright_green();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bc}SIF Replay{reset}");
    stdout!("{d}──────────{reset}");
    stdout!("  Input:     {input}");
    stdout!(
        "  Speed:     {}",
        if real_time {
            format!("{c}real-time (1x){reset}")
        } else {
            format!("{bg}max (no stall){reset}")
        }
    );
    stdout!("");

    // Read the entire file
    let data = match fs::read(input) {
        Ok(d) => d,
        Err(e) => {
            stderr!(
                "{r}Error:{reset} Cannot read '{input}': {e}",
                r = red(),
                reset = reset
            );
            std::process::exit(1);
        }
    };

    if data.is_empty() {
        stdout!("{d}Empty file — nothing to replay.{reset}");
        return;
    }

    // Parse framed records
    let level_names: [&str; 7] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL", "AUDIT"];
    let level_colors: [&str; 7] = [d, c, green(), yellow(), red(), magenta(), bright_magenta()];

    let mut offset: usize = 0;
    let mut count: u64 = 0;
    let mut prev_timestamp_ms: Option<u64> = None;
    let replay_start = Instant::now();

    while offset + 4 <= data.len() {
        let frame_len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;

        if frame_len == 0 {
            offset += 4;
            continue;
        }

        let payload_start = offset + 4;
        let payload_end = payload_start + frame_len;

        if payload_end > data.len() {
            break;
        }

        let payload = &data[payload_start..payload_end];

        // Decode the canonical SIF frame for display.
        let rec = match decode_any(payload, DecodeOptions::default()) {
            Ok((record, _kind)) => record,
            Err(_) => {
                offset = payload_end;
                continue;
            }
        };

        let lsn = rec.lsn;
        let ts_nanos = rec.timestamp;
        let level = rec.level as usize;
        let tid = rec.thread_id;
        let pid = rec.process_id;
        let message = rec.message.display_lossy().into_owned();
        let source_file = rec.source_file();
        let host_name = rec.host_name();

        let timestamp_ms = ts_nanos / 1_000_000;
        let level_name = level_names.get(level).copied().unwrap_or("?");
        let level_color = level_colors.get(level).copied().unwrap_or(reset);

        // Real-time stalling based on timestamp delta
        if real_time {
            if let Some(prev_ts) = prev_timestamp_ms {
                if timestamp_ms > prev_ts {
                    let elapsed_since_start = replay_start.elapsed();
                    let expected_elapsed = Duration::from_millis(timestamp_ms);
                    if expected_elapsed > elapsed_since_start {
                        let to_sleep = expected_elapsed - elapsed_since_start;
                        if to_sleep < Duration::from_secs(1) {
                            std::thread::sleep(to_sleep);
                        }
                    }
                }
            }
            prev_timestamp_ms = Some(timestamp_ms);
        }

        // Print record summary
        stdout!(
            "  [{c}{lsn:>6}{reset}] [{level_color}{level_name:<5}{reset}] tid={tid} pid={pid} [{d}{source_file}{reset}] [{host_name}] {message}"
        );

        count += 1;
        offset = payload_end;
    }

    let elapsed = replay_start.elapsed();

    stdout!("");
    stdout!("{b}Replay Complete{reset}");
    stdout!("{d}───────────────{reset}");
    stdout!("  Records replayed:  {c}{count}{reset}");
    stdout!("  Time elapsed:      {:.3}s", elapsed.as_secs_f64());
    if count > 0 {
        stdout!(
            "  Replay rate:       {bg}{:.0}{reset} rec/s",
            count as f64 / elapsed.as_secs_f64()
        );
    }
}

fn cmd_replay_json(input: &str, speed: &str) {
    let real_time = speed == "1" || speed.eq_ignore_ascii_case("realtime");

    let data = match fs::read(input) {
        Ok(d) => d,
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_ERR, "message": format!("Cannot read file: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_ERR);
        }
    };

    let mut offset: usize = 0;
    let mut count: u64 = 0;
    let replay_start = Instant::now();

    while offset + 4 <= data.len() {
        let frame_len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        if frame_len == 0 {
            offset += 4;
            continue;
        }
        let payload_start = offset + 4;
        let payload_end = payload_start + frame_len;
        if payload_end > data.len() {
            break;
        }
        count += 1;
        offset = payload_end;
    }

    let elapsed = replay_start.elapsed();

    let obj = serde_json::json!({
        "status": "ok",
        "input_file": input,
        "speed": if real_time { "real-time" } else { "max" },
        "records_replayed": count,
        "time_elapsed_secs": elapsed.as_secs_f64(),
        "replay_rate_rec_per_sec": if count > 0 { count as f64 / elapsed.as_secs_f64() } else { 0.0 }
    });
    output::stdout_line(&obj.to_string());
}

// ===========================================================================
// cmd_record_stop — manage recording PID file
// ===========================================================================

/// `dologctl record-stop <domain>`
pub fn cmd_record_stop(domain: &str, format: OutputFormat) {
    if format == OutputFormat::Json {
        cmd_record_stop_json(domain);
        return;
    }

    let b = bold();
    let d = dim();
    let g = green();
    let y = yellow();
    let c = cyan();
    let r = red();
    let reset = output::when_color(color::RESET);

    stdout!("{b}Recording Session Status{reset}");
    stdout!("{d}────────────────────────{reset}");
    stdout!("  Domain: {domain}");

    let pid_path = recording_pid_path(domain);

    stdout!("  PID file: {d}{}{reset}", pid_path.display());

    match fs::read_to_string(&pid_path) {
        Ok(content) => {
            let pid_str = content.trim();
            stdout!("  Stored PID: {c}{pid_str}{reset}");

            #[cfg(unix)]
            {
                let pid: libc::pid_t = match pid_str.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        stderr!("{y}Warning:{reset} Invalid PID in file");
                        stdout!("  Status: {y}UNKNOWN{reset} (invalid PID data)");
                        return;
                    }
                };
                let ret = unsafe { libc::kill(pid, 0) };
                if ret == 0 {
                    stdout!("  Status: {g}RUNNING{reset} — recording process is active");
                } else {
                    let err = std::io::Error::last_os_error();
                    stdout!("  Status: {y}STALE{reset} — PID {pid_str} is not running ({err})");
                    stdout!(
                        "  Hint: Remove stale PID file: {d}rm {}{reset}",
                        pid_path.display()
                    );
                }
            }

            #[cfg(not(unix))]
            {
                let pid_u32: u32 = match pid_str.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        stderr!("{y}Warning:{reset} Invalid PID in file");
                        stdout!("  Status: {y}UNKNOWN{reset} (invalid PID data)");
                        return;
                    }
                };
                let exists = check_windows_process(pid_u32);
                if exists {
                    stdout!("  Status: {g}RUNNING{reset} — recording process exists");
                } else {
                    stdout!("  Status: {y}STALE{reset} — PID {pid_str} is not running");
                    stdout!(
                        "  Hint: Remove stale PID file: {d}del {}{reset}",
                        pid_path.display()
                    );
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            stdout!("  Status: {d}NOT RECORDING{reset} — no recording session found");
        }
        Err(e) => {
            stderr!("{r}Error:{reset} Cannot read PID file: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_record_stop_json(domain: &str) {
    let pid_path = recording_pid_path(domain);

    let (status, pid_str) = match fs::read_to_string(&pid_path) {
        Ok(content) => {
            let pid_str = content.trim().to_string();
            #[cfg(unix)]
            {
                let status = match pid_str.parse::<libc::pid_t>() {
                    Ok(p) if unsafe { libc::kill(p, 0) } == 0 => "running".to_string(),
                    Ok(_) => "stale".to_string(),
                    Err(_) => "unknown".to_string(),
                };
                (status, pid_str)
            }
            #[cfg(not(unix))]
            {
                let status = match pid_str.parse::<u32>() {
                    Ok(p) if check_windows_process(p) => "running".to_string(),
                    Ok(_) => "stale".to_string(),
                    Err(_) => "unknown".to_string(),
                };
                (status, pid_str)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ("not_recording".to_string(), String::new())
        }
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_ERR, "message": format!("Cannot read PID file: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_ERR);
        }
    };

    let mut obj = serde_json::json!({
        "domain": domain,
        "pid_file": pid_path.to_string_lossy(),
        "session_status": status
    });
    if !pid_str.is_empty() {
        obj["pid"] = serde_json::Value::String(pid_str);
    }
    output::stdout_line(&obj.to_string());
}

/// Return the path to the recording PID file for a given domain.
fn recording_pid_path(domain: &str) -> PathBuf {
    let sanitized = domain.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let mut path = std::env::temp_dir();
    path.push(format!("dologger-record-{sanitized}.pid"));
    path
}

/// Check if a Windows process with the given PID exists.
#[cfg(windows)]
fn check_windows_process(pid: u32) -> bool {
    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
        fn CloseHandle(hObject: isize) -> i32;
        fn GetExitCodeProcess(hProcess: isize, lpExitCode: *mut u32) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 || handle == -1 {
        return false;
    }

    let mut exit_code: u32 = 0;
    let ret = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
    unsafe { CloseHandle(handle) };

    ret != 0 && exit_code == STILL_ACTIVE
}

// Call sites are gated on `not(unix)`, so the stub only compiles for
// neither-windows-nor-unix targets — on Unix it would be dead code.
#[cfg(all(not(windows), not(unix)))]
fn check_windows_process(_pid: u32) -> bool {
    false
}
