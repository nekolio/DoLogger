//! Official DoLogger FieldProvider plugin — `field_container`.
//!
//! Injects container orchestration metadata into every log record:
//! container ID (from /proc/self/cgroup or $CONTAINER_ID), pod name,
//! namespace, node name (from Kubernetes downward API).
//! Phase: FieldProvider (host info injection).
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

// Plugin mount phase — FieldProvider stage (host info injection)
const PHASE_HOSTINFO: u32 = 0x0100;

// Plugin info versioning — abi_version MUST match the core's declared ABI
// (0.1.0); the host validates it when the bundle is loaded.
const CORE_ABI_VERSION: u32 = dologger_core::plugin::CORE_ABI_VERSION;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

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
// Plugin info — canonical `dologger_plugin_info_t` (see core/src/ffi.rs).
// Registered by the official bundle via `plugin_query_multi`.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// VTable function stubs
// ---------------------------------------------------------------------------

/// Inject container.* fields into a record.
///
/// PLACEHOLDER — returns DO_LOG_ERR_NOT_SUPPORTED because the field-injection
/// pipeline is not yet wired at v0.1.0:
///   1. the engine's FieldProvider stage does not dispatch this vtable, and
///   2. no field-access accessor is handed to plugins, so the opaque `record`
///      handle cannot be written (container.id / pod / namespace / node)
///      from inside the bundle.
///
/// Both land with M6 (C ABI record access + pipeline dispatch). When they do,
/// this function detects the container runtime (`$CONTAINER_ID`,
/// `/proc/self/cgroup`, Kubernetes downward API), then writes fields via the
/// dispatched `dologger_field_set` accessor at Ring 3.
///
/// This is a *documented placeholder*, not dead code — it must not be deleted.
unsafe extern "C" fn inject_fields_impl(
    _record: *mut std::ffi::c_void,
    _plugin_state: *mut std::ffi::c_void,
) -> i32 {
    DO_LOG_ERR_NOT_SUPPORTED
}

static VTABLE: FieldProviderVTable = FieldProviderVTable {
    inject_fields: inject_fields_impl,
};

static PLUGIN_NAME: &[u8] = b"field-container\0";

// ---------------------------------------------------------------------------
// Plugin registry entry — aggregated by the official bundle.
// ---------------------------------------------------------------------------

/// Canonical plugin info for this crate, as it appears in the bundle registry.
pub static INFO: DologgerPluginInfo = DologgerPluginInfo {
    name: PLUGIN_NAME.as_ptr() as *const c_char,
    version: PLUGIN_VERSION,
    abi_version: CORE_ABI_VERSION,
    phase: PHASE_HOSTINFO,
    vtable: &VTABLE as *const FieldProviderVTable as *const std::ffi::c_void,
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
        assert_eq!(info.phase, PHASE_HOSTINFO);
        let name = unsafe { CStr::from_ptr(info.name) }.to_str().unwrap();
        assert_eq!(name, "field-container");
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
