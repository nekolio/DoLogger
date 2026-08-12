//! Platform-native I/O — direct syscalls, no libc stdio buffering.
//!
//! All I/O MUST use async mechanisms (io_uring/IOCP/kqueue) in
//! production. The current implementation provides direct `write`/`WriteFile`
//! syscalls; async upgrade is deferred to a later milestone.
//!
//! All log output routes through IOSink; system diagnostics use the
//! sysmon self-monitoring channel or the internal diagnostic log.

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
    // Use Windows WriteFile directly — no CRT stdio
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
    }
    const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5u32; // -11
    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4u32; // -12

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

    let mut written: u32 = 0;
    // SAFETY: WriteFile writes to a valid stdout/stderr handle obtained above.
    // buf.as_ptr() and buf.len() are valid for the lifetime of this call.
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

#[cfg(not(windows))]
fn raw_write(fd: i32, buf: &[u8]) {
    unsafe {
        libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len());
    }
}
