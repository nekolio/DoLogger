//! Official DoLogger FieldProvider plugin — `field_container`.
//!
//! Injects container orchestration metadata into every log record:
//! container ID (from /proc/self/cgroup or `$CONTAINER_ID`), pod name,
//! namespace, and node name (from the Kubernetes downward API).
//! Phase: FieldProvider (field injection, stage 2).
//!
//! # M6 — implemented via the host-accessor bridge
//!
//! The record handle is opaque to the plugin. The engine hands it a
//! [`HostAccessors`] table at `plugin_init`; `provide` writes container.* fields
//! through `accessors.field_set`. Container metadata is detected ONCE at `init`
//! (it does not change at runtime) and cached, so the per-record `provide` path
//! is a few `field_set` calls with no filesystem or env reads.
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
use dologger_core::plugin::vtable::{FieldProviderVTable, HostAccessors, HostInit};

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
#[allow(dead_code)]
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0101;

// Plugin mount phase — FieldProvider stage
const PHASE_FIELD_PROVIDER: u32 = dologger_core::plugin::phase::PHASE_FIELD_PROVIDER;

// Plugin info versioning — abi_version MUST match the core's declared ABI
// (0.1.0); the host validates it when the bundle is loaded.
const CORE_ABI_VERSION: u32 = dologger_core::plugin::CORE_ABI_VERSION;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// ---------------------------------------------------------------------------
// Container source selection (parsed from the `source` config key)
// ---------------------------------------------------------------------------

/// Which container runtime(s) to probe for metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldSource {
    /// Probe everything: env vars + /proc/self/cgroup + Kubernetes downward API.
    Auto,
    /// Docker/Podman-style cgroup detection only.
    Container,
    /// Kubernetes downward API (`$POD_NAME`, `$POD_NAMESPACE`, `$NODE_NAME`).
    K8s,
}

impl FieldSource {
    const fn default_source() -> Self {
        Self::Auto
    }
}

impl Default for FieldSource {
    fn default() -> Self {
        Self::default_source()
    }
}

/// Detected, cached container metadata (empty = nothing to inject).
#[derive(Debug, Clone)]
struct ContainerMeta {
    /// `container.id`
    id: String,
    /// `container.pod`
    pod: String,
    /// `container.namespace`
    namespace: String,
    /// `container.node`
    node: String,
}

impl ContainerMeta {
    /// Empty metadata. `const` so it can initialise the [`META`] static.
    const fn empty() -> Self {
        Self {
            id: String::new(),
            pod: String::new(),
            namespace: String::new(),
            node: String::new(),
        }
    }
}

