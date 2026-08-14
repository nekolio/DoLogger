//! Platform-native I/O — direct syscalls, no libc stdio buffering.
//!
//! All I/O MUST use async mechanisms (io_uring/IOCP/kqueue) in
//! production. The current implementation provides direct `write`/`WriteFile`
//! syscalls; async upgrade is deferred to a later milestone.
//!
//! All log output routes through the built-in sink layer; system diagnostics
//! use the sysmon self-monitoring channel or the internal diagnostic log.
//!
//! # Text encoding policy
//!
//! Console text on Windows is the one place where bytes ≠ characters:
//! legacy consoles interpret output bytes in their active codepage (GBK/936
//! on zh-CN systems, CP437 on en-US), so writing UTF-8 bytes garbles
//! non-ASCII text. The policy is:
//!
//! - **Auto (default)** — dynamic detection: on a Windows console use the
//!   Unicode console API (`WriteConsoleW`), which renders correctly on any
//!   codepage without mutating the console state; pipes/files and non-Windows
//!   targets always get plain UTF-8 bytes.
//! - **Utf8** — always emit UTF-8 bytes (legacy consoles need `chcp 65001`
//!   to display them correctly).
//! - **Native** — transcode to the console's active codepage when attached
//!   to a legacy console; redirected output stays UTF-8.
//!
//! The engine never changes the console codepage itself
//! (`SetConsoleOutputCP` mutates global console state and would surprise
//! other processes); `WriteConsoleW` makes that unnecessary.
//!
//! Persisted data (log files, WORM records, signed audit bytes) is never
//! transcoded — encoding conversion exists only at this display boundary.

use std::sync::atomic::{AtomicU8, Ordering};

/// Output encoding policy for console text (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OutputEncoding {
    /// Dynamic detection — Unicode console API on Windows consoles,
    /// UTF-8 bytes everywhere else.
    Auto = 0,
    /// Always emit UTF-8 bytes.
    Utf8 = 1,
    /// Transcode to the console codepage on legacy consoles.
    Native = 2,
}

static OUTPUT_ENCODING: AtomicU8 = AtomicU8::new(OutputEncoding::Auto as u8);

/// Set the process-wide output encoding policy.
pub fn set_output_encoding(enc: OutputEncoding) {
    OUTPUT_ENCODING.store(enc as u8, Ordering::Release);
}

/// Get the current output encoding policy.
pub fn output_encoding() -> OutputEncoding {
    match OUTPUT_ENCODING.load(Ordering::Acquire) {
        0 => OutputEncoding::Auto,
        1 => OutputEncoding::Utf8,
        _ => OutputEncoding::Native,
    }
}

/// Write bytes to stdout using a direct syscall (no libc buffering).
pub fn stdout_write(buf: &[u8]) {
    raw_write(1, buf);
}

/// Write bytes to stderr using a direct syscall (no libc buffering).
pub fn stderr_write(buf: &[u8]) {
    raw_write(2, buf);
}

/// Write a string + newline to stdout.
///
/// Uses two syscalls to avoid allocating an intermediate buffer.
/// For batched output, prefer `stdout_write` with pre-formatted data.
pub fn stdout_line(s: &str) {
    raw_write(1, s.as_bytes());
    raw_write(1, b"\n");
}

/// Write a string + newline to stderr.
///
/// Uses two syscalls to avoid allocating an intermediate buffer.
pub fn stderr_line(s: &str) {
    raw_write(2, s.as_bytes());
    raw_write(2, b"\n");
}

// ===========================================================================
// Platform implementations
// ===========================================================================

