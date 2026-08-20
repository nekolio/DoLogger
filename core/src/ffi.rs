//! C ABI wrappers for the DoLogger core engine.
//!
//! All public-facing functions use `extern "C"` and C-compatible types.

// C ABI wrappers: raw pointer derefs are inherent to FFI, documented in the C header.
// TODO: Remove #![allow(missing_docs)] and add doc comments to all public FFI items
// (structs, extern functions). Requires comprehensive C header documentation first.
#![allow(missing_docs)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{c_char, c_void, CStr};

use crate::config::DologgerConfig;
use crate::error::{
    DologgerError, DO_LOG_ERR_BUFFER_TOO_SMALL, DO_LOG_ERR_INVALID_ARG, DO_LOG_ERR_SIF_INVALID,
    DO_LOG_OK,
};
use crate::record::thread_id_u64;
use crate::record::{FieldRing, LogLevel, Record};
use crate::{create_handle, destroy_handle, Engine};

/// Opaque handle to a DoLogger core instance.
#[repr(C)]
pub struct DologgerHandle {
    pub(crate) engine: Engine,
}

/// Opaque handle to a Record slot.
#[repr(C)]
pub struct DologgerRecord {
    _private: (),
}

/// 128-bit unsigned integer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct dologger_uint128_t {
    pub hi: u64,
    pub lo: u64,
}

/// Canonical plugin info struct — matches `dologger_plugin_info_t` in
/// `core/include/dologger_core.h`. Returned by `plugin_query` (single-plugin
/// libraries) and by each entry of `plugin_query_multi` (bundle libraries).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DologgerPluginInfo {
    /// Unique plugin identifier (UTF-8, null-terminated)
    pub name: *const c_char,
    /// Encoded binary-compat version
    pub version: u32,
    /// Declared core ABI version (e.g. `0x000001` = 0.0.1)
    pub abi_version: u32,
    /// Mount phase bitmask (DO_LOG_PHASE_*)
    pub phase: u32,
    /// Pointer to the VTable for this phase
    pub vtable: *const c_void,
}

/// Multi-plugin info list — matches `dologger_plugin_info_list_t`. Returned by
/// `plugin_query_multi` so a single dynamic library can host several plugins
/// (the official plugins bundle) instead of one plugin per file.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DologgerPluginInfoList {
    /// Number of entries in `infos`
    pub count: u32,
    /// Array of `count` pointers to `DologgerPluginInfo`
    pub infos: *const *const DologgerPluginInfo,
}

// SAFETY: both structs only carry raw pointers to *static* data owned by the
// plugin library. They are constructed once, immutably, and remain valid for
// the library's lifetime — sharing across threads is safe.
unsafe impl Sync for DologgerPluginInfo {}
// SAFETY: the list holds `count` pointers to the same static info entries as
// DologgerPluginInfo — valid for the library's lifetime, no internal mutability.
unsafe impl Sync for DologgerPluginInfoList {}

// Thread-local error storage
std::thread_local! {
    static LAST_ERROR: std::cell::RefCell<DologgerError> =
        const { std::cell::RefCell::new(DologgerError::new()) };
}

fn set_last_error(code: i32, msg: &str) {
    LAST_ERROR.with(|e| {
        let mut err = e.borrow_mut();
        err.code = code;
        let bytes = msg.as_bytes();
        let len = bytes.len().min(err.message.len() - 1);
        err.message[..len].copy_from_slice(&bytes[..len]);
        err.message[len] = 0;
    });
}

// ==========================================================================
// Core lifecycle
// ==========================================================================

