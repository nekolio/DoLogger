//! Dynamic-loading integration test for the official plugins bundle.
//!
//! Loads the *built* `dologger_official_plugins` shared library with
//! `libloading` — exactly as the host (`PluginManager`) does — and verifies
//! the C-ABI contract end to end:
//!
//! - `plugin_query_multi` returns a stable list of all 4 official plugins.
//! - Every entry declares the matching core ABI version + a non-null vtable.
//! - `plugin_init` / `plugin_shutdown` fan out and return OK.
//! - The library hosts NO per-plugin `plugin_query` export (bundle-only ABI).
//!
//! This is the real proof of "ONE dynamic library, many plugins": the unit
//! tests call the Rust functions directly, this test goes through dlopen.

use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;

use libloading::Library;

/// Core ABI version — must match `dologger_core::plugin::CORE_ABI_VERSION`
/// (0.0.1 packed as `0x000001`). Kept local so the test asserts the contract
/// against the compiled artifact, not against a value re-imported from source.
const CORE_ABI_VERSION: u32 = 0x000001;

/// Canonical plugin info — mirrors `dologger_plugin_info_t`.
#[repr(C)]
struct DologgerPluginInfo {
    name: *const c_char,
    version: u32,
    abi_version: u32,
    phase: u32,
    vtable: *const c_void,
}

/// Multi-plugin list — mirrors `dologger_plugin_info_list_t`.
#[repr(C)]
struct DologgerPluginInfoList {
    count: u32,
    infos: *const *const DologgerPluginInfo,
}

type QueryMultiFn = unsafe extern "C" fn(u32) -> *const DologgerPluginInfoList;
type InitFn = unsafe extern "C" fn(*const c_void) -> i32;
type ShutdownFn = unsafe extern "C" fn() -> i32;

/// Locate the freshly built bundle cdylib in the target directory.
fn bundle_library_path() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let profile = option_env!("PROFILE").unwrap_or("debug");
    let stem = if cfg!(windows) {
        "dologger_official_plugins".to_string()
    } else {
        "libdologger_official_plugins".to_string()
    };
    let ext = if cfg!(windows) {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };

    let mut candidates: Vec<PathBuf> = vec![
        // Default: workspace target dir, both profiles.
        PathBuf::from(manifest).join(format!("../../../target/{profile}/{stem}.{ext}")),
        PathBuf::from(manifest).join(format!("../../../target/debug/{stem}.{ext}")),
        PathBuf::from(manifest).join(format!("../../../target/release/{stem}.{ext}")),
    ];
    // Honour a custom CARGO_TARGET_DIR.
    if let Ok(td) = std::env::var("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(&td).join(format!("{profile}/{stem}.{ext}")));
        candidates.push(PathBuf::from(&td).join(format!("debug/{stem}.{ext}")));
        candidates.push(PathBuf::from(&td).join(format!("release/{stem}.{ext}")));
    }

    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            panic!(
                "bundle cdylib not found under {manifest} (tried target/<profile>); \
                 run `cargo build` first"
            )
        })
}

/// Load the bundle and read the registered plugin names in order.
fn registered_names() -> Vec<String> {
    let path = bundle_library_path();
    // SAFETY: the library is our own freshly built bundle.
    let lib = unsafe { Library::new(&path) }.expect("bundle loads");
    // SAFETY: plugin_query_multi is a required export of the bundle.
    let query: libloading::Symbol<'_, QueryMultiFn> =
        unsafe { lib.get(b"plugin_query_multi") }.expect("symbol plugin_query_multi");
    let list_ptr = unsafe { query(CORE_ABI_VERSION) };
    assert!(
        !list_ptr.is_null(),
        "plugin_query_multi must not return NULL"
    );
    // SAFETY: non-null static list owned by the library.
    let list = unsafe { &*list_ptr };
    let infos = unsafe { std::slice::from_raw_parts(list.infos, list.count as usize) };
    infos
        .iter()
        .map(|p| {
            let info = unsafe { &**p };
            unsafe { CStr::from_ptr(info.name) }
                .to_str()
                .expect("plugin name is valid UTF-8")
                .to_string()
        })
        .collect()
}

#[test]
fn dlopen_registers_all_official_plugins() {
    let names = registered_names();
    assert_eq!(
        names.len(),
        4,
        "exactly the 4 official plugins, in mount order"
    );
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
fn dlopen_every_entry_declares_matching_abi_and_vtable() {
    let path = bundle_library_path();
    // SAFETY: our own freshly built bundle.
    let lib = unsafe { Library::new(&path) }.expect("bundle loads");
    // SAFETY: plugin_query_multi is a required export.
    let query: libloading::Symbol<'_, QueryMultiFn> =
        unsafe { lib.get(b"plugin_query_multi") }.expect("symbol plugin_query_multi");
    let list = unsafe { &*query(CORE_ABI_VERSION) };
    let infos = unsafe { std::slice::from_raw_parts(list.infos, list.count as usize) };

    for p in infos {
        let info = unsafe { &**p };
        assert_eq!(
            info.abi_version, CORE_ABI_VERSION,
            "every official plugin must declare ABI 0x000001"
        );
        assert!(!info.vtable.is_null(), "vtable must be non-null");
        assert!(info.phase != 0, "phase mask must be non-empty");
        assert!(info.version != 0, "version must be set");
    }
}

#[test]
fn dlopen_query_multi_is_stable_across_calls() {
    let path = bundle_library_path();
    // SAFETY: our own freshly built bundle.
    let lib = unsafe { Library::new(&path) }.expect("bundle loads");
    // SAFETY: plugin_query_multi is a required export.
    let query: libloading::Symbol<'_, QueryMultiFn> =
        unsafe { lib.get(b"plugin_query_multi") }.expect("symbol plugin_query_multi");
    let a = unsafe { query(CORE_ABI_VERSION) };
    let b = unsafe { query(CORE_ABI_VERSION) };
    assert_eq!(
        a, b,
        "the registry pointer must be stable for the library lifetime"
    );
}

#[test]
fn dlopen_lifecycle_fanout_returns_ok() {
    let path = bundle_library_path();
    // SAFETY: our own freshly built bundle.
    let lib = unsafe { Library::new(&path) }.expect("bundle loads");
    // SAFETY: init/shutdown are required exports of the bundle.
    let init: libloading::Symbol<'_, InitFn> =
        unsafe { lib.get(b"plugin_init") }.expect("symbol plugin_init");
    let shutdown: libloading::Symbol<'_, ShutdownFn> =
        unsafe { lib.get(b"plugin_shutdown") }.expect("symbol plugin_shutdown");

    assert_eq!(unsafe { init(std::ptr::null()) }, 0, "plugin_init → OK");
    // Idempotent: the host calls init once per registered plugin name.
    assert_eq!(unsafe { init(std::ptr::null()) }, 0, "repeated init → OK");
    assert_eq!(unsafe { shutdown() }, 0, "plugin_shutdown → OK");
}

#[test]
fn dlopen_bundle_hosts_no_single_plugin_query_export() {
    // The bundle exposes the multi-plugin contract ONLY. A third-party host
    // that still expects a per-plugin `plugin_query` must get a clean
    // MissingSymbol error, not a wrong-symbol crash.
    let path = bundle_library_path();
    // SAFETY: our own freshly built bundle.
    let lib = unsafe { Library::new(&path) }.expect("bundle loads");
    let single = unsafe { lib.get::<unsafe extern "C" fn() -> *const c_void>(b"plugin_query") };
    assert!(
        single.is_err(),
        "bundle must not export a single-plugin plugin_query"
    );
}
