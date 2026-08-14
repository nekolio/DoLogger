//! Official DoLogger Formatter plugin — `formatter_text`.
//!
//! Human-readable colored text output with configurable field columns.
//! Phase: Formatting (5), Trust: Blue.
//!
//! # Bundle member
//!
//! This crate provides the plugin LOGIC (VTable + metadata) as an rlib. It is
//! aggregated by the `dologger-official-plugins` bundle crate, which exposes
//! the C ABI (`plugin_query_multi` / `plugin_init` / `plugin_shutdown`) for
//! all official plugins in ONE dynamic library.

use std::ffi::{c_char, CStr};

use dologger_core::ffi::DologgerPluginInfo;

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
#[allow(dead_code)]
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0102;
const DO_LOG_ERR_NOT_SUPPORTED: i32 = -0x0103;

// Plugin mount phase — Formatting stage
const PHASE_FORMATTING: u32 = 0x0010;

// Plugin info versioning — abi_version MUST match the core's declared ABI
// (0.1.0); the host validates it when the bundle is loaded.
const CORE_ABI_VERSION: u32 = dologger_core::plugin::CORE_ABI_VERSION;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// ---------------------------------------------------------------------------
// VTable: Formatter
// ---------------------------------------------------------------------------

/// VTable for a Formatter plugin.
///
/// Layout MUST exactly match the C ABI `dologger_formatter_vtable_t`
/// (core/include/dologger_core.h) — a single `format` function pointer. The
/// engine reinterprets the `vtable` pointer in [`INFO`] as that C type and
/// calls it with THREE arguments, so this signature must not gain or lose
/// parameters relative to the header.
///
/// (Note: `formatter_json` and the C++ example currently expose *different*
/// vtable shapes — a `format` that leaks `dologger_core::Record`. That is a
/// known contract inconsistency to be unified at M6 when the engine's
/// Formatting stage is wired; the authoritative shape is the one below.)
///
/// SAFETY: Function pointers in the static instance point to static functions,
/// so sharing across threads is safe.
#[repr(C)]
struct FormatterVTable {
    /// Format one record into the caller-provided output buffer.
    ///
    /// Arguments (per `dologger_formatter_vtable_t`):
    /// - `record` — opaque `dologger_record_handle_t*`; read via a field
    ///   accessor the engine dispatches (none is dispatched at v0.1.0).
    /// - `output` — `dologger_output_buffer_t*` `{data, len, capacity}`.
    /// - `config` — plugin config pointer (`void*`), or NULL at v0.1.0.
    ///
    /// Returns DO_LOG_OK on success, or DO_LOG_ERR_BUFFER_TOO_SMALL if the
    /// buffer is insufficient (engine reallocates and retries).
    format: unsafe extern "C" fn(
        *const std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
    ) -> i32,
}

// SAFETY: Function pointers point to static functions.
unsafe impl Sync for FormatterVTable {}

// ---------------------------------------------------------------------------
// Plugin info — canonical `dologger_plugin_info_t` (see core/src/ffi.rs).
// Registered by the official bundle via `plugin_query_multi`.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// VTable function stubs
// ---------------------------------------------------------------------------

/// Format one record as plain, configurable, colored text.
///
/// PLACEHOLDER — returns DO_LOG_ERR_NOT_SUPPORTED because the formatting
/// pipeline is not yet wired at v0.1.0:
///   1. the engine's Formatting stage does not dispatch Formatter vtables, and
///   2. no field-access accessor is handed to plugins, so the opaque `record`
///      handle cannot be read (level/message/timestamp) from inside the bundle.
///
/// Both land with M6 (C ABI record access + pipeline dispatch). When they do,
/// this function reads fields via the dispatched accessor and writes rendered
/// text into the `dologger_output_buffer_t` pointed to by `output`, honoring
/// `capacity` and returning DO_LOG_ERR_BUFFER_TOO_SMALL when it overflows.
///
/// This is a *documented placeholder*, not dead code — it must not be deleted.
unsafe extern "C" fn format_impl(
    _record: *const std::ffi::c_void,
    _output: *mut std::ffi::c_void,
    _config: *mut std::ffi::c_void,
) -> i32 {
    DO_LOG_ERR_NOT_SUPPORTED
}

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

pub fn init(config: *const std::ffi::c_void) -> i32 {
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
    fn test_init_with_null_config() {
        assert_eq!(init(std::ptr::null()), DO_LOG_OK);
    }

    #[test]
    fn test_shutdown_returns_ok() {
        assert_eq!(shutdown(), DO_LOG_OK);
    }
}