#[no_mangle]
pub extern "C" fn dologger_init(
    config_path: *const std::os::raw::c_char,
    err: *mut DologgerError,
) -> *mut DologgerHandle {
    if err.is_null() {
        return std::ptr::null_mut();
    }

    let (config, warnings) = if config_path.is_null() {
        DologgerConfig::load_default()
    } else {
        // SAFETY: config_path is either NULL (handled above) or a valid C string from the host
        let c_path = unsafe { CStr::from_ptr(config_path) };
        match c_path.to_str() {
            Ok(path) => DologgerConfig::load_from_file(path).unwrap_or_else(|(code, msg)| {
                set_last_error(code, &msg);
                (DologgerConfig::hardcoded_defaults(), vec![msg])
            }),
            Err(_) => {
                set_last_error(DO_LOG_ERR_INVALID_ARG, "Config path not valid UTF-8");
                // SAFETY: err is non-null (validated at function entry)
                unsafe {
                    (*err).code = DO_LOG_ERR_INVALID_ARG;
                }
                return std::ptr::null_mut();
            }
        }
    };

    for w in &warnings {
        crate::sys::diagnostics::warn("ffi", w);
    }

    match Engine::init(config) {
        Ok(engine) => {
            // SAFETY: err is non-null (validated at function entry)
            unsafe {
                (*err).code = DO_LOG_OK;
            }
            create_handle(engine)
        }
        Err(msg) => {
            set_last_error(DO_LOG_ERR_INVALID_ARG, &msg);
            // SAFETY: err is non-null (validated at function entry)
            unsafe {
                (*err).code = DO_LOG_ERR_INVALID_ARG;
            }
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn dologger_shutdown(handle: *mut DologgerHandle) {
    if handle.is_null() {
        return;
    }
    // SAFETY: handle is non-null (validated above), and ownership transfers back from host
    unsafe {
        (*handle).engine.shutdown();
        destroy_handle(handle);
    }
}

// ==========================================================================
// Log submission
// ==========================================================================

/// Parameters for dologger_log (mirrors C header's dologger_record_params_t).
#[repr(C)]
pub struct dologger_log_params {
    pub level: u8,
    pub message: *const std::os::raw::c_char,
    pub source_file: *const std::os::raw::c_char,
    pub source_line: u32,
    _reserved: [u8; 16],
}

#[no_mangle]
pub extern "C" fn dologger_log(
    handle: *mut DologgerHandle,
    params: *const dologger_log_params,
) -> i32 {
    if handle.is_null() || params.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }

    // SAFETY: handle is non-null (validated above)
    let engine = unsafe { &(*handle).engine };

    // Allocate from pool
    let record_ptr = match engine.pool.alloc() {
        Some(r) => r,
        None => {
            set_last_error(DO_LOG_ERR_INVALID_ARG, "Record pool exhausted");
            return DO_LOG_ERR_INVALID_ARG;
        }
    };

    // SAFETY: record_ptr was just allocated from the pool (exclusive ownership)
    //         params is non-null (validated above)
    unsafe {
        let record = &mut *record_ptr;
        let p = &*params;

        // Ring 0: ID + timestamp
        let id = engine.time_source.next_id();
        record.set_id(id.hi, id.lo);
        record.timestamp = engine.time_source.now_nanos();

        // Ring 1: Level + message
        record.level = LogLevel::from_u8(p.level).unwrap_or(LogLevel::Info);
        if !p.message.is_null() {
            if let Ok(msg) = CStr::from_ptr(p.message).to_str() {
                record.message.set(msg);
            }
        }
        if !p.source_file.is_null() {
            if let Ok(s) = CStr::from_ptr(p.source_file).to_str() {
                record.set_source_file(s);
            }
        }
        record.set_source_line(p.source_line);

        // Thread/process info (fixed fields)
        record.thread_id = thread_id_u64() as u32;
        record.process_id = std::process::id();
    }

    // Push to ring buffer — with cooperative helping
    match engine.ring_buffer.try_push(record_ptr) {
        Ok(()) => DO_LOG_OK,
        Err(ptr) => {
            // Cooperative helping — when enabled and the ring buffer
            // is ≥90% full, try to help drain a small batch inline before
            // giving up. This prevents the calling application thread from
            // blocking while the consumer catches up.
            if let Some(ref helping) = engine.coop_helping {
                let helped = helping.try_help();
                if helped > 0 {
                    // We helped drain some records — retry the push once.
                    match engine.ring_buffer.try_push(ptr) {
                        Ok(()) => return DO_LOG_OK,
                        Err(ptr2) => {
                            // Still full after helping — relinquish to pool.
                            // SAFETY: ptr2 was allocated from the pool and has
                            // exclusive ownership at this point.
                            unsafe {
                                engine.pool.free(&*ptr2);
                            }
                            set_last_error(
                                crate::error::DO_LOG_ERR_BUFFER_FULL,
                                "Ring buffer full (after cooperative helping)",
                            );
                            return crate::error::DO_LOG_ERR_BUFFER_FULL;
                        }
                    }
                }
            }
            // No helping or helping was not triggered — drop the record.
            // SAFETY: ptr is valid, and we're relinquishing ownership back to pool.
            unsafe {
                engine.pool.free(&*ptr);
            }
            set_last_error(crate::error::DO_LOG_ERR_BUFFER_FULL, "Ring buffer full");
            crate::error::DO_LOG_ERR_BUFFER_FULL
        }
    }
}

// ==========================================================================
// Error query
// ==========================================================================

#[no_mangle]
pub extern "C" fn dologger_get_last_error(
    _handle: *const DologgerHandle,
    err: *mut DologgerError,
) -> i32 {
    if err.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }
    // SAFETY: err is non-null (validated above); thread-local storage access is safe
    LAST_ERROR.with(|e| unsafe {
        *err = e.borrow().clone();
    });
    // SAFETY: err is non-null
    unsafe { (*err).code }
}

// ==========================================================================
// Field access
// ==========================================================================

#[no_mangle]
pub extern "C" fn dologger_field_set(
    record: *mut DologgerRecord,
    field_name: *const std::os::raw::c_char,
    value: *const std::os::raw::c_char,
    err: *mut DologgerError,
) -> i32 {
    if record.is_null() || field_name.is_null() || value.is_null() || err.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }

    // SAFETY: field_name validated non-null above.
    let name = unsafe { CStr::from_ptr(field_name) };
    // SAFETY: value validated non-null above.
    let val = unsafe { CStr::from_ptr(value) };

    let name_str = match name.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error(DO_LOG_ERR_INVALID_ARG, "field_name not valid UTF-8");
            return DO_LOG_ERR_INVALID_ARG;
        }
    };
    let val_str = match val.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error(DO_LOG_ERR_INVALID_ARG, "value not valid UTF-8");
            return DO_LOG_ERR_INVALID_ARG;
        }
    };

    // SAFETY: `record` is an opaque handle the engine hands out as a pointer to
    // a live `Record`; the host passes it back verbatim, so this cast recovers
    // the original `Record` reference — the same pattern `dologger_log` uses on
    // records it allocates from the engine pool. The C ABI contract
    // (`dologger_core.h`) requires callers to pass only handles obtained from
    // the engine; passing a fabricated pointer is a caller-side violation.
    //
    // The raw FFI surface is treated as an untrusted (Ring 3) caller, matching
    // the documented contract: Ring 0 fields stay read-only, Ring 1 fields
    // require HostInfoProvider, Ring 2/3 fields are writable by plugins.
    let rec = unsafe { &mut *(record as *mut Record) };
    match rec.field_set(name_str, val_str, FieldRing::Ring3) {
        Ok(()) => {
            set_last_error(DO_LOG_OK, "ok");
            DO_LOG_OK
        }
        Err(e) => {
            // Preserve the fine-grained failure cause (not found vs permission
            // denied vs type mismatch) instead of collapsing to one code.
            set_last_error(e.abi_code(), e.as_str());
            e.abi_code()
        }
    }
}

