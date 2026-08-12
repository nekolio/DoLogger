//! Shared memory Sink (`sink_shm`).
//!
//! Cross-platform shared memory ring buffer for zero-copy SIF record
//! delivery to external consumers (TUI dashboards, monitoring agents).
//!
//! # Design
//!
//! - Non-persistent: data lives only in RAM, process exit = data gone
//! - `durability_level` forced to `UNSAFE` — no fsync, no persistence
//! - No fallback chain — configuring `fallback` is an error
//! - AUDIT domain FORBIDDEN — must use WORM Sink for audit records
//! - `full_policy`: only `drop_newest` or `drop_oldest` (no `block`)
//! - P99 < 1μs write latency target
//!
//! # Ring buffer layout
//!
//! ```text
//! [ShmHeader (64B)] [Slot 0] [Slot 1] ... [Slot N-1]
//! ```
//!
//! Each slot: [len: u32 LE] [SIF data: len bytes] [padding to slot_size]
//!
//! # Platform support
//!
//! | Platform | API |
//! |----------|-----|
//! | Linux | `shm_open()` + `mmap()` |
//! | macOS | `shm_open()` + `mmap()` |
//! | Windows | `CreateFileMappingW` + `MapViewOfFile` |

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::record::Record;
use crate::sys::diag;
use crate::sys::Sysmon;

// ---------------------------------------------------------------------------
// Shared memory layout constants
// ---------------------------------------------------------------------------

/// Magic number for shared memory validation ("DLOG").
const SHM_MAGIC: u32 = 0x474F4C44;
/// Current layout version.
const SHM_VERSION: u32 = 1;
/// Header size in bytes.
const SHM_HEADER_SIZE: usize = 64;
/// Minimum buffer size in MB (must be power of two).
const MIN_BUFFER_SIZE_MB: usize = 8;
/// Minimum slot size in KB.
const MIN_SLOT_SIZE_KB: usize = 64;
/// Default ring buffer capacity in slots.
const DEFAULT_SLOT_COUNT: usize = 1024;

// ---------------------------------------------------------------------------
// Flags for shared memory header
// ---------------------------------------------------------------------------

/// Producer is alive and writing.
const FLAG_PRODUCER_ALIVE: u32 = 0x00000001;
/// Producer has shut down cleanly.
const FLAG_PRODUCER_DEAD: u32 = 0x00000002;
/// Buffer has overflowed.
const FLAG_BUFFER_OVERFLOW: u32 = 0x00000004;

// ---------------------------------------------------------------------------
// Shared memory header (must match dologger_shm.h exactly)
// ---------------------------------------------------------------------------

/// Header at the start of the shared memory region (64 bytes, cache-line aligned).
///
/// This layout is shared between DoLogger (producer) and external consumers.
/// ALL fields use atomic access for cross-process safety.
#[repr(C, align(64))]
struct ShmHeader {
    /// Total buffer size in bytes
    buffer_size_bytes: u64,
    /// Next slot to read (advanced by consumer via CAS)
    consumer_seq: AtomicU64,
    /// Next slot to write (advanced by producer)
    producer_seq: AtomicU64,
    /// Total records dropped due to buffer full
    dropped_count: AtomicU64,
    /// Total records overwritten (drop_oldest)
    overwritten_count: AtomicU64,
    /// Magic number (SHM_MAGIC)
    magic: u32,
    /// Layout version (SHM_VERSION)
    version: u32,
    /// Number of slots in the ring buffer
    slot_count: u32,
    /// Size of each slot in bytes
    slot_size_bytes: u32,
    /// Producer process ID
    producer_pid: u32,
    /// Flags bitmask
    flags: AtomicU32,
}

const _SHM_HEADER_SIZE_CHECK: () = assert!(std::mem::size_of::<ShmHeader>() == 64);

// SAFETY: ShmHeader is Plain-Old-Data with only atomic fields and no
// internal pointers. It lives in memory-mapped shared memory accessible
// from multiple processes. AtomicU64/AtomicU32 use hardware-level atomic
// instructions, safe for cross-process concurrent access.
unsafe impl Send for ShmHeader {}
// SAFETY: &ShmHeader can be shared across threads because all mutable
// fields use Atomic types with safe interior mutability via Ordering.
unsafe impl Sync for ShmHeader {}

