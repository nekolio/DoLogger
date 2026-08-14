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
use crate::error::{DologgerError, DO_LOG_ERR_INVALID_ARG, DO_LOG_ERR_NOT_SUPPORTED, DO_LOG_OK};
use crate::record::thread_id_u64;
use crate::record::LogLevel;
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
    /// Declared core ABI version (e.g. `0x000100` = 0.1.0)
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
        record.id = engine.time_source.next_id();
        record.timestamp = engine.time_source.now_utc();

        // Ring 1: Level + message
        record.level = LogLevel::from_u8(p.level).unwrap_or(LogLevel::Info);
        if !p.message.is_null() {
            if let Ok(msg) = CStr::from_ptr(p.message).to_str() {
                record.message.set(msg);
            }
        }
        if !p.source_file.is_null() {
            if let Ok(s) = CStr::from_ptr(p.source_file).to_str() {
                record.source_file.set(s);
            }
        }
        record.source_line = p.source_line;

        // Thread/process info
        record.thread_id = thread_id_u64();
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

    // SAFETY: field_name validated non-null above. CStr::from_ptr reads
    // a null-terminated UTF-8 string provided by the host.
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

    // SAFETY: record is a valid DologgerRecord pointer from the host.
    // We're accessing the engine's record pool. In practice this function
    // is a placeholder — the real implementation routes through Engine.
    // For now, we return DO_LOG_ERR_NOT_SUPPORTED as field access requires
    // the full Engine context.
    let _ = (record, name_str, val_str);
    DO_LOG_ERR_NOT_SUPPORTED
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

    if buffer_size > 0 {
        let msg = b"(field_get: not implemented)\0";
        let len = msg.len().min(buffer_size);
        // SAFETY: buffer is valid for buffer_size bytes (non-null check above).
        // msg is a static byte string within bounds; we copy at most buffer_size bytes.
        // The cast is required on targets where c_char is i8 (x86_64) and is a
        // no-op where c_char is u8 (aarch64-linux) — clippy flags the latter.
        #[allow(clippy::unnecessary_cast)]
        unsafe {
            std::ptr::copy_nonoverlapping(msg.as_ptr(), buffer as *mut u8, len);
        }
    }

    DO_LOG_ERR_NOT_SUPPORTED
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