#[no_mangle]
pub extern "C" fn dologger_field_get(
    record: *const DologgerRecord,
    field_name: *const std::os::raw::c_char,
    buffer: *mut std::os::raw::c_char,
    buffer_size: usize,
    err: *mut DologgerError,
) -> i32 {
    if record.is_null() || field_name.is_null() || buffer.is_null() || err.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }

    if buffer_size == 0 {
        set_last_error(DO_LOG_ERR_BUFFER_TOO_SMALL, "buffer size 0");
        return DO_LOG_ERR_BUFFER_TOO_SMALL;
    }

    // SAFETY: field_name validated non-null above.
    let name = unsafe { CStr::from_ptr(field_name) };
    let name_str = match name.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error(DO_LOG_ERR_INVALID_ARG, "field_name not valid UTF-8");
            return DO_LOG_ERR_INVALID_ARG;
        }
    };

    // SAFETY: same opaque-handle contract as `dologger_field_set`.
    let rec = unsafe { &*(record as *const Record) };
    let value = match rec.field_get(name_str, FieldRing::Ring3) {
        Ok(v) => v,
        Err(e) => {
            // Reads can only fail with FIELD_NOT_FOUND today, but keep the
            // typed mapping so new read failures stay distinguishable.
            set_last_error(e.abi_code(), e.as_str());
            return e.abi_code();
        }
    };

    // Copy the value into the caller's buffer, NUL-terminated. If the value
    // does not fit in `buffer_size - 1` bytes, the copy is truncated and
    // `DO_LOG_ERR_BUFFER_TOO_SMALL` is returned so the caller can grow.
    let bytes = value.as_bytes();
    if bytes.len() >= buffer_size {
        // SAFETY: buffer valid for buffer_size bytes; we copy buffer_size-1
        // and NUL-terminate at buffer_size-1, both within bounds.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer as *mut u8, buffer_size - 1);
            *buffer.add(buffer_size - 1) = 0;
        }
        set_last_error(DO_LOG_ERR_BUFFER_TOO_SMALL, "field value truncated");
        return DO_LOG_ERR_BUFFER_TOO_SMALL;
    }

    let n = bytes.len();
    // SAFETY: buffer valid for buffer_size bytes; n <= buffer_size-1 and the
    // NUL write at buffer.add(n) is within bounds.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer as *mut u8, n);
        *buffer.add(n) = 0;
    }
    set_last_error(DO_LOG_OK, "ok");
    n as i32
}