impl ShmHeader {
    fn init(&mut self, buffer_size_bytes: u64, slot_count: u32, slot_size_bytes: u32) {
        self.magic = SHM_MAGIC;
        self.version = SHM_VERSION;
        self.buffer_size_bytes = buffer_size_bytes;
        self.slot_count = slot_count;
        self.slot_size_bytes = slot_size_bytes;
        self.consumer_seq = AtomicU64::new(0);
        self.producer_seq = AtomicU64::new(0);
        self.dropped_count = AtomicU64::new(0);
        self.overwritten_count = AtomicU64::new(0);
        self.producer_pid = std::process::id();
        self.flags = AtomicU32::new(FLAG_PRODUCER_ALIVE);
    }

    fn producer_alive(&self) -> bool {
        self.flags.load(Ordering::Acquire) & FLAG_PRODUCER_ALIVE != 0
    }

    fn mark_producer_dead(&self) {
        self.flags
            .fetch_and(!FLAG_PRODUCER_ALIVE, Ordering::Release);
        self.flags.fetch_or(FLAG_PRODUCER_DEAD, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Platform abstraction: shared memory primitives
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[allow(clippy::undocumented_unsafe_blocks)]
mod shm_platform {
    use std::os::unix::io::RawFd;

    pub struct ShmHandle {
        pub ptr: *mut u8,
        pub size: usize,
        pub shm_fd: RawFd,
        pub shm_name: String,
    }

    // SAFETY: Owns a pointer from mmap — safe to send across threads.
    unsafe impl Send for ShmHandle {}

    pub fn create(name: &str, size: usize, permissions: u32) -> Result<ShmHandle, String> {
        use std::ffi::CString;
        let name_c = CString::new(name).map_err(|e| format!("shm name: {e}"))?;

        let fd = unsafe {
            libc::shm_open(
                name_c.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                permissions as libc::mode_t as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(format!(
                "shm_open('{name}'): {}",
                std::io::Error::last_os_error()
            ));
        }

        if unsafe { libc::ftruncate(fd, size as libc::off_t) } != 0 {
            let e = std::io::Error::last_os_error();
            unsafe {
                libc::close(fd);
                libc::shm_unlink(name_c.as_ptr());
            }
            return Err(format!("ftruncate('{name}'): {e}"));
        }

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            let e = std::io::Error::last_os_error();
            unsafe {
                libc::close(fd);
                libc::shm_unlink(name_c.as_ptr());
            }
            return Err(format!("mmap('{name}'): {e}"));
        }

        Ok(ShmHandle {
            ptr: ptr as *mut u8,
            size,
            shm_fd: fd,
            shm_name: name.to_string(),
        })
    }

    pub fn destroy(handle: ShmHandle) {
        unsafe {
            libc::munmap(handle.ptr as *mut libc::c_void, handle.size);
            libc::close(handle.shm_fd);
            let name_c = std::ffi::CString::new(handle.shm_name.as_str()).unwrap();
            libc::shm_unlink(name_c.as_ptr());
        }
    }
}

#[cfg(windows)]
#[allow(clippy::undocumented_unsafe_blocks)]
mod shm_platform {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    // Raw Windows FFI (no external crate dependency)
    extern "system" {
        fn CreateFileMappingW(
            hFile: isize,
            lpAttributes: *const u8,
            flProtect: u32,
            dwMaximumSizeHigh: u32,
            dwMaximumSizeLow: u32,
            lpName: *const u16,
        ) -> isize;

        fn MapViewOfFile(
            hFileMappingObject: isize,
            dwDesiredAccess: u32,
            dwFileOffsetHigh: u32,
            dwFileOffsetLow: u32,
            dwNumberOfBytesToMap: usize,
        ) -> *mut u8;

        fn UnmapViewOfFile(lpBaseAddress: *const u8) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }

    const INVALID_HANDLE_VALUE: isize = -1;
    const PAGE_READWRITE: u32 = 0x04;
    const FILE_MAP_ALL_ACCESS: u32 = 0x000F001F;

    pub struct ShmHandle {
        pub ptr: *mut u8,
        pub size: usize,
        pub mapping_handle: isize,
        pub shm_name: String,
    }

    // SAFETY: Owns a pointer from MapViewOfFile — safe to send across threads.
    unsafe impl Send for ShmHandle {}

    pub fn create(name: &str, size: usize, _permissions: u32) -> Result<ShmHandle, String> {
        let wide_name: Vec<u16> = OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let size_high = (size as u64 >> 32) as u32;
        let size_low = (size as u64 & 0xFFFF_FFFF) as u32;

        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                size_high,
                size_low,
                wide_name.as_ptr(),
            )
        };

        if handle == 0 || handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "CreateFileMappingW('{name}'): {}",
                std::io::Error::last_os_error()
            ));
        }

        let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) };

        if ptr.is_null() {
            let e = std::io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(format!("MapViewOfFile('{name}'): {e}"));
        }

        Ok(ShmHandle {
            ptr,
            size,
            mapping_handle: handle,
            shm_name: name.to_string(),
        })
    }

    pub fn destroy(handle: ShmHandle) {
        unsafe {
            UnmapViewOfFile(handle.ptr);
            CloseHandle(handle.mapping_handle);
        }
    }
}

