//! Shared memory management commands for `dologctl`.
//!
//! Cross-platform shared memory inspection and cleanup for the
//! DoLogger shared memory ring buffer (`sink_shm`).
//!
//! # Commands
//!
//! | Command      | Description |
//! |--------------|-------------|
//! | `shm status` | Open shared memory region read-only, display ShmHeader fields |
//! | `shm clear`  | Cleanup orphaned SHM — unlink if producer dead, require --force if alive |

use crate::output::{self, color, OutputFormat};
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

// ---------------------------------------------------------------------------
// Shared memory layout constants (matching sink_shm.rs exactly)
// ---------------------------------------------------------------------------

/// Magic number for shared memory validation ("DLOG" = 0x444C4F47 in ASCII).
const SHM_MAGIC: u32 = 0x474F4C44;
/// Current layout version.
const SHM_VERSION: u32 = 1;
/// Header size in bytes.
const SHM_HEADER_SIZE: usize = 64;

/// Producer is alive and writing.
const FLAG_PRODUCER_ALIVE: u32 = 0x00000001;
/// Producer has shut down cleanly.
const FLAG_PRODUCER_DEAD: u32 = 0x00000002;
/// Buffer has overflowed.
const FLAG_BUFFER_OVERFLOW: u32 = 0x00000004;

// ---------------------------------------------------------------------------
// ShmHeader — mirror of sink_shm::ShmHeader (must be byte-identical)
// ---------------------------------------------------------------------------

/// Mirror of the shared memory header layout from `core/src/sink_shm.rs`.
#[repr(C, align(64))]
struct ShmHeaderRead {
    /// Total buffer size in bytes.
    buffer_size_bytes: u64,
    /// Next slot to read (advanced by consumer via CAS).
    consumer_seq: u64,
    /// Next slot to write (advanced by producer).
    producer_seq: u64,
    /// Total records dropped due to buffer full.
    dropped_count: u64,
    /// Total records overwritten (drop_oldest policy).
    overwritten_count: u64,
    /// Magic number (SHM_MAGIC = 0x474F4C44).
    magic: u32,
    /// Layout version (SHM_VERSION = 1).
    version: u32,
    /// Number of slots in the ring buffer.
    slot_count: u32,
    /// Size of each slot in bytes.
    slot_size_bytes: u32,
    /// Producer process ID.
    producer_pid: u32,
    /// Flags bitmask (FLAG_PRODUCER_ALIVE, FLAG_PRODUCER_DEAD, etc.).
    flags: u32,
}

const _SHM_HEADER_SIZE_CHECK: () = assert!(std::mem::size_of::<ShmHeaderRead>() == 64);

/// Read a potentially-atomic field from shared memory with Acquire ordering.
#[inline(always)]
fn atomic_read_u64(val: &u64) -> u64 {
    let v = unsafe { std::ptr::read_volatile(val) };
    std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
    v
}

#[inline(always)]
fn atomic_read_u32(val: &u32) -> u32 {
    let v = unsafe { std::ptr::read_volatile(val) };
    std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
    v
}

// ===========================================================================
// Platform-specific shared memory access
// ===========================================================================

/// Handle for an opened shared memory region (read-only).
enum ShmAccess {
    #[cfg(unix)]
    Unix {
        ptr: *const u8,
        size: usize,
        fd: std::os::unix::io::RawFd,
        // Kept for symmetry with the Windows variant (diagnostic display).
        #[allow(dead_code)]
        name: String,
    },
    #[cfg(windows)]
    Windows {
        ptr: *const u8,
        mapping_handle: isize,
        #[allow(dead_code)]
        name: String,
    },
}