// ==========================================================================
// Memory
// ==========================================================================

#[no_mangle]
pub extern "C" fn dologger_alloc(size: usize) -> *mut std::os::raw::c_void {
    let layout = std::alloc::Layout::from_size_align(size.max(1), 8).unwrap();
    // SAFETY: layout is valid (size > 0, alignment = 8)
    unsafe { std::alloc::alloc(layout) as *mut _ }
}

#[no_mangle]
pub extern "C" fn dologger_free(ptr: *mut std::os::raw::c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr was previously allocated by dologger_alloc with the same layout.
    // We use a default layout (align=8, size unknown at free time). In production,
    // a tracked allocator would store the layout alongside each allocation.
    // For now, we deallocate with a conservative estimate.
    unsafe {
        let layout = std::alloc::Layout::from_size_align(1, 8).unwrap();
        // Note: This is not strictly correct — the original allocation layout
        // should be tracked. A proper tracked allocator is planned.
        std::alloc::dealloc(ptr as *mut u8, layout);
    }
}

// ==========================================================================
// Version
// ==========================================================================

#[no_mangle]
pub extern "C" fn dologger_version() -> *const std::os::raw::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const _
}

// ==========================================================================
// Config from string
// ==========================================================================

#[no_mangle]
pub extern "C" fn dologger_config_load_from_string(
    handle: *mut DologgerHandle,
    toml_data: *const std::os::raw::c_char,
    err: *mut DologgerError,
) -> i32 {
    if handle.is_null() || toml_data.is_null() || err.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }

    // SAFETY: All pointers validated non-null. CStr::from_ptr reads a
    // null-terminated UTF-8 TOML string from the host.
    let toml_str = match unsafe { CStr::from_ptr(toml_data) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error(DO_LOG_ERR_INVALID_ARG, "TOML data not valid UTF-8");
            return DO_LOG_ERR_INVALID_ARG;
        }
    };

    // Parse and merge the TOML config into the engine's current config
    // SAFETY: handle is a valid DologgerHandle from dologger_init.
    let engine = unsafe { &mut (*handle).engine };

    match DologgerConfig::parse(toml_str, engine.config.config_path.clone()) {
        Ok((new_config, warnings)) => {
            for w in &warnings {
                crate::sys::diagnostics::warn("ffi", w);
            }
            engine.config = new_config;
            DO_LOG_OK
        }
        Err((code, msg)) => {
            set_last_error(code, &msg);
            code
        }
    }
}