use shm_platform::ShmHandle;

// ---------------------------------------------------------------------------
// Ring buffer operations (platform-independent)
// ---------------------------------------------------------------------------

/// Get a mutable reference to the header from the shared memory pointer.
///
/// # Safety
/// Caller must guarantee `ptr` points to a valid, writable mmap'd memory
/// region of at least `SHM_HEADER_SIZE` bytes. The returned reference has
/// static lifetime because the shared memory outlives the process.
unsafe fn header_mut(ptr: *mut u8) -> &'static mut ShmHeader {
    // SAFETY: precondition documented on the function — ptr must point
    // to a valid mapped ShmHeader.
    unsafe { &mut *(ptr as *mut ShmHeader) }
}

/// Get a shared reference to the header from the shared memory pointer.
///
/// # Safety
/// Caller must guarantee `ptr` points to a valid, readable mmap'd memory
/// region of at least `SHM_HEADER_SIZE` bytes.
unsafe fn header_ref(ptr: *mut u8) -> &'static ShmHeader {
    // SAFETY: precondition documented on the function — ptr must point
    // to a valid mapped ShmHeader.
    unsafe { &*(ptr as *const ShmHeader) }
}

/// Get a pointer to the slot at the given index.
fn slot_ptr(ptr: *mut u8, slot_size: u32, index: usize) -> *mut u8 {
    let offset = SHM_HEADER_SIZE + index * slot_size as usize;
    // SAFETY: offset is within the mapped region bounds.
    unsafe { ptr.add(offset) }
}

/// Write data to a slot. Returns bytes written (including 4B length prefix).
unsafe fn write_slot(ptr: *mut u8, slot_size: u32, index: usize, data: &[u8]) -> usize {
    let len = data.len().min(slot_size as usize - 4);
    let slot = slot_ptr(ptr, slot_size, index);

    // SAFETY: slot_ptr returns a valid pointer within the mapped region.
    unsafe {
        let len_bytes = (len as u32).to_le_bytes();
        std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), slot, 4);
        std::ptr::copy_nonoverlapping(data.as_ptr(), slot.add(4), len);
    }

    len + 4
}

// ---------------------------------------------------------------------------
// Full policy
// ---------------------------------------------------------------------------

/// Ring buffer full policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmFullPolicy {
    /// Drop the record currently being written
    DropNewest,
    /// Overwrite the oldest unread record
    DropOldest,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for sink_shm.
#[derive(Debug, Clone)]
pub struct ShmSinkConfig {
    /// Shared memory object path
    pub path: String,
    /// input_format MUST be "sif" — other values rejected with error
    pub input_format: String,
    /// Total buffer size in megabytes (power of two, min 8)
    pub buffer_size_mb: usize,
    /// Each slot's max capacity in kilobytes (min 64)
    pub slot_size_kb: usize,
    /// Ring buffer full policy
    pub full_policy: ShmFullPolicy,
    /// durability_level is forced to Unsafe for sink_shm (non-persistent)
    pub durability_level: DurabilityLevel,
    /// Unix permissions (ignored on Windows)
    pub permissions: u32,
    /// Auto-cleanup on close
    pub auto_cleanup: bool,
    /// Allowed consumer paths (empty = allow all)
    pub allowed_consumers: Vec<String>,
}

