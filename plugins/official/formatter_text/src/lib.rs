//! Official DoLogger Formatter plugin — `formatter_text`.
//!
//! Human-readable colored text output with configurable field columns.
//! Phase: Formatting (5), Trust: Blue.
//!
//! # M6 — implemented via the host-accessor bridge
//!
//! The record handle is opaque to the plugin. The engine hands it a
//! [`HostAccessors`] table at `plugin_init`; `format_impl` reads fields
//! (`message`, `level`, `thread.id`, `record.timestamp`) through
//! `accessors.field_get` and renders a plain-text line into the engine-owned
//! [`OutputBuffer`], honouring `capacity` and returning
//! `DO_LOG_ERR_BUFFER_TOO_SMALL` when the line does not fit (the engine grows
//! the buffer and retries).
//!
//! # Bundle member
//!
//! This crate provides the plugin LOGIC (VTable + metadata) as an rlib. It is
//! aggregated by the `dologger-official-plugins` bundle crate, which exposes
//! the C ABI (`plugin_query_multi` / `plugin_init` / `plugin_shutdown`) for
//! all official plugins in ONE dynamic library.

use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

use dologger_core::ffi::DologgerPluginInfo;
use dologger_core::plugin::vtable::{FormatterVTable, HostAccessors, HostInit, OutputBuffer};

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0102;
const DO_LOG_ERR_BUFFER_TOO_SMALL: i32 = -0x0107;

// Plugin mount phase — Formatting stage
const PHASE_FORMATTING: u32 = dologger_core::plugin::phase::PHASE_FORMATTING;

// Plugin info versioning — abi_version MUST match the core's declared ABI
// (0.1.0); the host validates it when the bundle is loaded.
const CORE_ABI_VERSION: u32 = dologger_core::plugin::CORE_ABI_VERSION;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// ---------------------------------------------------------------------------
// Static state — the host-accessor bridge captured at init
// ---------------------------------------------------------------------------

/// Host accessor bridge captured at `plugin_init`; used by `format_impl`.
static HOST: Mutex<Option<HostAccessors>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Host-accessor helpers
// ---------------------------------------------------------------------------

/// Read a field via the host accessor, growing the buffer on overflow.
///
/// `field_get` fills a caller buffer and returns `>= 0` (byte count) on
/// success or a negative error code. We start with a modest buffer and double
/// it on `DO_LOG_ERR_BUFFER_TOO_SMALL`, capped to avoid unbounded growth.
unsafe fn read_field(
    accessors: &HostAccessors,
    record: *const std::ffi::c_void,
    name: &str,
) -> Option<String> {
    let name_c = CString::new(name).ok()?;
    let mut size: usize = 64;
    loop {
        // SAFETY: `alloc` returns host-owned, writable memory of `size` bytes
        // (or NULL). We free it on every exit path below.
        let buf = (accessors.alloc)(size);
        if buf.is_null() {
            return None;
        }
        let rc = (accessors.field_get)(record, name_c.as_ptr(), buf as *mut c_char, size);
        if rc >= 0 {
            // SAFETY: rc is the byte count; `buf` holds a NUL-terminated value
            // of that length. We copy it out before freeing.
            let val = CStr::from_ptr(buf as *const c_char)
                .to_string_lossy()
                .into_owned();
            (accessors.free)(buf);
            return Some(val);
        }
        (accessors.free)(buf);
        if rc == DO_LOG_ERR_BUFFER_TOO_SMALL {
            size *= 2;
            if size > (1 << 20) {
                return None; // safety cap: 1 MiB per field is far more than enough
            }
        } else {
            return None; // field not found / permission denied / other error
        }
    }
}

// ---------------------------------------------------------------------------
// VTable function: format
// ---------------------------------------------------------------------------

/// Format one record as a plain-text line into the output buffer.
///
/// Reads `message`, `level`, `thread.id`, and `record.timestamp` through the
/// host accessor and renders `[ts] [level] [thread] message`. Returns
/// `DO_LOG_OK` on success or `DO_LOG_ERR_BUFFER_TOO_SMALL` if the line does
/// not fit the engine-provided buffer (engine grows and retries).
unsafe extern "C" fn format_impl(
    record: *const std::ffi::c_void,
    output: *mut OutputBuffer,
    _config: *mut std::ffi::c_void,
) -> i32 {
    if record.is_null() || output.is_null() {
        return DO_LOG_ERR_INVALID_ARG;
    }
    let accessors = match *HOST.lock().unwrap() {
        Some(a) => a,
        None => return DO_LOG_ERR_INVALID_ARG, // init() not called — no bridge
    };

    let message = read_field(&accessors, record, "message").unwrap_or_default();
    let level = read_field(&accessors, record, "level").unwrap_or_default();
    let thread = read_field(&accessors, record, "thread.id").unwrap_or_default();
    let ts = read_field(&accessors, record, "record.timestamp").unwrap_or_default();

    let line = format!("[{ts}] [{level}] [{thread}] {message}");
    let bytes = line.as_bytes();
    let ob = &mut *output;
    if bytes.len() > ob.capacity {
        return DO_LOG_ERR_BUFFER_TOO_SMALL;
    }
    // SAFETY: output is engine-owned; `data[0..capacity]` is writable and we
    // copy at most `capacity` bytes. The engine reads back `data[0..len]`.
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ob.data, bytes.len());
    ob.len = bytes.len();
    DO_LOG_OK
}

