//! Example Filter plugin for DoLogger — M2 deliverable.
//!
//! This plugin demonstrates the Filter VTable interface:
//! it drops log records below a configurable severity level.
//!
//! # Usage
//!
//! Build: `cargo build --release -p dologger-filter-example`
//! Load: copy `target/release/example_filter.so` to `./plugins/`
//! Config: `[plugins.example_filter]` with `min_level = "WARN"`
//!
//! # C ABI symbols exported
//!
//! - `plugin_query(core_abi_version)` → returns the canonical
//!   `dologger_plugin_info_t` with a filter VTable
//! - `plugin_init(config)` → reads min_level from config
//! - `plugin_shutdown()` → cleanup
//!
//! This is a *standalone* third-party plugin: it exports the single-plugin
//! `plugin_query(u32)` contract. The official plugins ship in ONE dynamic
//! library via `plugin_query_multi` instead — see `plugins/official/bundle`.

use std::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicU8, Ordering};

use dologger_core::ffi::DologgerPluginInfo;

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0101;

// Log level constants (must match core's LogLevel enum)
const LEVEL_TRACE: u8 = 0;
const LEVEL_DEBUG: u8 = 1;
const LEVEL_INFO: u8 = 2;
const LEVEL_WARN: u8 = 3;
const LEVEL_ERROR: u8 = 4;
const LEVEL_FATAL: u8 = 5;
const LEVEL_AUDIT: u8 = 6;

// Plugin mount phase (must match DO_LOG_PHASE_FILTER in C header)
const PHASE_FILTER: u32 = 0x0002;

// Plugin info versioning — abi_version MUST match the core's declared ABI
// (0.1.0); the host validates it on load.
const CORE_ABI_VERSION: u32 = dologger_core::plugin::CORE_ABI_VERSION;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// ---------------------------------------------------------------------------
// Plugin state
// ---------------------------------------------------------------------------

/// Minimum log level to pass through (records below this are dropped).
static MIN_LEVEL: AtomicU8 = AtomicU8::new(LEVEL_WARN);

// ---------------------------------------------------------------------------
// C ABI: plugin_query
// ---------------------------------------------------------------------------

/// VTable for a Filter plugin.
///
/// SAFETY: Function pointers in the static instance point to static functions,
/// so sharing across threads is safe.
#[repr(C)]
struct FilterVTable {
    /// Filter function: receives record handle, returns 0=pass, 1=drop
    filter: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) -> i32,
    /// Batch filter function (optional, NULL if not implemented)
    filter_batch: Option<
        unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void, u32, *mut u8) -> i32,
    >,
}

// SAFETY: Function pointers in the static instance point to static functions.
unsafe impl Sync for FilterVTable {}

/// The actual filter function — drops records below MIN_LEVEL.
unsafe extern "C" fn filter_impl(
    _record: *mut std::ffi::c_void,
    level: *const std::ffi::c_void,
) -> i32 {
    if level.is_null() {
        return 1; // Drop if no level info
    }
    let record_level = unsafe { *(level as *const u8) };
    let min = MIN_LEVEL.load(Ordering::Relaxed);
    if record_level < min {
        1 // Drop
    } else {
        0 // Pass
    }
}

static VTABLE: FilterVTable = FilterVTable {
    filter: filter_impl,
    filter_batch: None,
};

static PLUGIN_NAME: &[u8] = b"example-filter\0";

#[no_mangle]
pub extern "C" fn plugin_query(_core_abi_version: u32) -> *const DologgerPluginInfo {
    static INFO: DologgerPluginInfo = DologgerPluginInfo {
        name: PLUGIN_NAME.as_ptr() as *const c_char,
        version: PLUGIN_VERSION,
        abi_version: CORE_ABI_VERSION,
        phase: PHASE_FILTER,
        vtable: &VTABLE as *const FilterVTable as *const std::ffi::c_void,
    };
    &INFO as *const DologgerPluginInfo
}

// ---------------------------------------------------------------------------
// C ABI: plugin_init
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_init(config: *const std::ffi::c_void) -> i32 {
    if config.is_null() {
        return DO_LOG_OK; // Use default min_level
    }

    // Config is a JSON string: {"min_level":"DEBUG"}
    let config_str = unsafe { CStr::from_ptr(config as *const c_char) };
    if let Ok(s) = config_str.to_str() {
        // Simple JSON value extraction for "min_level" key
        if let Some(val) = s
            .split("\"min_level\"")
            .nth(1)
            .and_then(|rest| rest.split('"').nth(1))
        {
            match val.to_uppercase().as_str() {
                "TRACE" => MIN_LEVEL.store(LEVEL_TRACE, Ordering::Relaxed),
                "DEBUG" => MIN_LEVEL.store(LEVEL_DEBUG, Ordering::Relaxed),
                "INFO" => MIN_LEVEL.store(LEVEL_INFO, Ordering::Relaxed),
                "WARN" => MIN_LEVEL.store(LEVEL_WARN, Ordering::Relaxed),
                "ERROR" => MIN_LEVEL.store(LEVEL_ERROR, Ordering::Relaxed),
                "FATAL" => MIN_LEVEL.store(LEVEL_FATAL, Ordering::Relaxed),
                "AUDIT" => MIN_LEVEL.store(LEVEL_AUDIT, Ordering::Relaxed),
                _ => return DO_LOG_ERR_INVALID_ARG,
            }
        }
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
    use std::ffi::CString;
    use std::sync::Mutex;

    // cargo test runs test functions on parallel threads. The tests below
    // mutate the process-wide MIN_LEVEL static, so they must hold this lock
    // to avoid racing each other's global state.
    static TEST_GUARD: Mutex<()> = Mutex::new(());

    fn lock_globals() -> std::sync::MutexGuard<'static, ()> {
        TEST_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn test_plugin_query_returns_valid_info() {
        let info = unsafe { &*plugin_query(dologger_core::plugin::CORE_ABI_VERSION) };
        assert_eq!(info.abi_version, CORE_ABI_VERSION);
        assert_eq!(info.phase, PHASE_FILTER);
        let name = unsafe { CStr::from_ptr(info.name) }.to_str().unwrap();
        assert_eq!(name, "example-filter");
    }

    #[test]
    fn test_filter_drops_trace_when_min_is_warn() {
        let _guard = lock_globals();
        MIN_LEVEL.store(LEVEL_WARN, Ordering::Relaxed);
        let trace: u8 = LEVEL_TRACE;
        let warn: u8 = LEVEL_WARN;
        assert_eq!(
            unsafe {
                filter_impl(
                    std::ptr::null_mut(),
                    &trace as *const u8 as *const std::ffi::c_void,
                )
            },
            1
        );
        assert_eq!(
            unsafe {
                filter_impl(
                    std::ptr::null_mut(),
                    &warn as *const u8 as *const std::ffi::c_void,
                )
            },
            0
        );
    }

    #[test]
    fn test_init_parses_level() {
        let _guard = lock_globals();
        let config = CString::new("{\"min_level\":\"DEBUG\"}").unwrap();
        assert_eq!(
            plugin_init(config.as_ptr() as *const std::ffi::c_void),
            DO_LOG_OK
        );
        assert_eq!(MIN_LEVEL.load(Ordering::Relaxed), LEVEL_DEBUG);
        // Reset
        MIN_LEVEL.store(LEVEL_WARN, Ordering::Relaxed);
    }
}