/// Durability level — matches sink.rs but minimal copy for sink_shm independence.
/// sink_shm is forced to Unsafe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityLevel {
    /// No durability — data lost on crash
    Unsafe = 0,
    /// OS cache flush only
    OsCache = 1,
    /// Media flush per write
    Media = 2,
    /// Media flush with FUA
    MediaWithFua = 3,
}

impl Default for ShmSinkConfig {
    fn default() -> Self {
        Self {
            path: "/dologger_default.shm".into(),
            input_format: "sif".into(),
            buffer_size_mb: 64,
            slot_size_kb: 64,
            full_policy: ShmFullPolicy::DropNewest,
            durability_level: DurabilityLevel::Unsafe, // forced to UNSAFE
            permissions: 0o660,
            auto_cleanup: true,
            allowed_consumers: Vec::new(),
        }
    }
}

impl ShmSinkConfig {
    /// Validate configuration against requirements.
    pub fn validate(&self) -> Result<(), String> {
        // input_format MUST be "sif"
        if self.input_format != "sif" {
            return Err(format!(
                "DO_LOG_ERR_SINK_FORMAT_INVALID: sink_shm input_format must be \"sif\", got \"{}\"",
                self.input_format
            ));
        }
        if self.buffer_size_mb < MIN_BUFFER_SIZE_MB {
            return Err(format!(
                "buffer_size_mb must be >= {MIN_BUFFER_SIZE_MB}, got {}",
                self.buffer_size_mb
            ));
        }
        if !self.buffer_size_mb.is_power_of_two() {
            return Err(format!(
                "buffer_size_mb must be power of two, got {}",
                self.buffer_size_mb
            ));
        }
        if self.slot_size_kb < MIN_SLOT_SIZE_KB {
            return Err(format!(
                "slot_size_kb must be >= {MIN_SLOT_SIZE_KB}, got {}",
                self.slot_size_kb
            ));
        }
        // durability_level is forced to UNSAFE
        if self.durability_level != DurabilityLevel::Unsafe {
            crate::sys::diag::warn(
                "shm_sink",
                "durability_level overridden to UNSAFE for sink_shm",
            );
        }
        Ok(())
    }

    /// Check that no fallback is configured — sink_shm does not support fallback.
    pub fn check_no_fallback(fallback_configured: bool) -> Result<(), String> {
        if fallback_configured {
            return Err(
                "DO_LOG_ERR_SINK_NO_FALLBACK: sink_shm does not support fallback chains".into(),
            );
        }
        Ok(())
    }

