//! Official DoLogger FieldProvider plugin — `field_container`.
//!
//! Injects container orchestration metadata into every log record:
//! container ID (from /proc/self/cgroup or $CONTAINER_ID), pod name,
//! namespace, node name (from Kubernetes downward API).
//! Phase: FieldProvider (2), Trust: Blue.
//!
//! # C ABI symbols exported
//!
//! - `plugin_query()` → returns PluginInfo with FieldProvider VTable
//! - `plugin_init(config)` → parses config (source: auto/docker/k8s/podman)
//! - `plugin_shutdown()` → cleanup

use std::ffi::CStr;
use std::os::raw::c_char;

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
#[allow(dead_code)]
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0102;
const DO_LOG_ERR_NOT_SUPPORTED: i32 = -0x0103;

// Plugin mount phase — FieldProvider stage (host info injection)
const PHASE_HOSTINFO: u32 = 0x0100;

// Plugin info versioning
const CORE_ABI_VERSION: u32 = 1;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// Trust level: Blue (official, signed)
const TRUST_BLUE: u32 = 0;

// Plugin type: FieldProvider
const PLUGIN_TYPE_FIELD_PROVIDER: u32 = 2;

// ---------------------------------------------------------------------------
// VTable: FieldProvider
// ---------------------------------------------------------------------------

/// VTable for a FieldProvider plugin.
///
/// FieldProvider plugins inject custom key-value fields into every log
/// record before processing. The engine calls `inject_fields` once per
/// record during the FieldProvider pipeline stage (stage 2).
///
/// SAFETY: Function pointers in the static instance point to static functions,
/// so sharing across threads is safe.
#[repr(C)]
struct FieldProviderVTable {
    /// Inject fields into a record.
    ///
    /// Parameters:
    /// - `record`: mutable pointer to the log record (opaque to plugin)
    /// - `plugin_state`: mutable pointer to plugin-private state
    ///
    /// Returns DO_LOG_OK on success, or an error code.
    inject_fields: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32,
}

// SAFETY: Function pointers point to static functions.
unsafe impl Sync for FieldProviderVTable {}

// ---------------------------------------------------------------------------
// PluginInfo (C ABI struct)
// ---------------------------------------------------------------------------

/// Plugin information returned by plugin_query.
#[repr(C)]
pub struct PluginInfo {
    /// Plugin type identifier (2 = FieldProvider)
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

/// Inject fields stub — returns DO_LOG_OK without injecting any fields.
/// Real implementation: detect container runtime, read /proc/self/cgroup
/// or environment variables, and inject container.* fields into the record.
unsafe extern "C" fn inject_fields_impl(
    _record: *mut std::ffi::c_void,
    _plugin_state: *mut std::ffi::c_void,
) -> i32 {
    DO_LOG_ERR_NOT_SUPPORTED // stub: not yet implemented
}

static VTABLE: FieldProviderVTable = FieldProviderVTable {
    inject_fields: inject_fields_impl,
};

static PLUGIN_NAME: &[u8] = b"field-container\0";

// ---------------------------------------------------------------------------
// C ABI: plugin_query
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_query() -> *const PluginInfo {
    static INFO: PluginInfo = PluginInfo {
        plugin_type: PLUGIN_TYPE_FIELD_PROVIDER,
        abi_version: CORE_ABI_VERSION,
        version: PLUGIN_VERSION,
        name: PLUGIN_NAME.as_ptr() as *const c_char,
        phase: PHASE_HOSTINFO,
        trust_level: TRUST_BLUE,
        vtable: &VTABLE as *const FieldProviderVTable as *const std::ffi::c_void,
    };
    &INFO as *const PluginInfo
}

// ---------------------------------------------------------------------------
// C ABI: plugin_init
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_init(config: *const std::ffi::c_void) -> i32 {
    if config.is_null() {
        return DO_LOG_OK; // Use defaults (source = "auto")
    }

    // Config is a JSON string: {"source":"auto"}
    let config_str = unsafe { CStr::from_ptr(config as *const c_char) };
    if let Ok(_s) = config_str.to_str() {
        // TODO: Parse config field (source: auto/docker/k8s/podman)
        // and store in static state. The real implementation will also
        // cache container metadata after first detection.
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
        assert_eq!(info.plugin_type, PLUGIN_TYPE_FIELD_PROVIDER);
        assert_eq!(info.abi_version, CORE_ABI_VERSION);
        assert_eq!(info.phase, PHASE_HOSTINFO);
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