impl Default for ContainerMeta {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// Static state — accessor bridge + source + cached metadata
// ---------------------------------------------------------------------------

/// Host accessor bridge captured at `plugin_init`; used by `provide`.
static HOST: Mutex<Option<HostAccessors>> = Mutex::new(None);

/// Container source selected at `plugin_init`.
static SOURCE: Mutex<FieldSource> = Mutex::new(FieldSource::default_source());

/// Container metadata detected once at `plugin_init`, injected on every record.
static META: Mutex<ContainerMeta> = Mutex::new(ContainerMeta::empty());

// ---------------------------------------------------------------------------
// Container detection (runs once at init)
// ---------------------------------------------------------------------------

/// Best-effort container-id extraction from `/proc/self/cgroup`.
///
/// Matches the well-known Kubernetes/Docker/Podman cgroup formats:
/// `.../docker/<id>`, `.../docker/<id>/...`, `.../kubepods.../<pod>/<id>`,
/// `.../crio-<id>`, `.../podman-<id>`. Returns `None` when no cgroup ID is
/// found or the file is unreadable (e.g. on Windows, or non-container hosts).
fn container_id_from_cgroup() -> Option<String> {
    let text = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    for line in text.lines() {
        // Kubernetes / cri-o: `.../cri-containerd-<id>.scope` or `crio-<id>`.
        if let Some(idx) = line.find("cri-containerd-") {
            let id = &line[idx + "cri-containerd-".len()..];
            let id = id.trim_end_matches(".scope");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
        if let Some(idx) = line.find("crio-") {
            let id = &line[idx + "crio-".len()..];
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
        // Docker / Podman: the last `/`-delimited segment after `/docker/`,
        // `/podman/`, or a `docker-<id>.scope` segment.
        for marker in ["/docker/", "/podman/"] {
            if let Some(idx) = line.find(marker) {
                let rest = &line[idx + marker.len()..];
                let id = rest.split('/').find(|s| !s.is_empty())?;
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
        // Generic `docker-<64hex>.scope` / `podman-<64hex>.scope`.
        for prefix in ["docker-", "podman-"] {
            if let Some(idx) = line.find(prefix) {
                let id = &line[idx + prefix.len()..];
                let id = id.trim_end_matches(".scope");
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

/// Detect and cache container metadata for the selected [`FieldSource`].
fn detect_metadata(source: FieldSource) -> ContainerMeta {
    use std::env;

    let mut meta = ContainerMeta::default();

    // Container ID — from cgroup (auto/container) or $CONTAINER_ID override.
    let mut id = if source != FieldSource::K8s {
        container_id_from_cgroup()
    } else {
        None
    };
    if id.is_none() {
        id = env::var("CONTAINER_ID").ok();
    }
    // Podman also exposes the id via $HOSTNAME inside a container.
    if id.is_none() {
        id = env::var("HOSTNAME").ok();
    }
    meta.id = id.unwrap_or_default();

    // Kubernetes downward API (auto/k8s).
    if source != FieldSource::Container {
        meta.pod = env::var("POD_NAME").unwrap_or_default();
        meta.namespace = env::var("POD_NAMESPACE").unwrap_or_default();
        meta.node = env::var("NODE_NAME").unwrap_or_default();
    }

    meta
}

// ---------------------------------------------------------------------------
// VTable function: provide
// ---------------------------------------------------------------------------

/// Inject cached `container.*` fields into the record via the host accessor.
///
/// Returns the number of fields injected, or a negative `DO_LOG_ERR_*` code.
/// Field writes go through the host's `field_set` (Ring 3 caller), which is
/// the plugin's only sanctioned way to touch the opaque record handle.
unsafe extern "C" fn provide_impl(
    _record: *mut std::ffi::c_void,
    _config: *mut std::ffi::c_void,
) -> i32 {
    // SAFETY: the record pointer comes from the engine (opaque handle). We only
    // hand it back to the host accessor, which casts it under its own contract.
    let record = _record;

    let accessors = match *HOST.lock().unwrap() {
        Some(a) => a,
        None => return -1, // init() not called — no bridge available
    };

    let meta = META.lock().unwrap();
    let mut injected = 0;

    macro_rules! inject {
        ($name:expr, $value:expr) => {
            if !$value.is_empty() {
                // SAFETY: build a NUL-terminated C string from the Rust value.
                // Both `field_name` and `value` must be NUL-terminated for the
                // C accessor. Non-UTF8 (impossible here) leaves the field unset.
                if let (Ok(name), Ok(value)) = (CString::new($name), CString::new($value.as_str()))
                {
                    let rc = (accessors.field_set)(record, name.as_ptr(), value.as_ptr());
                    if rc >= 0 {
                        injected += 1;
                    }
                }
            }
        };
    }

    inject!("container.id", meta.id);
    inject!("container.pod", meta.pod);
    inject!("container.namespace", meta.namespace);
    inject!("container.node", meta.node);

    injected
}

/// The plugin's VTable — `provide` (matches `dologger_field_provider_vtable_t`).
static VTABLE: FieldProviderVTable = FieldProviderVTable {
    provide: provide_impl,
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
    phase: PHASE_FIELD_PROVIDER,
    vtable: &VTABLE as *const FieldProviderVTable as *const std::ffi::c_void,
};

/// Accessor for the registry entry (used by tests and the bundle crate).
pub fn plugin_info() -> &'static DologgerPluginInfo {
    &INFO
}

// ---------------------------------------------------------------------------
// Lifecycle — called by the bundle's `plugin_init` fan-out
// ---------------------------------------------------------------------------

/// Initialise the plugin: capture the host-accessor bridge, select the
/// container source from `config_json`, and cache detected metadata.
///
/// `config` points to a [`HostInit`] (`dologger_host_init_t`): `.accessors` is
/// the host-accessor bridge, `.config_json` is a JSON object
/// (`{"source":"auto|container|k8s"}`) or NULL for defaults.
pub fn init(config: *const std::ffi::c_void) -> i32 {
    // SAFETY: config is NULL (defaults) or a HostInit pointer from the host.
    let accessors = if config.is_null() {
        HostAccessors::default()
    } else {
        // SAFETY: non-null config is a HostInit* from the engine (M6 bridge).
        unsafe { &*(config as *const HostInit) }.accessors
    };
    *HOST.lock().unwrap() = Some(accessors);

    let mut source = FieldSource::default_source();
    if !config.is_null() {
        // SAFETY: non-null config is a HostInit*; config_json is NULL or a
        // NUL-terminated JSON string.
        let cfg_json = unsafe { &*(config as *const HostInit) }.config_json;
        if !cfg_json.is_null() {
            // SAFETY: config_json validated non-null above.
            if let Ok(s) = unsafe { CStr::from_ptr(cfg_json) }.to_str() {
                if let Some(s) = s
                    .find('"')
                    .and_then(|_| s.split_once("source").map(|(_, v)| v))
                    .and_then(|v| {
                        v.trim()
                            .trim_start_matches(':')
                            .trim()
                            .trim_matches('"')
                            .to_string()
                            .into()
                    })
                {
                    source = match s.as_str() {
                        "container" | "docker" | "podman" => FieldSource::Container,
                        "k8s" | "kubernetes" => FieldSource::K8s,
                        _ => FieldSource::Auto,
                    };
                }
            }
        }
    }
    *SOURCE.lock().unwrap() = source;

    // Detect + cache metadata once. Override the env for testing by clearing
    // cgroup reads is not needed — detection is deterministic per env.
    *META.lock().unwrap() = detect_metadata(source);

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

    /// Serializes tests that mutate the process-global [`HOST`] bridge static.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_plugin_info_returns_valid_entry() {
        let info = plugin_info();
        assert_eq!(info.abi_version, CORE_ABI_VERSION);
        assert_eq!(info.phase, PHASE_FIELD_PROVIDER);
        let name = unsafe { CStr::from_ptr(info.name) }.to_str().unwrap();
        assert_eq!(name, "field-container");
    }

    #[test]
    fn test_init_null_config_uses_defaults_and_captures_bridge() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert_eq!(init(std::ptr::null()), DO_LOG_OK);
        // The host-accessor bridge must be captured (non-NULL function pointers).
        let guard = HOST.lock().unwrap();
        let a = guard.expect("bridge captured");
        assert_ne!(a.field_get as usize, 0);
        assert_ne!(a.field_set as usize, 0);
        assert_ne!(a.alloc as usize, 0);
        assert_ne!(a.free as usize, 0);
    }

    #[test]
    fn test_init_accepts_hostinit_config() {
        let _guard = TEST_LOCK.lock().unwrap();
        let hi = HostInit::default();
        assert_eq!(
            init(&hi as *const HostInit as *const std::ffi::c_void),
            DO_LOG_OK
        );
    }

    #[test]
    fn test_provide_without_init_returns_error() {
        let _guard = TEST_LOCK.lock().unwrap();
        // Ensure a clean slate (no bridge captured).
        *HOST.lock().unwrap() = None;
        let rc = unsafe { provide_impl(std::ptr::null_mut(), std::ptr::null_mut()) };
        assert!(rc < 0, "no bridge -> error, got {rc}");
    }

    #[test]
    fn test_vtable_provide_field_is_non_null() {
        assert_ne!(VTABLE.provide as usize, 0);
    }

    #[test]
    fn test_field_provider_entry_from_vtable() {
        let entry = dologger_core::plugin::vtable::FieldProviderEntry {
            provide: VTABLE.provide,
            config: std::ptr::null_mut(),
        };
        assert_ne!(entry.provide as usize, 0);
    }

    #[test]
    fn test_detect_metadata_handles_absent_cgroup() {
        // On a non-container / Windows host, cgroup reads fail gracefully and
        // env detection returns empty strings — never panics, never injects
        // garbage. (Container presence is environment-dependent, so we only
        // assert the function runs and returns a well-formed struct.)
        let meta = detect_metadata(FieldSource::Auto);
        let _ = meta.id;
        let _ = meta.pod;
        let _ = meta.namespace;
        let _ = meta.node;
    }
}