    /// Check that this sink is not being used for an AUDIT domain.
    pub fn check_audit_forbidden(is_audit_domain: bool) -> Result<(), String> {
        if is_audit_domain {
            return Err(
                "DO_LOG_ERR_AUDIT_SHM_FORBIDDEN: sink_shm cannot be used with AUDIT domain".into(),
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

/// Runtime statistics for the shared memory sink.
#[derive(Debug, Clone, Default)]
pub struct ShmSinkStats {
    /// Total records successfully written
    pub total_written: u64,
    /// Total records dropped (ring full with drop_newest)
    pub total_dropped: u64,
    /// Total records overwritten (drop_oldest)
    pub total_overwritten: u64,
    /// Total bytes written
    pub total_bytes: u64,
    /// Current producer sequence number
    pub producer_seq: u64,
    /// Current consumer sequence number
    pub consumer_seq: u64,
}

// ---------------------------------------------------------------------------
// ShmSink
// ---------------------------------------------------------------------------

/// The shared memory Sink — zero-copy SIF record delivery to external consumers.
///
/// The hot write path uses a raw pointer to the ShmHandle instead of a Mutex
/// to achieve P99 < 1μs latency. The pointer is set once during open()
/// and never changes. Safety: `handle_ptr` is only accessed when `open` is true.
pub struct ShmSink {
    config: ShmSinkConfig,
    /// Owned handle (for cleanup on close/drop)
    handle: Mutex<Option<ShmHandle>>,
    /// Raw pointer to the handle for lock-free hot-path access
    handle_ptr: std::sync::atomic::AtomicPtr<u8>,
    open: AtomicBool,
    total_written: AtomicU64,
    total_dropped: AtomicU64,
    total_overwritten: AtomicU64,
    total_bytes: AtomicU64,
    /// Timestamp (millis since epoch) of the last drop report.
    /// Uses compare_exchange for lock-free rate limiting.
    last_drop_report_ms: AtomicU64,
}

impl ShmSink {
    /// Create a new shared memory sink with the given configuration.
    pub fn new(config: ShmSinkConfig) -> Self {
        Self {
            config,
            handle: Mutex::new(None),
            handle_ptr: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
            open: AtomicBool::new(false),
            total_written: AtomicU64::new(0),
            total_dropped: AtomicU64::new(0),
            total_overwritten: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            last_drop_report_ms: AtomicU64::new(0),
        }
    }

    /// Open the shared memory region and initialize the ring buffer.
    pub fn open(&self, sysmon: &Sysmon) -> Result<(), String> {
        if self.open.load(Ordering::Acquire) {
            return Ok(());
        }

        let total_bytes = self.config.buffer_size_mb * 1024 * 1024;
        let slot_size = (self.config.slot_size_kb * 1024) as u32;
        let slot_count = (total_bytes - SHM_HEADER_SIZE) / slot_size as usize;

        if slot_count < 4 {
            return Err(format!(
                "sink_shm: need ≥4 slots, got {slot_count} (buf={total_bytes}B, slot={slot_size}B)"
            ));
        }

        let handle = shm_platform::create(&self.config.path, total_bytes, self.config.permissions)?;

        // SAFETY: handle.ptr points to the freshly mapped shared memory.
        unsafe {
            header_mut(handle.ptr).init(total_bytes as u64, slot_count as u32, slot_size);
        }

        sysmon.info(
            "shm",
            &format!(
                "SHM_INIT path={} size={}MB slots={}",
                self.config.path, self.config.buffer_size_mb, slot_count
            ),
        );

        // Store raw pointer for lock-free hot-path access
        self.handle_ptr.store(handle.ptr, Ordering::Release);
        self.open.store(true, Ordering::Release);
        *self.handle.lock().unwrap() = Some(handle);
        Ok(())
    }

    /// Write a serialized SIF record to the ring buffer.
    ///
    /// Non-blocking: applies `full_policy` if buffer is full.
    /// Returns `true` if written, `false` if dropped.
    #[allow(clippy::undocumented_unsafe_blocks)]
    pub fn write(&self, sif_data: &[u8]) -> bool {
        if !self.open.load(Ordering::Acquire) {
            return false;
        }

        // Lock-free hot path: read handle_ptr directly
        let ptr = self.handle_ptr.load(Ordering::Acquire);
        if ptr.is_null() {
            return false;
        }

        // SAFETY: ptr is set during open() and only cleared after open=false.
        // open guard above ensures it's valid.
        let header = unsafe { header_ref(ptr) };
        let slot_count = header.slot_count as u64;
        let slot_size = header.slot_size_bytes;

        // Check data fits in a slot (with 4B length prefix)
        if sif_data.len() + 4 > slot_size as usize {
            diag::warn(
                "shm_sink",
                &format!(
                    "SIF record {}B > slot {}B — dropped",
                    sif_data.len(),
                    slot_size
                ),
            );
            return false;
        }

        loop {
            let producer = header.producer_seq.load(Ordering::Acquire);
            let consumer = header.consumer_seq.load(Ordering::Acquire);

            if producer - consumer >= slot_count {
                // Buffer full
                return match self.config.full_policy {
                    ShmFullPolicy::DropNewest => {
                        self.total_dropped.fetch_add(1, Ordering::Relaxed);
                        header.dropped_count.fetch_add(1, Ordering::Relaxed);
                        self.report_drop();
                        false
                    }
                    ShmFullPolicy::DropOldest => {
                        let overwrite_seq = consumer;
                        let idx = (overwrite_seq % slot_count) as usize;
                        unsafe {
                            write_slot(ptr, slot_size, idx, sif_data);
                        }
                        header.producer_seq.fetch_add(1, Ordering::Release);
                        let _ = header.consumer_seq.compare_exchange(
                            overwrite_seq,
                            overwrite_seq + 1,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        header.overwritten_count.fetch_add(1, Ordering::Relaxed);
                        self.total_overwritten.fetch_add(1, Ordering::Relaxed);
                        self.total_written.fetch_add(1, Ordering::Relaxed);
                        self.total_bytes
                            .fetch_add(sif_data.len() as u64, Ordering::Relaxed);
                        true
                    }
                };
            }

            // Buffer has space — CAS claim a slot
            let idx = (producer % slot_count) as usize;
            match header.producer_seq.compare_exchange(
                producer,
                producer + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    unsafe {
                        write_slot(ptr, slot_size, idx, sif_data);
                    }
                    self.total_written.fetch_add(1, Ordering::Relaxed);
                    self.total_bytes
                        .fetch_add(sif_data.len() as u64, Ordering::Relaxed);
                    return true;
                }
                Err(_) => continue, // Another writer claimed it — retry
            }
        }
    }

    /// Write a Record as SIF to the shared memory buffer.
    pub fn write_record(&self, record: &Record) -> bool {
        let sif = record_to_sif(record);
        self.write(&sif)
    }

    /// Flush (no-op: shared memory writes are immediately visible).
    pub fn flush(&self) -> Result<(), String> {
        Ok(())
    }

    /// Close the sink, marking the producer as dead.
    /// If `auto_cleanup`, destroys the shared memory object.
    #[allow(clippy::undocumented_unsafe_blocks)]
    pub fn close(&self, sysmon: &Sysmon) {
        if !self.open.load(Ordering::Acquire) {
            return;
        }

        if let Some(ref handle) = *self.handle.lock().unwrap() {
            let header = unsafe { header_ref(handle.ptr) };
            header.mark_producer_dead();
        }

        self.open.store(false, Ordering::Release);

        let mut guard = self.handle.lock().unwrap();
        if let Some(handle) = guard.take() {
            if self.config.auto_cleanup {
                sysmon.info("shm", &format!("SHM_CLEANUP path={}", self.config.path));
                shm_platform::destroy(handle);
            } else {
                drop(handle);
            }
        }
    }

    /// Get current statistics.
    #[allow(clippy::undocumented_unsafe_blocks)]
    pub fn stats(&self) -> ShmSinkStats {
        let (producer, consumer) = match self.handle.lock().unwrap().as_ref() {
            Some(h) => {
                let hdr = unsafe { header_ref(h.ptr) };
                (
                    hdr.producer_seq.load(Ordering::Relaxed),
                    hdr.consumer_seq.load(Ordering::Relaxed),
                )
            }
            None => (0, 0),
        };

        ShmSinkStats {
            total_written: self.total_written.load(Ordering::Relaxed),
            total_dropped: self.total_dropped.load(Ordering::Relaxed),
            total_overwritten: self.total_overwritten.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            producer_seq: producer,
            consumer_seq: consumer,
        }
    }

    /// Check whether the sink is currently open and accepting writes.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Report a dropped record (rate-limited to ~once per second).
    ///
    /// Uses `AtomicU64` + `compare_exchange` instead of a Mutex to avoid
    /// lock contention in the hot write path.
    fn report_drop(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let last = self.last_drop_report_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 1000
            && self
                .last_drop_report_ms
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let dropped = self.total_dropped.load(Ordering::Relaxed);
            diag::warn("shm_sink", &format!("SHM_DROP total_dropped={dropped}"));
        }
    }
}

impl Drop for ShmSink {
    fn drop(&mut self) {
        if self.open.load(Ordering::Acquire) {
            if let Some(ref handle) = *self.handle.lock().unwrap() {
                // SAFETY: handle.ptr was created by shm_platform::create and
                // is guaranteed to point to a valid mapped ShmHeader.
                let header = unsafe { header_ref(handle.ptr) };
                header.mark_producer_dead();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Record → SIF binary format
// ---------------------------------------------------------------------------

/// Convert a Record to a simplified SIF binary blob.
///
/// Format: `SIF1` + total_len(u32 LE) + lsn(u64) + timestamp_hi(u64) +
/// timestamp_lo(u64) + level(u8) + flags(u8) + thread_id(u64) +
/// process_id(u32) + message(varlen) + source_file(varlen) +
/// host_name(varlen) + [signature(64B)] + [prev_hash(32B)]
pub fn record_to_sif(record: &Record) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);

    // Magic
    buf.extend_from_slice(b"SIF1");

    // Placeholder for total length (patched at end)
    let len_pos = buf.len();
    buf.extend_from_slice(&[0u8; 4]);

    // LSN
    buf.extend_from_slice(&record.lsn.to_le_bytes());

    // Timestamp
    buf.extend_from_slice(&record.timestamp.hi.to_le_bytes());
    buf.extend_from_slice(&record.timestamp.lo.to_le_bytes());

    // Level
    buf.push(record.level as u8);

    // Flags: bit0=has_signature, bit1=has_prev_hash
    let mut flags: u8 = 0;
    if record.signature.iter().any(|&b| b != 0) {
        flags |= 0x01;
    }
    if record.prev_hash.iter().any(|&b| b != 0) {
        flags |= 0x02;
    }
    buf.push(flags);

    // Thread + Process IDs
    buf.extend_from_slice(&record.thread_id.to_le_bytes());
    buf.extend_from_slice(&record.process_id.to_le_bytes());

    // Variable-length fields (2B LE length + UTF-8)
    for field in [
        record.message.as_str(),
        record.source_file.as_str(),
        record.host_name.as_str(),
    ] {
        let bytes = field.as_bytes();
        let len = bytes.len().min(u16::MAX as usize) as u16;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&bytes[..len as usize]);
    }

    // Optional: signature
    if flags & 0x01 != 0 {
        buf.extend_from_slice(&record.signature);
    }

    // Optional: prev_hash
    if flags & 0x02 != 0 {
        buf.extend_from_slice(&record.prev_hash);
    }

    // Patch total length
    let total = (buf.len() - len_pos - 4) as u32;
    buf[len_pos..len_pos + 4].copy_from_slice(&total.to_le_bytes());

    buf
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::LogLevel;
    use crate::sys::Sysmon;

    fn make_test_record() -> Record {
        let mut rec = Record::new(0);
        rec.lsn = 42;
        rec.level = LogLevel::Info;
        rec.thread_id = 12345;
        rec.process_id = 6789;
        rec.message.set("test message");
        rec.host_name.set("test-host");
        rec
    }

    #[test]
    fn test_config_validation() {
        assert!(ShmSinkConfig {
            buffer_size_mb: 10,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ShmSinkConfig {
            buffer_size_mb: 4,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ShmSinkConfig {
            slot_size_kb: 32,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ShmSinkConfig::default().validate().is_ok());
    }

    #[test]
    fn test_record_to_sif() {
        let rec = make_test_record();
        let sif = record_to_sif(&rec);
        assert!(sif.len() >= 32);
        assert_eq!(&sif[..4], b"SIF1");
    }

    #[test]
    fn test_header_size() {
        assert_eq!(std::mem::size_of::<ShmHeader>(), 64);
    }

    #[test]
    fn test_sink_lifecycle() {
        let config = ShmSinkConfig {
            path: format!("/dologger_test_lc_{}.shm", std::process::id()),
            buffer_size_mb: 8,
            slot_size_kb: 64,
            auto_cleanup: true,
            ..Default::default()
        };
        let sink = ShmSink::new(config);
        let sys = Sysmon::start();

        assert!(!sink.is_open());
        assert!(sink.open(&sys).is_ok());
        assert!(sink.is_open());

        // Write some records
        let rec = make_test_record();
        assert!(sink.write_record(&rec));
        assert!(sink.write(&record_to_sif(&rec)));

        // Flush is no-op but shouldn't error
        assert!(sink.flush().is_ok());

        sink.close(&sys);
        assert!(!sink.is_open());
    }

    #[test]
    fn test_write_to_closed_sink() {
        let sink = ShmSink::new(ShmSinkConfig::default());
        assert!(!sink.write(b"test"));
        assert!(!sink.write_record(&make_test_record()));
    }

    #[test]
    fn test_full_policy_drop_newest() {
        let config = ShmSinkConfig {
            path: format!("/dologger_test_full_{}.shm", std::process::id()),
            buffer_size_mb: 8,
            slot_size_kb: 64,
            full_policy: ShmFullPolicy::DropNewest,
            auto_cleanup: true,
            ..Default::default()
        };
        let sink = ShmSink::new(config);
        let sys = Sysmon::start();
        sink.open(&sys).unwrap();

        let data = vec![0xAAu8; 200];
        let mut written = 0;
        for _ in 0..5000 {
            if sink.write(&data) {
                written += 1;
            }
        }
        assert!(written > 0);
        assert_eq!(sink.stats().total_written, written);

        sink.close(&sys);
    }

    #[test]
    fn test_stats_initial() {
        let stats = ShmSink::new(ShmSinkConfig::default()).stats();
        assert_eq!(stats.total_written, 0);
        assert_eq!(stats.total_dropped, 0);
    }
}