/// Open a shared memory region read-only.
fn shm_open_readonly(name: &str) -> Result<ShmAccess, String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let name_c = CString::new(name).map_err(|e| format!("SHM name '{name}': {e}"))?;

        let fd = unsafe { libc::shm_open(name_c.as_ptr(), libc::O_RDONLY, 0o660) };
        if fd < 0 {
            return Err(format!(
                "shm_open('{name}'): {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut stat) } != 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(format!("fstat('{name}'): {e}"));
        }
        let size = stat.st_size as usize;

        if size < SHM_HEADER_SIZE {
            unsafe { libc::close(fd) };
            return Err(format!(
                "SHM region too small: {size} bytes (need at least {SHM_HEADER_SIZE})"
            ));
        }

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(format!("mmap('{name}'): {e}"));
        }

        Ok(ShmAccess::Unix {
            ptr: ptr as *const u8,
            size,
            fd,
            name: name.to_string(),
        })
    }

    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        extern "system" {
            fn OpenFileMappingW(
                dwDesiredAccess: u32,
                bInheritHandle: i32,
                lpName: *const u16,
            ) -> isize;
            fn MapViewOfFile(
                hFileMappingObject: isize,
                dwDesiredAccess: u32,
                dwFileOffsetHigh: u32,
                dwFileOffsetLow: u32,
                dwNumberOfBytesToMap: usize,
            ) -> *mut u8;
            fn CloseHandle(hObject: isize) -> i32;
        }

        const FILE_MAP_READ: u32 = 0x0004;
        const INVALID_HANDLE_VALUE: isize = -1;

        let wide_name: Vec<u16> = OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, wide_name.as_ptr()) };

        if handle == 0 || handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "OpenFileMappingW('{name}'): {}",
                std::io::Error::last_os_error()
            ));
        }

        let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, SHM_HEADER_SIZE) };

        if ptr.is_null() {
            let e = std::io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(format!("MapViewOfFile('{name}'): {e}"));
        }

        Ok(ShmAccess::Windows {
            ptr: ptr as *const u8,
            mapping_handle: handle,
            name: name.to_string(),
        })
    }
}

/// Get a reference to the ShmHeader from a read-only SHM accessor.
unsafe fn header_from_access(access: &ShmAccess) -> &ShmHeaderRead {
    let ptr = match access {
        #[cfg(unix)]
        ShmAccess::Unix { ptr, .. } => *ptr,
        #[cfg(windows)]
        ShmAccess::Windows { ptr, .. } => *ptr,
    };
    unsafe { &*(ptr as *const ShmHeaderRead) }
}