// ==========================================================================
// Record lifecycle (for SIF encode/decode round-trips)
// ==========================================================================

/// Create an un-pooled, zero-initialised [`Record`] for C hosts to populate via
/// [`dologger_field_set`] and then encode with [`dologger_sif_encode_record`].
///
/// Returns an opaque `dologger_record_handle_t*`, or NULL on allocation failure.
/// Release with [`dologger_record_destroy`]. The record is owned by the host;
/// it is *not* pooled and is *not* submitted to the ring buffer.
#[no_mangle]
pub extern "C" fn dologger_record_create() -> *mut DologgerRecord {
    let rec = Box::new(Record::new(0));
    Box::into_raw(rec) as *mut DologgerRecord
}

/// Destroy a record previously created by [`dologger_record_create`].
///
/// # Safety
///
/// `record` must have been returned by [`dologger_record_create`] and not yet
/// destroyed. NULL is a no-op.
#[no_mangle]
pub extern "C" fn dologger_record_destroy(record: *mut DologgerRecord) {
    if record.is_null() {
        return;
    }
    // SAFETY: caller guarantees `record` came from dologger_record_create and
    // is not yet destroyed. Box::from_raw retakes ownership and drops it.
    unsafe {
        drop(Box::from_raw(record as *mut Record));
    }
}

// ==========================================================================
// SIF encode / decode / validate
// ==========================================================================

/// Validate a SIF frame's magic, version, and length without decoding it.
///
/// Returns `DO_LOG_OK` (0) if the frame is structurally valid, or
/// `DO_LOG_ERR_SIF_INVALID` otherwise (message via `dologger_get_last_error`).
#[no_mangle]
pub extern "C" fn dologger_sif_validate_frame(
    frame: *const u8,
    frame_len: usize,
    err: *mut DologgerError,
) -> i32 {
    if frame.is_null() || err.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }
    // SAFETY: caller supplies a readable `frame` of exactly `frame_len` bytes.
    let buf = unsafe { std::slice::from_raw_parts(frame, frame_len) };
    match crate::sif::validate_frame(buf) {
        Ok(_) => {
            set_last_error(DO_LOG_OK, "ok");
            DO_LOG_OK
        }
        Err(e) => {
            set_last_error(DO_LOG_ERR_SIF_INVALID, &e.to_string());
            DO_LOG_ERR_SIF_INVALID
        }
    }
}

/// Encode a record into a complete SIF frame (magic + header + FlatBuffer).
///
/// On success allocates a host-owned buffer via [`dologger_alloc`], stores its
/// pointer in `*out` and its byte length in `*out_len`, and returns `DO_LOG_OK`.
/// The caller must release the buffer with [`dologger_free`]. On failure returns
/// a negative `DO_LOG_ERR_*` code and leaves `*out`/`*out_len` untouched.
#[no_mangle]
pub extern "C" fn dologger_sif_encode_record(
    record: *const DologgerRecord,
    out: *mut *mut u8,
    out_len: *mut usize,
    err: *mut DologgerError,
) -> i32 {
    if record.is_null() || out.is_null() || out_len.is_null() || err.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }
    // SAFETY: `record` is a valid opaque handle to a live Record; cast back
    // through the raw pointer the way the engine's record pool hands them out.
    let rec = unsafe { &*(record as *const Record) };
    let bytes = crate::sif::encode_record(rec);
    let len = bytes.len();
    // SAFETY: allocates `len` bytes of host-owned memory (never freed here).
    let ptr = dologger_alloc(len) as *mut u8;
    if ptr.is_null() {
        set_last_error(crate::error::DO_LOG_ERR_OUT_OF_MEMORY, "alloc failed");
        return crate::error::DO_LOG_ERR_OUT_OF_MEMORY;
    }
    // SAFETY: `ptr` points to `len` freshly-allocated bytes; copy the frame in.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
        *out = ptr;
        *out_len = len;
    }
    set_last_error(DO_LOG_OK, "ok");
    DO_LOG_OK
}