#[cfg(windows)]
fn raw_write(fd: i32, buf: &[u8]) {
    use std::ffi::c_void;
    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> isize;
        fn WriteFile(
            hFile: isize,
            lpBuffer: *const u8,
            nNumberOfBytesToWrite: u32,
            lpNumberOfBytesWritten: *mut u32,
            lpOverlapped: *mut c_void,
        ) -> i32;
        fn GetConsoleMode(hConsoleHandle: isize, lpMode: *mut u32) -> i32;
        fn GetConsoleOutputCP() -> u32;
        fn WriteConsoleW(
            hConsoleOutput: isize,
            lpBuffer: *const u16,
            nNumberOfCharsToWrite: u32,
            lpNumberOfCharsWritten: *mut u32,
            lpReserved: *mut c_void,
        ) -> i32;
        fn MultiByteToWideChar(
            CodePage: u32,
            dwFlags: u32,
            lpMultiByteStr: *const u8,
            cbMultiByte: i32,
            lpWideCharStr: *mut u16,
            cchWideChar: i32,
        ) -> i32;
        fn WideCharToMultiByte(
            CodePage: u32,
            dwFlags: u32,
            lpWideCharStr: *const u16,
            cchWideChar: i32,
            lpMultiByteStr: *mut u8,
            cbMultiByte: i32,
            lpDefaultChar: *const u8,
            lpUsedDefaultChar: *mut i32,
        ) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5u32; // -11
    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4u32; // -12
    const CP_UTF8: u32 = 65001;

    let std_handle = if fd == 1 {
        STD_OUTPUT_HANDLE
    } else {
        STD_ERROR_HANDLE
    };
    // SAFETY: GetStdHandle is a Win32 API that takes a constant and returns a
    // handle or -1/0 on failure. The input values are well-defined constants.
    let handle = unsafe { GetStdHandle(std_handle) };
    if handle == -1 || handle == 0 {
        return;
    }

    // write_file: plain byte write (UTF-8 path, pipes/files, fallback).
    fn write_file(handle: isize, buf: &[u8]) {
        let mut written: u32 = 0;
        // SAFETY: WriteFile writes to a valid stdout/stderr handle obtained
        // above; buf.as_ptr() and buf.len() are valid for the call duration.
        unsafe {
            WriteFile(
                handle,
                buf.as_ptr(),
                buf.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );
        }
    }

    // write_console_w: Unicode console API — renders correctly on any
    // codepage without mutating the console's global state.
    fn write_console_w(handle: isize, text: &str) -> bool {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut written: u32 = 0;
        // SAFETY: `wide` is a valid UTF-16 buffer for the call duration;
        // lpReserved must be NULL for console handles.
        let ok = unsafe {
            WriteConsoleW(
                handle,
                wide.as_ptr(),
                wide.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        ok != 0
    }

    // Is this handle attached to a console (vs a pipe or file)?
    let mut mode: u32 = 0;
    // SAFETY: GetConsoleMode reads the mode of a valid handle obtained above.
    let is_console = unsafe { GetConsoleMode(handle, &mut mode) } != 0;

    let text = match std::str::from_utf8(buf) {
        Ok(s) => s,
        // Non-UTF-8 bytes: not display text — pass through untouched.
        Err(_) => {
            write_file(handle, buf);
            return;
        }
    };

    match output_encoding() {
        OutputEncoding::Utf8 => write_file(handle, buf),
        OutputEncoding::Native if is_console => {
            // SAFETY: GetConsoleOutputCP takes no arguments.
            let cp = unsafe { GetConsoleOutputCP() };
            if cp == CP_UTF8 || cp == 0 {
                write_file(handle, buf);
                return;
            }
            // Transcode UTF-8 → UTF-16 → console codepage bytes.
            let wide: Vec<u16> = text.encode_utf16().collect();
            // SAFETY: MultiByteToWideChar with CP_UTF8: buffer sizes derived
            // from valid slices; 0 for cbMultiByte means NUL-terminated input
            // — we pass an explicit length variant below instead.
            let wide_len = unsafe {
                MultiByteToWideChar(
                    CP_UTF8,
                    0,
                    buf.as_ptr(),
                    buf.len() as i32,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if wide_len <= 0 {
                write_file(handle, buf);
                return;
            }
            let mut wide_buf = vec![0u16; wide_len as usize];
            // SAFETY: wide_buf has exactly wide_len elements as computed above.
            unsafe {
                MultiByteToWideChar(
                    CP_UTF8,
                    0,
                    buf.as_ptr(),
                    buf.len() as i32,
                    wide_buf.as_mut_ptr(),
                    wide_len,
                );
            }
            // SAFETY: wide_buf is a valid UTF-16 buffer; len 0 query first.
            let mb_len = unsafe {
                WideCharToMultiByte(
                    cp,
                    0,
                    wide_buf.as_ptr(),
                    wide_buf.len() as i32,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                )
            };
            if mb_len <= 0 {
                write_file(handle, buf);
                return;
            }
            let mut mb_buf = vec![0u8; mb_len as usize];
            // SAFETY: mb_buf has exactly mb_len elements as computed above.
            unsafe {
                WideCharToMultiByte(
                    cp,
                    0,
                    wide_buf.as_ptr(),
                    wide_buf.len() as i32,
                    mb_buf.as_mut_ptr(),
                    mb_len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                );
            }
            write_file(handle, &mb_buf);
            let _ = wide;
        }
        OutputEncoding::Auto if is_console => {
            if !write_console_w(handle, text) {
                write_file(handle, buf);
            }
        }
        // Pipes, files, redirected output: UTF-8 bytes always — this keeps
        // JSON consumers and log capture tools byte-stable.
        _ => write_file(handle, buf),
    }
}

#[cfg(not(windows))]
fn raw_write(fd: i32, buf: &[u8]) {
    // SAFETY: `fd` is a valid open file descriptor (stdout/stderr), and `buf`
    // is a valid slice whose pointer and length remain valid for the duration
    // of the `write` call.
    unsafe {
        libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len());
    }
}