/// Close and release a shared memory access handle (does NOT unlink).
fn shm_close(access: ShmAccess) {
    match access {
        #[cfg(unix)]
        ShmAccess::Unix { ptr, size, fd, .. } => unsafe {
            libc::munmap(ptr as *mut libc::c_void, size);
            libc::close(fd);
        },
        #[cfg(windows)]
        ShmAccess::Windows {
            ptr,
            mapping_handle,
            ..
        } => {
            extern "system" {
                fn UnmapViewOfFile(lpBaseAddress: *const u8) -> i32;
                fn CloseHandle(hObject: isize) -> i32;
            }
            unsafe {
                UnmapViewOfFile(ptr);
                CloseHandle(mapping_handle);
            }
        }
    }
}

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

    let access = match shm_open_readonly(path) {
        Ok(a) => {
            stdout!("  Status: {g}OPENED{reset} — region mapped read-only");
            a
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

    // SAFETY: access was just opened successfully above and has not been closed.
    let header = unsafe { header_from_access(&access) };

    // Header validation
    stdout!("");
    stdout!("{b}ShmHeader{reset}");
    stdout!("{d}─────────{reset}");

    let magic = atomic_read_u32(&header.magic);
    let magic_str = if magic == SHM_MAGIC {
        format!("{g}0x{magic:08X}{reset}")
    } else {
        format!("{r}0x{magic:08X} (expected 0x{SHM_MAGIC:08X}){reset}")
    };
    stdout!("  Magic:         {magic_str}");

    let version = atomic_read_u32(&header.version);
    let ver_str = if version == SHM_VERSION {
        format!("{g}{version}{reset}")
    } else {
        format!("{y}{version} (expected {SHM_VERSION}){reset}")
    };
    stdout!("  Version:       {ver_str}");

    let buffer_size = atomic_read_u64(&header.buffer_size_bytes);
    stdout!(
        "  Buffer size:   {} ({:.1} MiB)",
        buffer_size,
        buffer_size as f64 / (1024.0 * 1024.0)
    );

    let slot_count = atomic_read_u32(&header.slot_count);
    let slot_size = atomic_read_u32(&header.slot_size_bytes);
    stdout!("  Slots:         {slot_count} x {slot_size} bytes");

    let producer_seq = atomic_read_u64(&header.producer_seq);
    let consumer_seq = atomic_read_u64(&header.consumer_seq);
    stdout!("  Producer seq:  {c}{producer_seq}{reset}");
    stdout!("  Consumer seq:  {c}{consumer_seq}{reset}");

    let dropped = atomic_read_u64(&header.dropped_count);
    let overwritten = atomic_read_u64(&header.overwritten_count);
    stdout!("  Dropped:       {y}{dropped}{reset}");
    stdout!("  Overwritten:   {y}{overwritten}{reset}");

    let producer_pid = atomic_read_u32(&header.producer_pid);
    stdout!("  Producer PID:  {producer_pid}");

    // Compute fill percentage
    let fill = if slot_count > 0 {
        let in_flight = producer_seq.saturating_sub(consumer_seq);
        let fill_pct = (in_flight as f64 / slot_count as f64 * 100.0).min(100.0);
        let tcolor = if fill_pct > 90.0 {
            r
        } else if fill_pct > 70.0 {
            y
        } else {
            g
        };
        format!("{tcolor}{fill_pct:.1}%{reset} ({in_flight} / {slot_count} slots)")
    } else {
        format!("{y}N/A (0 slots){reset}")
    };
    stdout!("  Fill:          {fill}");

    // Flags interpretation
    let flags = atomic_read_u32(&header.flags);
    stdout!("  Flags:         0x{flags:08X}");

    let alive = flags & FLAG_PRODUCER_ALIVE != 0;
    let dead = flags & FLAG_PRODUCER_DEAD != 0;
    let overflow = flags & FLAG_BUFFER_OVERFLOW != 0;

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
    if producer_pid > 0 {
        let proc_alive = check_process_alive(producer_pid);
        let proc_str = if proc_alive {
            format!("{g}Process {producer_pid} is running{reset}")
        } else if alive && !dead {
            format!("{r}Process {producer_pid} is NOT running (stale ALIVE flag?){reset}")
        } else {
            format!("{d}Process {producer_pid} is not running{reset}")
        };
        stdout!("  Process check: {proc_str}");
    }

    // Close the access handle
    shm_close(access);
}

fn cmd_shm_status_json(path: &str) {
    let access = match shm_open_readonly(path) {
        Ok(a) => a,
        Err(e) => {
            let obj =
                serde_json::json!({"status": "error", "message": format!("Cannot open SHM: {e}")});
            output::stdout_line(&obj.to_string());
            std::process::exit(1);
        }
    };

    let header = unsafe { header_from_access(&access) };

    let magic = atomic_read_u32(&header.magic);
    let version = atomic_read_u32(&header.version);
    let buffer_size = atomic_read_u64(&header.buffer_size_bytes);
    let slot_count = atomic_read_u32(&header.slot_count);
    let slot_size = atomic_read_u32(&header.slot_size_bytes);
    let producer_seq = atomic_read_u64(&header.producer_seq);
    let consumer_seq = atomic_read_u64(&header.consumer_seq);
    let dropped = atomic_read_u64(&header.dropped_count);
    let overwritten = atomic_read_u64(&header.overwritten_count);
    let producer_pid = atomic_read_u32(&header.producer_pid);
    let flags = atomic_read_u32(&header.flags);

    let in_flight = producer_seq.saturating_sub(consumer_seq);
    let fill_pct = if slot_count > 0 {
        (in_flight as f64 / slot_count as f64 * 100.0).min(100.0)
    } else {
        0.0
    };

    let obj = serde_json::json!({
        "status": "ok",
        "path": path,
        "magic": format!("0x{magic:08X}"),
        "magic_valid": magic == SHM_MAGIC,
        "version": version,
        "version_expected": SHM_VERSION,
        "buffer_size_bytes": buffer_size,
        "buffer_size_mib": buffer_size as f64 / (1024.0 * 1024.0),
        "slot_count": slot_count,
        "slot_size_bytes": slot_size,
        "producer_seq": producer_seq,
        "consumer_seq": consumer_seq,
        "in_flight": in_flight,
        "fill_percent": fill_pct,
        "dropped_count": dropped,
        "overwritten_count": overwritten,
        "producer_pid": producer_pid,
        "flags": format!("0x{flags:08X}"),
        "producer_alive": flags & FLAG_PRODUCER_ALIVE != 0,
        "producer_dead": flags & FLAG_PRODUCER_DEAD != 0,
        "buffer_overflow": flags & FLAG_BUFFER_OVERFLOW != 0,
        "process_alive": check_process_alive(producer_pid)
    });
    output::stdout_line(&obj.to_string());

    shm_close(access);
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

    let access = match shm_open_readonly(path) {
        Ok(a) => a,
        Err(e) => {
            stderr!("{d}Cannot open '{path}': {e}{reset}");
            stdout!("");
            stdout!("{d}The shared memory region may already be cleaned up.{reset}");
            return;
        }
    };

    // SAFETY: access was just opened successfully.
    let header = unsafe { header_from_access(&access) };
    let flags = atomic_read_u32(&header.flags);
    let producer_pid = atomic_read_u32(&header.producer_pid);

    let alive = flags & FLAG_PRODUCER_ALIVE != 0;
    let dead = flags & FLAG_PRODUCER_DEAD != 0;
    let proc_alive = if producer_pid > 0 {
        check_process_alive(producer_pid)
    } else {
        false
    };

    stdout!("  Producer PID:   {producer_pid}");
    stdout!("  Producer alive: {alive}");
    stdout!("  Producer dead:  {dead}");
    stdout!("  Process alive:  {proc_alive}");
    stdout!("");

    // Close the access handle BEFORE unlinking
    shm_close(access);

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
        stdout!("{y}{b}Warning:{reset}{y} Producer is ALIVE (PID {producer_pid}).{reset}");
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
        stderr!("{r}{b}Aborted:{reset}{r} Producer is ALIVE (PID {producer_pid}).{reset}");
        stderr!("{r}Use --force to override and forcefully clean up.{reset}");
        stderr!("{r}WARNING: Forcing cleanup while the producer is running{reset}");
        stderr!("{r}         may cause data loss or crashes in the producer.{reset}");
        std::process::exit(1);
    }
}