/// Decode a SIF frame into a new record.
///
/// On success stores an opaque handle (create via [`dologger_record_destroy`])
/// in `*out_record` and returns `DO_LOG_OK`. On failure returns a negative
/// `DO_LOG_ERR_*` code and leaves `*out_record` untouched.
#[no_mangle]
pub extern "C" fn dologger_sif_decode_record(
    frame: *const u8,
    frame_len: usize,
    out_record: *mut *mut DologgerRecord,
    err: *mut DologgerError,
) -> i32 {
    if frame.is_null() || out_record.is_null() || err.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }
    // SAFETY: caller supplies a readable `frame` of exactly `frame_len` bytes.
    let buf = unsafe { std::slice::from_raw_parts(frame, frame_len) };
    match crate::sif::decode_record(buf) {
        Ok(rec) => {
            let ptr = Box::into_raw(Box::new(rec)) as *mut DologgerRecord;
            // SAFETY: `out_record` is non-null; store the fresh handle.
            unsafe { *out_record = ptr };
            set_last_error(DO_LOG_OK, "ok");
            DO_LOG_OK
        }
        Err(e) => {
            set_last_error(DO_LOG_ERR_SIF_INVALID, &e.to_string());
            DO_LOG_ERR_SIF_INVALID
        }
    }
}

