//! Shared memory management commands for `dologctl`.
//!
//! Cross-platform shared memory inspection and cleanup for the
//! DoLogger shared memory ring buffer (`sink_shm`).
//!
//! # Commands
//!
//! | Command      | Description |
//! |--------------|-------------|
//! | `shm status` | Open shared memory region read-only, display header fields |
//! | `shm clear`  | Cleanup orphaned SHM — unlink if producer dead, require --force if alive |
//!
//! Header inspection is delegated to `dologger_core::sink::shm::read_status`,
//! the single source of truth for the shared-memory header layout. The CLI no
//! longer maintains a hand-written mirror of the header.

use dologger_core::sink::shm::read_status;
use dologger_core::sink::{
    FLAG_BUFFER_OVERFLOW, FLAG_PRODUCER_ALIVE, FLAG_PRODUCER_DEAD, SHM_MAGIC, SHM_VERSION,
};

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

// ===========================================================================
// cmd_shm_status — display shared memory region metadata
// ===========================================================================

/// `dologctl shm status <path>`
pub fn cmd_shm_status(path: &str, format: OutputFormat) {
    if format == OutputFormat::Json {
        cmd_shm_status_json(path);
        return;
    }

    let bg = bright_cyan();
    let b = bold();
    let d = dim();
    let g = green();
    let r = red();
    let y = yellow();
    let c = cyan();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bg}Shared Memory Status{reset}");
    stdout!("{d}─────────────────────{reset}");
    stdout!("  Path: {path}");

    let status = match read_status(path) {
        Ok(s) => {
            stdout!("  Status: {g}OPENED{reset} — region mapped read-only");
            s
        }
        Err(e) => {
            stderr!("{r}Error:{reset} Cannot open shared memory '{path}': {e}");
            stdout!("");
            stdout!("{d}Hint: The shared memory region may not exist. Check:{reset}");
            stdout!("{d}  - Is dologger running with sink_shm configured?{reset}");
            stdout!("{d}  - Is the path correct? (e.g., /dologger_default.shm){reset}");
            stdout!("{d}  - On Windows, the name is case-insensitive.{reset}");
            std::process::exit(1);
        }
    };

    // Header validation
    stdout!("");
    stdout!("{b}ShmHeader{reset}");
    stdout!("{d}─────────{reset}");

    let magic_str = if status.magic == SHM_MAGIC {
        format!("{g}0x{:08X}{reset}", status.magic)
    } else {
        format!(
            "{r}0x{:08X} (expected 0x{SHM_MAGIC:08X}){reset}",
            status.magic
        )
    };
    stdout!("  Magic:         {magic_str}");

    let ver_str = if status.version == SHM_VERSION {
        format!("{g}{}{reset}", status.version)
    } else {
        format!("{y}{} (expected {SHM_VERSION}){reset}", status.version)
    };
    stdout!("  Version:       {ver_str}");

    let buffer_size = status.buffer_size_bytes;
    stdout!(
        "  Buffer size:   {} ({:.1} MiB)",
        buffer_size,
        buffer_size as f64 / (1024.0 * 1024.0)
    );

    stdout!(
        "  Slots:         {} x {} bytes",
        status.slot_count,
        status.slot_size_bytes
    );
    stdout!("  Producer seq:  {c}{}{reset}", status.producer_seq);
    stdout!("  Consumer seq:  {c}{}{reset}", status.consumer_seq);
    stdout!("  Dropped:       {y}{}{reset}", status.dropped_count);
    stdout!("  Overwritten:   {y}{}{reset}", status.overwritten_count);
    stdout!("  Producer PID:  {}", status.producer_pid);

    // Compute fill percentage
    let fill = if status.slot_count > 0 {
        let in_flight = status.producer_seq.saturating_sub(status.consumer_seq);
        let fill_pct = (in_flight as f64 / status.slot_count as f64 * 100.0).min(100.0);
        let tcolor = if fill_pct > 90.0 {
            r
        } else if fill_pct > 70.0 {
            y
        } else {
            g
        };
        format!(
            "{tcolor}{fill_pct:.1}%{reset} ({} / {} slots)",
            in_flight, status.slot_count
        )
    } else {
        format!("{y}N/A (0 slots){reset}")
    };
    stdout!("  Fill:          {fill}");

    // Flags interpretation
    stdout!("  Flags:         0x{:08X}", status.flags);

    let alive = status.flags & FLAG_PRODUCER_ALIVE != 0;
    let dead = status.flags & FLAG_PRODUCER_DEAD != 0;
    let overflow = status.flags & FLAG_BUFFER_OVERFLOW != 0;

    let alive_str = if alive {
        format!("{g}ALIVE{reset}")
    } else if dead {
        format!("{r}DEAD{reset}")
    } else {
        format!("{y}UNKNOWN{reset}")
    };
    stdout!("  Producer:      {alive_str}");

    if overflow {
        stdout!("  Overflow:      {r}YES{reset} — buffer has overflowed");
    }

    // System-level process check for the producer PID
    if status.producer_pid > 0 {
        let proc_alive = check_process_alive(status.producer_pid);
        let proc_str = if proc_alive {
            format!("{g}Process {} is running{reset}", status.producer_pid)
        } else if alive && !dead {
            format!(
                "{r}Process {} is NOT running (stale ALIVE flag?){reset}",
                status.producer_pid
            )
        } else {
            format!("{d}Process {} is not running{reset}", status.producer_pid)
        };
        stdout!("  Process check: {proc_str}");
    }
}