/// The plugin's VTable — `format` (matches `dologger_formatter_vtable_t`).
static VTABLE: FormatterVTable = FormatterVTable {
    format: format_impl,
};

static PLUGIN_NAME: &[u8] = b"formatter-text\0";

// ---------------------------------------------------------------------------
// Plugin registry entry — aggregated by the official bundle.
// ---------------------------------------------------------------------------

/// Canonical plugin info for this crate, as it appears in the bundle registry.
pub static INFO: DologgerPluginInfo = DologgerPluginInfo {
    name: PLUGIN_NAME.as_ptr() as *const c_char,
    version: PLUGIN_VERSION,
    abi_version: CORE_ABI_VERSION,
    phase: PHASE_FORMATTING,
    vtable: &VTABLE as *const FormatterVTable as *const std::ffi::c_void,
};

/// Accessor for the registry entry (used by tests and the bundle crate).
pub fn plugin_info() -> &'static DologgerPluginInfo {
    &INFO
}

// ---------------------------------------------------------------------------
// Lifecycle — called by the bundle's `plugin_init` fan-out
// ---------------------------------------------------------------------------

/// Initialise the plugin: capture the host-accessor bridge.
///
/// `config` points to a [`HostInit`] (`dologger_host_init_t`). For v0.1.0 the
/// bridge is captured; `config_json` (color/show_thread/timestamp_format) is
/// reserved for a future formatting-config pass.
pub fn init(config: *const std::ffi::c_void) -> i32 {
    // SAFETY: config is NULL (defaults) or a HostInit pointer from the host.
    let accessors = if config.is_null() {
        HostAccessors::default()
    } else {
        // SAFETY: non-null config is a HostInit* from the engine (M6 bridge).
        unsafe { &*(config as *const HostInit) }.accessors
    };
    *HOST.lock().unwrap() = Some(accessors);
    DO_LOG_OK
}

// ---------------------------------------------------------------------------
// Lifecycle — called by the bundle's `plugin_shutdown` fan-out
// ---------------------------------------------------------------------------

pub fn shutdown() -> i32 {
    DO_LOG_OK
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_info_returns_valid_entry() {
        let info = plugin_info();
        assert_eq!(info.abi_version, CORE_ABI_VERSION);
        assert_eq!(info.phase, PHASE_FORMATTING);
        let name = unsafe { CStr::from_ptr(info.name) }.to_str().unwrap();
        assert_eq!(name, "formatter-text");
    }

    #[test]
    fn test_init_null_config_captures_bridge() {
        assert_eq!(init(std::ptr::null()), DO_LOG_OK);
        let a = HOST.lock().unwrap().expect("bridge captured");
        assert_ne!(a.field_get as usize, 0);
        assert_ne!(a.field_set as usize, 0);
        assert_ne!(a.alloc as usize, 0);
        assert_ne!(a.free as usize, 0);
    }

    #[test]
    fn test_init_accepts_hostinit_config() {
        let hi = HostInit::default();
        assert_eq!(
            init(&hi as *const HostInit as *const std::ffi::c_void),
            DO_LOG_OK
        );
    }

    #[test]
    fn test_shutdown_returns_ok() {
        assert_eq!(shutdown(), DO_LOG_OK);
    }

    #[test]
    fn test_vtable_format_field_is_non_null() {
        assert_ne!(VTABLE.format as usize, 0);
    }

    #[test]
    fn test_format_without_bridge_returns_error() {
        *HOST.lock().unwrap() = None;
        let mut buf = OutputBuffer {
            data: std::ptr::null_mut(),
            len: 0,
            capacity: 0,
        };
        assert_eq!(
            unsafe { format_impl(std::ptr::null(), &mut buf, std::ptr::null_mut()) },
            DO_LOG_ERR_INVALID_ARG
        );
    }
}