// ==========================================================================
// Tests
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{DO_LOG_ERR_FIELD_NOT_FOUND, DO_LOG_ERR_FIELD_PERMISSION_DENIED};
    use std::ffi::CString;

    /// Reinterpret a `&mut Record` as the opaque `DologgerRecord` pointer, the
    /// way the engine's record pool hands records to a C caller.
    fn as_opaque(rec: &mut Record) -> *mut DologgerRecord {
        rec as *mut Record as *mut DologgerRecord
    }
    fn as_opaque_const(rec: &Record) -> *const DologgerRecord {
        rec as *const Record as *const DologgerRecord
    }
    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }
    fn err_out() -> DologgerError {
        DologgerError::new()
    }

    #[test]
    fn field_set_get_ring3_round_trip() {
        let mut rec = Record::new(0);
        let mut err = err_out();

        // Ring 3 (ext.*) field write succeeds.
        let ret = dologger_field_set(
            as_opaque(&mut rec),
            c("ext.trace_id").as_ptr(),
            c("abc123").as_ptr(),
            &mut err,
        );
        assert_eq!(ret, DO_LOG_OK);

        // And reads back with the value NUL-terminated.
        let mut buf = [0u8; 64];
        let n = dologger_field_get(
            as_opaque_const(&rec),
            c("ext.trace_id").as_ptr(),
            buf.as_mut_ptr() as *mut std::os::raw::c_char,
            buf.len(),
            &mut err,
        );
        assert_eq!(n, 6, "bytes written should equal value length");
        assert_eq!(&buf[..6], b"abc123");
        assert_eq!(buf[6], 0, "buffer must be NUL-terminated");
    }

    #[test]
    fn field_set_ring0_is_read_only_for_ffi() {
        let mut rec = Record::new(0);
        let mut err = err_out();

        // Ring 0 fields are read-only; an untrusted FFI caller must be denied.
        let ret = dologger_field_set(
            as_opaque(&mut rec),
            c("record.id").as_ptr(),
            c("fabricated").as_ptr(),
            &mut err,
        );
        assert_eq!(ret, DO_LOG_ERR_FIELD_PERMISSION_DENIED);
    }

    #[test]
    fn field_get_unknown_returns_not_found() {
        let rec = Record::new(0);
        let mut err = err_out();
        let mut buf = [0u8; 64];

        let ret = dologger_field_get(
            as_opaque_const(&rec),
            c("nope.not_a_field").as_ptr(),
            buf.as_mut_ptr() as *mut std::os::raw::c_char,
            buf.len(),
            &mut err,
        );
        assert_eq!(ret, DO_LOG_ERR_FIELD_NOT_FOUND);
    }

    #[test]
    fn field_get_truncation_reports_buffer_too_small() {
        let mut rec = Record::new(0);
        rec.message.set("a fairly long message value");
        let mut err = err_out();

        // Buffer smaller than the value → truncated copy + BUFFER_TOO_SMALL.
        let mut small = [0u8; 8];
        let ret = dologger_field_get(
            as_opaque_const(&rec),
            c("message").as_ptr(),
            small.as_mut_ptr() as *mut std::os::raw::c_char,
            small.len(),
            &mut err,
        );
        assert_eq!(ret, DO_LOG_ERR_BUFFER_TOO_SMALL);
        // The truncated copy is NUL-terminated within the small buffer.
        assert_eq!(&small[..7], b"a fairl");
        assert_eq!(small[7], 0);
    }

    #[test]
    fn field_get_zero_buffer_is_rejected() {
        let rec = Record::new(0);
        let mut err = err_out();
        let mut buf = [0u8; 1];

        let ret = dologger_field_get(
            as_opaque_const(&rec),
            c("message").as_ptr(),
            buf.as_mut_ptr() as *mut std::os::raw::c_char,
            0,
            &mut err,
        );
        assert_eq!(ret, DO_LOG_ERR_BUFFER_TOO_SMALL);
    }

    #[test]
    fn sif_encode_validate_decode_round_trip() {
        // Build a record via the C ABI create/populate/destroy surface.
        let rec = dologger_record_create();
        assert!(!rec.is_null());
        let mut err = err_out();

        // The C ABI field API treats callers as Ring 3, so Ring 1 fields
        // (message, trace.id, …) are correctly denied by the ring guard and
        // Ring 2/3 vendor slots (`ext.*`, `verified.*`) are not yet carried by
        // the SIF wire format. Populate the message directly through the
        // `Record` struct the opaque handle wraps — it is the payload SIF
        // round-trips end-to-end.
        // SAFETY: `rec` is the live `Record` wrapped by the C ABI handle
        // created above; mutating it through the raw pointer is the intended
        // FFI test surface and the handle is destroyed after the assertions.
        let rec_inner = unsafe { &mut *(rec as *mut Record) };
        rec_inner.message.set("hello sif");

        // Encode → validate → decode.
        let mut out: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let ret = dologger_sif_encode_record(rec, &mut out, &mut out_len, &mut err);
        assert_eq!(ret, DO_LOG_OK);
        assert!(!out.is_null());
        assert!(out_len > 0);

        // Validate the frame.
        assert_eq!(
            dologger_sif_validate_frame(out, out_len, &mut err),
            DO_LOG_OK
        );
        // A corrupt magic must fail validation.
        let mut corrupt = vec![0u8; out_len];
        // SAFETY: `out` points to exactly `out_len` bytes allocated above and
        // not yet freed; copying them out for corruption is safe.
        corrupt.copy_from_slice(unsafe { std::slice::from_raw_parts(out, out_len) });
        corrupt[0] = b'X';
        assert_eq!(
            dologger_sif_validate_frame(corrupt.as_ptr(), corrupt.len(), &mut err),
            DO_LOG_ERR_SIF_INVALID
        );

        // Decode and read a field back.
        let mut decoded: *mut DologgerRecord = std::ptr::null_mut();
        let ret = dologger_sif_decode_record(out, out_len, &mut decoded, &mut err);
        assert_eq!(ret, DO_LOG_OK);
        assert!(!decoded.is_null());
        let mut buf = [0u8; 64];
        let n = dologger_field_get(
            decoded,
            c("message").as_ptr(),
            buf.as_mut_ptr() as *mut std::os::raw::c_char,
            buf.len(),
            &mut err,
        );
        assert_eq!(n, 9, "bytes written should equal 'hello sif' length");
        assert_eq!(&buf[..9], b"hello sif");

        // Cleanup.
        dologger_record_destroy(rec);
        dologger_record_destroy(decoded);
        dologger_free(out as *mut std::os::raw::c_void);
    }
}
