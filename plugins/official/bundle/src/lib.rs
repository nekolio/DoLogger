//! Official plugins bundle — ONE dynamic library for every official plugin.
//!
//! Aggregates each official plugin crate (rlib logic-only, no C exports of
//! their own) into a single `cdylib` that exposes the multi-plugin registry
//! contract:
//!
//! - `plugin_query_multi(core_abi_version)` → `DologgerPluginInfoList`
//!   carrying all 4 official plugin entries (formatter-json, formatter-text,
//!   filter-level, field-container). The host registers every entry from this
//!   one library handle.
//! - `plugin_init(config)` / `plugin_shutdown()` fan out to each member.
//!
//! This replaces the old one-plugin-per-file layout: one shared object
//! (`libdologger_official_plugins.so` / `.dylib` / `.dll`) ships the full
//! official plugin set. See `core/src/plugin/manager.rs` for the host side.

use dologger_core::ffi::{DologgerPluginInfo, DologgerPluginInfoList};

const DO_LOG_OK: i32 = 0;

/// Registry of every official plugin, in mount order.
///
/// Stored as `&'static` references (not raw pointers) so the array is `Sync`;
/// `DologgerPluginInfo` is immutable once constructed, and the two-pointer
/// cast below yields the C-ABI list without copying.
static INFOS: [&DologgerPluginInfo; 4] = [
    &formatter_json::INFO,
    &formatter_text::INFO,
    &filter_level::INFO,
    &field_container::INFO,
];

/// Static multi-plugin info list returned by `plugin_query_multi`.
static LIST: DologgerPluginInfoList = DologgerPluginInfoList {
    count: INFOS.len() as u32,
    infos: INFOS.as_ptr() as *const *const DologgerPluginInfo,
};

/// Register every official plugin hosted by this library.
///
/// The host calls this once per library, then validates each entry's ABI
/// version and inserts it into the plugin registry. The returned list is a
/// static owned by this library and is valid for the library's lifetime.
///
/// `core_abi_version` is informational — the authoritative ABI check happens
/// per-entry in the host against each plugin's declared `abi_version`.
#[no_mangle]
pub extern "C" fn plugin_query_multi(_core_abi_version: u32) -> *const DologgerPluginInfoList {
    &LIST
}

/// Initialise every official plugin.
///
/// Fan-out: each member's `init` is called with the same `config` pointer.
/// Member inits are idempotent (NULL config keeps defaults), and the host may
/// call this once per registered plugin name — every call is safe to repeat.
#[no_mangle]
pub extern "C" fn plugin_init(config: *const std::ffi::c_void) -> i32 {
    let results = [
        formatter_json::init(config),
        formatter_text::init(config),
        filter_level::init(config),
        field_container::init(config),
    ];
    results
        .into_iter()
        .find(|&r| r != DO_LOG_OK)
        .unwrap_or(DO_LOG_OK)
}

/// Shut down every official plugin.
#[no_mangle]
pub extern "C" fn plugin_shutdown() -> i32 {
    let results = [
        formatter_json::shutdown(),
        formatter_text::shutdown(),
        filter_level::shutdown(),
        field_container::shutdown(),
    ];
    results
        .into_iter()
        .find(|&r| r != DO_LOG_OK)
        .unwrap_or(DO_LOG_OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn test_query_multi_registers_all_plugins() {
        let list = unsafe { &*plugin_query_multi(dologger_core::plugin::CORE_ABI_VERSION) };
        assert_eq!(list.count, 4);
        let infos = unsafe { std::slice::from_raw_parts(list.infos, list.count as usize) };
        let names: Vec<String> = infos
            .iter()
            .map(|p| {
                let info = unsafe { &**p };
                unsafe { CStr::from_ptr(info.name) }
                    .to_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            [
                "formatter-json",
                "formatter-text",
                "filter-level",
                "field-container"
            ]
        );
    }

    #[test]
    fn test_list_is_static_and_repeatable() {
        // Same pointer on every call — the host holds a stable reference for
        // as long as the library is loaded.
        let a = plugin_query_multi(dologger_core::plugin::CORE_ABI_VERSION);
        let b = plugin_query_multi(dologger_core::plugin::CORE_ABI_VERSION);
        assert_eq!(a, b);
        assert!(!a.is_null());
    }

    #[test]
    fn test_every_entry_declares_matching_abi() {
        let list = unsafe { &*plugin_query_multi(dologger_core::plugin::CORE_ABI_VERSION) };
        let infos = unsafe { std::slice::from_raw_parts(list.infos, list.count as usize) };
        for p in infos {
            let info = unsafe { &**p };
            assert_eq!(
                info.abi_version,
                dologger_core::plugin::CORE_ABI_VERSION,
                "every official plugin must declare the core ABI version"
            );
            assert!(!info.vtable.is_null());
            assert!(info.phase != 0);
        }
    }

    #[test]
    fn test_fanout_init_shutdown_ok() {
        assert_eq!(plugin_init(std::ptr::null()), DO_LOG_OK);
        assert_eq!(plugin_shutdown(), DO_LOG_OK);
        // Idempotent — the host may call init once per registered name.
        assert_eq!(plugin_init(std::ptr::null()), DO_LOG_OK);
    }
}
