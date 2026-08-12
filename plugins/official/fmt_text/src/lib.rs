//! Official DoLogger Formatter plugin — `fmt_text`.
//!
//! Human-readable colored text output with configurable field columns.
//! Phase: Formatting (5), Trust: Blue.
//!
//! # C ABI symbols exported
//!
//! - `plugin_query()` → returns PluginInfo with Formatter VTable
//! - `plugin_init(config)` → parses config (color, show_thread, show_timestamp, timestamp_format)
//! - `plugin_shutdown()` → cleanup

use std::ffi::CStr;
use std::os::raw::c_char;

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
#[allow(dead_code)]
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0102;
const DO_LOG_ERR_NOT_SUPPORTED: i32 = -0x0103;

// Plugin mount phase — Formatting stage
const PHASE_FORMATTING: u32 = 0x0010;

// Plugin info versioning
const CORE_ABI_VERSION: u32 = 1;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// Trust level: Blue (official, signed)
const TRUST_BLUE: u32 = 0;

// Plugin type: Formatter
const PLUGIN_TYPE_FORMATTER: u32 = 5;

// ---------------------------------------------------------------------------
// VTable: Formatter
// ---------------------------------------------------------------------------

/// VTable for a Formatter plugin.
///
/// Contains function pointers the engine calls to serialize log records.
/// SAFETY: Function pointers in the static instance point to static functions,
/// so sharing across threads is safe.
#[repr(C)]
struct FormatterVTable {
    /// Format a single record into the caller-provided output buffer.
    /// Returns DO_LOG_OK on success, or DO_LOG_ERR_BUF_TOO_SMALL if
    /// the buffer is insufficient (engine will reallocate and retry).
    format: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> i32,
    /// Flush any buffered output (optional, NULL if not implemented).
    flush: Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> i32>,
}

// SAFETY: Function pointers point to static functions.
unsafe impl Sync for FormatterVTable {}

// ---------------------------------------------------------------------------
// PluginInfo (C ABI struct)
// ---------------------------------------------------------------------------

/// Plugin information returned by plugin_query.
#[repr(C)]
pub struct PluginInfo {
    /// Plugin type identifier (5 = Formatter)
    plugin_type: u32,
    /// Core ABI version this plugin was compiled against
    abi_version: u32,
    /// Plugin version (packed: major<<16 | minor<<8 | patch)
    version: u32,
    /// Human-readable plugin name
    name: *const c_char,
    /// Mount phase(s) bitmask
    phase: u32,
    /// Trust level (0=Blue, 1=Yellow, 2=Red)
    trust_level: u32,
    /// Pointer to the VTable (cast to appropriate type)
    vtable: *const std::ffi::c_void,
}

// SAFETY: All raw pointers point to static data.
unsafe impl Sync for PluginInfo {}

// ---------------------------------------------------------------------------
// VTable function stubs
// ---------------------------------------------------------------------------

/// Format stub — returns DO_LOG_OK without writing output.
/// Real implementation: render colored text with configurable columns.
unsafe extern "C" fn format_impl(
    _record: *const std::ffi::c_void,
    _output: *mut std::ffi::c_void,
) -> i32 {
    DO_LOG_ERR_NOT_SUPPORTED // stub: not yet implemented
}

static VTABLE: FormatterVTable = FormatterVTable {
    format: format_impl,
    flush: None,
};

static PLUGIN_NAME: &[u8] = b"fmt-text\0";

// ---------------------------------------------------------------------------
// C ABI: plugin_query
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_query() -> *const PluginInfo {
    static INFO: PluginInfo = PluginInfo {
        plugin_type: PLUGIN_TYPE_FORMATTER,
        abi_version: CORE_ABI_VERSION,
        version: PLUGIN_VERSION,
        name: PLUGIN_NAME.as_ptr() as *const c_char,
        phase: PHASE_FORMATTING,
        trust_level: TRUST_BLUE,
        vtable: &VTABLE as *const FormatterVTable as *const std::ffi::c_void,
    };
    &INFO as *const PluginInfo
}

// ---------------------------------------------------------------------------
// C ABI: plugin_init
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_init(config: *const std::ffi::c_void) -> i32 {
    if config.is_null() {
        return DO_LOG_OK; // Use defaults
    }

    // Config is a JSON string: {"color":true,"show_thread":true,...}
    let config_str = unsafe { CStr::from_ptr(config as *const c_char) };
    if let Ok(_s) = config_str.to_str() {
        // TODO: Parse config fields (color, show_thread, show_timestamp,
        // timestamp_format) and store in static state.
    }
    DO_LOG_OK
}

// ---------------------------------------------------------------------------
// C ABI: plugin_shutdown
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_shutdown() -> i32 {
    DO_LOG_OK
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_query_returns_valid_info() {
        let info = unsafe { &*plugin_query() };
        assert_eq!(info.plugin_type, PLUGIN_TYPE_FORMATTER);
        assert_eq!(info.abi_version, CORE_ABI_VERSION);
        assert_eq!(info.phase, PHASE_FORMATTING);
        assert_eq!(info.trust_level, TRUST_BLUE);
    }

    #[test]
    fn test_init_with_null_config() {
        assert_eq!(plugin_init(std::ptr::null()), DO_LOG_OK);
    }

    #[test]
    fn test_shutdown_returns_ok() {
        assert_eq!(plugin_shutdown(), DO_LOG_OK);
    }
}