fn cmd_shm_status_json(path: &str) {
    let status = match read_status(path) {
        Ok(s) => s,
        Err(e) => {
            let obj = serde_json::json!({"status": "error", "error_code": EXIT_ERR, "message": format!("Cannot open SHM: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(EXIT_ERR);
        }
    };

    let in_flight = status.producer_seq.saturating_sub(status.consumer_seq);
    let fill_pct = if status.slot_count > 0 {
        (in_flight as f64 / status.slot_count as f64 * 100.0).min(100.0)
    } else {
        0.0
    };

    let obj = serde_json::json!({
        "status": "ok",
        "path": status.path,
        "magic": format!("0x{:08X}", status.magic),
        "magic_valid": status.magic == SHM_MAGIC,
        "version": status.version,
        "version_expected": SHM_VERSION,
        "buffer_size_bytes": status.buffer_size_bytes,
        "buffer_size_mib": status.buffer_size_bytes as f64 / (1024.0 * 1024.0),
        "slot_count": status.slot_count,
        "slot_size_bytes": status.slot_size_bytes,
        "producer_seq": status.producer_seq,
        "consumer_seq": status.consumer_seq,
        "in_flight": in_flight,
        "fill_percent": fill_pct,
        "dropped_count": status.dropped_count,
        "overwritten_count": status.overwritten_count,
        "producer_pid": status.producer_pid,
        "flags": format!("0x{:08X}", status.flags),
        "producer_alive": status.flags & FLAG_PRODUCER_ALIVE != 0,
        "producer_dead": status.flags & FLAG_PRODUCER_DEAD != 0,
        "buffer_overflow": status.flags & FLAG_BUFFER_OVERFLOW != 0,
        "process_alive": check_process_alive(status.producer_pid)
    });
    output::stdout_line(&obj.to_string());
}

/// Cross-platform check if a process with the given PID is alive.
fn check_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(windows)]
    {
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
}

// ===========================================================================
// cmd_shm_clear — cleanup orphaned shared memory
// ===========================================================================

/// Unlink (destroy) the shared memory object.
fn shm_unlink(name: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let name_c = CString::new(name).map_err(|e| format!("SHM name '{name}': {e}"))?;
        let ret = unsafe { libc::shm_unlink(name_c.as_ptr()) };
        if ret != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(2) {
                return Err(format!("shm_unlink('{name}'): {e}"));
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = name;
    }
    Ok(())
}

/// `dologctl shm clear <path> [--force]`
pub fn cmd_shm_clear(path: &str, force: bool, format: OutputFormat) {
    if format == OutputFormat::Json {
        cmd_shm_clear_json(path, force);
        return;
    }

    let b = bold();
    let d = dim();
    let r = red();
    let y = yellow();
    let bg = bright_green();
    let reset = output::when_color(color::RESET);

    stdout!("{b}Shared Memory Cleanup{reset}");
    stdout!("{d}─────────────────────{reset}");
    stdout!("  Path:  {path}");
    stdout!("  Force: {force}");
    stdout!("");

    let status = match read_status(path) {
        Ok(s) => s,
        Err(e) => {
            stderr!("{d}Cannot open '{path}': {e}{reset}");
            stdout!("");
            stdout!("{d}The shared memory region may already be cleaned up.{reset}");
            return;
        }
    };

    let alive = status.flags & FLAG_PRODUCER_ALIVE != 0;
    let dead = status.flags & FLAG_PRODUCER_DEAD != 0;
    let proc_alive = if status.producer_pid > 0 {
        check_process_alive(status.producer_pid)
    } else {
        false
    };

    stdout!("  Producer PID:   {}", status.producer_pid);
    stdout!("  Producer alive: {alive}");
    stdout!("  Producer dead:  {dead}");
    stdout!("  Process alive:  {proc_alive}");
    stdout!("");

    if dead || !proc_alive {
        match shm_unlink(path) {
            Ok(()) => {
                stdout!("{bg}{b}Cleaned up{reset}{bg} '{path}' unlinked successfully.{reset}");
            }
            Err(e) => {
                stderr!("{r}Error:{reset} {e}");
                std::process::exit(1);
            }
        }
    } else if force {
        stdout!(
            "{y}{b}Warning:{reset}{y} Producer is ALIVE (PID {}).{reset}",
            status.producer_pid
        );
        stdout!("{y}Forcing cleanup with --force...{reset}");

        match shm_unlink(path) {
            Ok(()) => {
                stdout!("{bg}{b}Cleaned up{reset}{bg} '{path}' unlinked (forced).{reset}");
                stdout!("{y}Note: The running producer will lose its shared memory region.{reset}");
            }
            Err(e) => {
                stderr!("{r}Error:{reset} {e}");
                std::process::exit(1);
            }
        }
    } else {
        stderr!(
            "{r}{b}Aborted:{reset}{r} Producer is ALIVE (PID {}).{reset}",
            status.producer_pid
        );
        stderr!("{r}Use --force to override and forcefully clean up.{reset}");
        stderr!("{r}WARNING: Forcing cleanup while the producer is running{reset}");
        stderr!("{r}         may cause data loss or crashes in the producer.{reset}");
        std::process::exit(1);
    }
}

fn cmd_shm_clear_json(path: &str, force: bool) {
    let status = match read_status(path) {
        Ok(s) => s,
        Err(_) => {
            // Already cleaned up
            let obj = serde_json::json!({"status": "already_cleaned", "path": path});
            output::stdout_line(&obj.to_string());
            return;
        }
    };

    let alive = status.flags & FLAG_PRODUCER_ALIVE != 0;
    let dead = status.flags & FLAG_PRODUCER_DEAD != 0;
    let proc_alive = check_process_alive(status.producer_pid);

    if dead || !proc_alive || force {
        match shm_unlink(path) {
            Ok(()) => {
                let obj = serde_json::json!({
                    "status": "cleaned",
                    "path": path,
                    "forced": force && alive && proc_alive,
                    "producer_pid": status.producer_pid
                });
                output::stdout_line(&obj.to_string());
            }
            Err(e) => {
                let obj =
                    serde_json::json!({"status": "error", "error_code": EXIT_ERR, "message": e});
                output::stdout_line(&obj.to_string());
                std::process::exit(EXIT_ERR);
            }
        }
    } else {
        let obj = serde_json::json!({
            "status": "aborted",
            "reason": "producer still alive",
            "path": path,
            "producer_pid": status.producer_pid,
            "hint": "use --force to override"
        });
        output::stdout_line(&obj.to_string());
        std::process::exit(1);
    }
}