fn cmd_shm_clear_json(path: &str, force: bool) {
    let access = match shm_open_readonly(path) {
        Ok(a) => a,
        Err(_) => {
            // Already cleaned up
            let obj = serde_json::json!({"status": "already_cleaned", "path": path});
            output::stdout_line(&obj.to_string());
            return;
        }
    };

    let header = unsafe { header_from_access(&access) };
    let flags = atomic_read_u32(&header.flags);
    let producer_pid = atomic_read_u32(&header.producer_pid);
    let alive = flags & FLAG_PRODUCER_ALIVE != 0;
    let dead = flags & FLAG_PRODUCER_DEAD != 0;
    let proc_alive = check_process_alive(producer_pid);

    shm_close(access);

    if dead || !proc_alive || force {
        match shm_unlink(path) {
            Ok(()) => {
                let obj = serde_json::json!({
                    "status": "cleaned",
                    "path": path,
                    "forced": force && alive && proc_alive,
                    "producer_pid": producer_pid
                });
                output::stdout_line(&obj.to_string());
            }
            Err(e) => {
                let obj = serde_json::json!({"status": "error", "message": e});
                output::stdout_line(&obj.to_string());
                std::process::exit(1);
            }
        }
    } else {
        let obj = serde_json::json!({
            "status": "aborted",
            "reason": "producer still alive",
            "path": path,
            "producer_pid": producer_pid,
            "hint": "use --force to override"
        });
        output::stdout_line(&obj.to_string());
        std::process::exit(1);
    }
}
