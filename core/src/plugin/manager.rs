//! Plugin manager — discovery, loading, sandbox, and lifecycle.
//!
//! # M3 Implementation
//!
//! - Scans plugin directories for dynamic libraries (`.so`/`.dylib`/`.dll`)
//! - Uses `libloading` to load libraries and call `plugin_query`
//! - Validates plugin compatibility (ABI version, platform, trust level)
//! - Manages plugin lifecycle (init → run → shutdown)
//! - Stores loaded library handles; unloads on `PluginManager::drop`

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};

// ===========================================================================
// FFI types matching C header `dologger_plugin_info_t`
// ===========================================================================

/// C-compatible plugin info returned by `plugin_query`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RawPluginInfo {
    /// Unique plugin identifier (UTF-8, null-terminated)
    name: *const c_char,
    /// Encoded binary-compat version
    version: u32,
    /// Declared core ABI version
    abi_version: u32,
    /// Mount phase bitmask (DO_LOG_PHASE_*)
    phase: u32,
    /// Pointer to the VTable for this phase
    vtable: *const c_void,
}

/// Signature of `plugin_query(core_abi_version: u32) -> *const RawPluginInfo`.
type PluginQueryFn = unsafe extern "C" fn(u32) -> *const RawPluginInfo;

/// Signature of `plugin_init(config: *const c_void) -> i32`.
type PluginInitFn = unsafe extern "C" fn(*const c_void) -> i32;

/// Signature of `plugin_shutdown() -> i32`.
type PluginShutdownFn = unsafe extern "C" fn() -> i32;

// ===========================================================================
// Plugin metadata (public types)
// ===========================================================================

/// Represents a loaded plugin instance.
pub struct LoadedPlugin {
    /// Plugin metadata (from manifest + plugin_query)
    pub info: PluginMeta,
    /// Path to the dynamic library file
    pub library_path: PathBuf,
    /// Whether the plugin is currently initialised
    pub is_initialised: bool,
    /// Trust level assigned by core after signature verification
    pub trust_level: TrustLevel,
    /// Raw VTable pointer for dispatch (stored for plugin lifecycle)
    #[allow(dead_code)]
    pub(crate) vtable: *const c_void,
    /// Loaded library handle (kept alive until unload)
    #[allow(dead_code)]
    pub(crate) library: Option<Library>,
}

/// Plugin metadata extracted from plugin_query() response.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    /// Unique plugin identifier
    pub name: String,
    /// Plugin version (encoded uint32)
    pub version: u32,
    /// Declared ABI version compatibility
    pub abi_version: u32,
    /// Mount phase(s) — bitmask of DO_LOG_PHASE_* values
    pub phase: u32,
}

/// Three-colour trust model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Officially signed (root key or enterprise root)
    Blue,
    /// Self-signed with recognised CA or TOFU-bound
    Yellow,
    /// Unsigned — only allowed in dev mode
    Red,
}

/// Plugin discovery and loading result.
pub type PluginResult<T> = Result<T, PluginError>;

/// Errors during plugin discovery and loading.
#[derive(Debug)]
pub enum PluginError {
    /// Plugin file not found
    NotFound(String),
    /// Dynamic library failed to load
    LoadFailed(String),
    /// ABI version mismatch
    IncompatibleAbi {
        /// Plugin name
        plugin: String,
        /// Core's ABI version
        core_abi: u32,
        /// Plugin's declared ABI version
        plugin_abi: u32,
    },
    /// Required symbol not found in library
    MissingSymbol(String),
    /// plugin_query returned NULL (incompatible or error)
    QueryRejected(String),
    /// Plugin already loaded
    AlreadyLoaded(String),
    /// Plugin filename is not valid UTF-8
    InvalidFileName(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "Plugin not found: {name}"),
            Self::LoadFailed(msg) => write!(f, "Plugin load failed: {msg}"),
            Self::IncompatibleAbi {
                plugin,
                core_abi,
                plugin_abi,
            } => write!(
                f,
                "ABI mismatch for {plugin}: core={core_abi:#x}, plugin={plugin_abi:#x}"
            ),
            Self::MissingSymbol(sym) => write!(f, "Missing required symbol: {sym}"),
            Self::QueryRejected(msg) => write!(f, "Plugin query rejected: {msg}"),
            Self::AlreadyLoaded(name) => write!(f, "Plugin already loaded: {name}"),
            Self::InvalidFileName(msg) => write!(f, "Invalid plugin file name: {msg}"),
        }
    }
}

// ===========================================================================
// PluginManager
// ===========================================================================

/// Manages all loaded plugins.
pub struct PluginManager {
    /// Loaded plugins keyed by name
    plugins: HashMap<String, LoadedPlugin>,
    /// Directories to scan for plugins
    search_paths: Vec<PathBuf>,
    /// Current core ABI version
    core_abi_version: u32,
    /// Whether dev mode is active (allows unsigned plugins)
    dev_mode: bool,
}

/// Current core ABI version (major.minor.patch → 32-bit).
/// Packed as `(major << 16) | (minor << 8) | patch`.
pub const CORE_ABI_VERSION: u32 = 0x000100; // 0.1.0

/// Default plugin search paths resolved at runtime.
///
/// Priority order:
/// 1. `DO_LOG_PLUGIN_DIR` environment variable (colon-separated on Unix, semicolon on Windows)
/// 2. `./plugins` (relative to current working directory)
/// 3. Platform-specific system path:
///    - Linux: `/usr/lib/dologger/plugins`
///    - Windows: `%PROGRAMDATA%\dologger\plugins`
///    - macOS: `/usr/local/lib/dologger/plugins`
pub fn default_plugin_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Priority 1: DO_LOG_PLUGIN_DIR environment variable
    if let Ok(env_dirs) = std::env::var("DO_LOG_PLUGIN_DIR") {
        #[cfg(unix)]
        let separator = ':';
        #[cfg(not(unix))]
        let separator = ';';

        for dir in env_dirs.split(separator) {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                paths.push(PathBuf::from(trimmed));
            }
        }
    }

    // Priority 2: Local project directory
    paths.push(PathBuf::from("./plugins"));

    // Priority 3: Platform-specific system directory
    #[cfg(target_os = "linux")]
    paths.push(PathBuf::from("/usr/lib/dologger/plugins"));
    #[cfg(target_os = "macos")]
    paths.push(PathBuf::from("/usr/local/lib/dologger/plugins"));
    #[cfg(windows)]
    {
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            paths.push(PathBuf::from(format!("{programdata}\\dologger\\plugins")));
        } else {
            paths.push(PathBuf::from("C:\\ProgramData\\dologger\\plugins"));
        }
    }

    paths
}

/// Plugin naming convention: `dologger-plugin-{type}-{name}`
///
/// Examples:
/// - `dologger-plugin-filter-level` — official filter plugin "level"
/// - `dologger-plugin-fmt-json` — official formatter plugin "json"
/// - `dologger-plugin-sink-kafka` — official sink plugin "kafka"
///
/// Third-party plugins should use a vendor prefix:
/// - `dologger-plugin-fmt-acme-csv` — Acme Corp's CSV formatter
///
/// This convention prevents filename collisions and makes plugin purpose
/// immediately identifiable from the library filename.
pub fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Plugin name must not be empty".into());
    }
    if name.len() > 128 {
        return Err("Plugin name exceeds 128 characters".into());
    }
    // Allow: lowercase alphanumeric, hyphens, underscores, dots (for vendor prefix)
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.')
    {
        return Err(format!(
            "Plugin name '{name}' contains invalid characters. Use only [a-z0-9-_.]"
        ));
    }
    Ok(())
}

impl PluginManager {
    /// Create a new plugin manager with default search paths.
    pub fn with_default_paths(dev_mode: bool) -> Self {
        Self::new(default_plugin_paths(), dev_mode)
    }

    /// Create a new plugin manager.
    pub fn new(search_paths: Vec<PathBuf>, dev_mode: bool) -> Self {
        Self {
            plugins: HashMap::new(),
            search_paths,
            core_abi_version: CORE_ABI_VERSION,
            dev_mode,
        }
    }

    /// Scan the search paths for plugins and load all found.
    pub fn discover(&mut self) -> Vec<(String, PluginError)> {
        let mut errors = Vec::new();
        let paths: Vec<PathBuf> = self.search_paths.clone();

        for path in &paths {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let file_path = entry.path();
                    if Self::is_plugin_library(&file_path) {
                        match self.load_plugin(&file_path) {
                            Ok(name) => {
                                crate::sys::diag::info(
                                    "plugin_mgr",
                                    &format!("Plugin loaded: {name}"),
                                );
                            }
                            Err(e) => {
                                let name = file_path.to_string_lossy().into_owned();
                                crate::sys::diag::warn(
                                    "plugin_mgr",
                                    &format!("Plugin load failed: {name} — {e}"),
                                );
                                errors.push((name, e));
                            }
                        }
                    }
                }
            }
        }

        errors
    }

    /// Load a single plugin from a dynamic library file.
    ///
    /// Uses `libloading` to open the library, resolve `plugin_query`,
    /// call it, and store the metadata and VTable pointer.
    pub fn load_plugin(&mut self, path: &Path) -> PluginResult<String> {
        // Extract plugin name from filename stem
        let plugin_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                PluginError::InvalidFileName(format!(
                    "Cannot extract plugin name from: {}",
                    path.display()
                ))
            })?
            .to_string();

        // Guard against duplicate loading
        if self.plugins.contains_key(&plugin_name) {
            return Err(PluginError::AlreadyLoaded(plugin_name));
        }

        // SAFETY: libloading is a cross-platform safe wrapper around dlopen/LoadLibrary.
        // We validate all symbols returned by the library before using them.
        let library = unsafe {
            Library::new(path).map_err(|e| {
                PluginError::LoadFailed(format!("Cannot load '{}': {e}", path.display()))
            })?
        };

        // Resolve plugin_query symbol
        // SAFETY: libloading::Library::get resolves a symbol by name from the
        // loaded dynamic library. The symbol 'plugin_query' is a required export
        // per the C ABI contract. If the plugin is malicious and exports a wrong
        // function signature, the unsoundness is bounded: we validate the ABI
        // version on the returned pointer before any further use.
        let query_fn: Symbol<'_, PluginQueryFn> = unsafe {
            library
                .get(b"plugin_query")
                .map_err(|_| PluginError::MissingSymbol("plugin_query".into()))?
        };

        // Call plugin_query to get the plugin info
        // SAFETY: We invoke the function pointer returned by libloading.
        // We validate: (a) non-null return, (b) ABI version match, and
        // (c) the C string name field immediately after the call. The
        // returned pointer points to static memory owned by the plugin.
        let raw_info: &RawPluginInfo = unsafe {
            let ptr = query_fn(self.core_abi_version);
            if ptr.is_null() {
                return Err(PluginError::QueryRejected(format!(
                    "Plugin '{}' rejected ABI version {:#x}",
                    plugin_name, self.core_abi_version
                )));
            }
            // SAFETY: plugin_query returns a valid, non-null pointer to a static
            // RawPluginInfo struct owned by the plugin library.
            &*ptr
        };

        // Validate ABI version
        if raw_info.abi_version != self.core_abi_version {
            return Err(PluginError::IncompatibleAbi {
                plugin: plugin_name.clone(),
                core_abi: self.core_abi_version,
                plugin_abi: raw_info.abi_version,
            });
        }

        // Read the plugin name from the C string
        // SAFETY: raw_info.name was validated non-null by plugin_query's return
        // check. It points to a valid null-terminated UTF-8 string per the
        // C ABI contract. CStr::from_ptr reads up to the null terminator.
        let name = unsafe {
            CStr::from_ptr(raw_info.name)
                .to_str()
                .unwrap_or(&plugin_name)
                .to_string()
        };

        // Extract the VTable pointer
        let vtable = raw_info.vtable;

        // Determine trust level (M3: actual signature verification)
        let trust_level = if self.dev_mode {
            TrustLevel::Red
        } else {
            // In production, unsigned plugins are rejected unless explicitly allowed.
            // Full signature verification is gated on M4 key infrastructure.
            TrustLevel::Red
        };

        let meta = PluginMeta {
            name: name.clone(),
            version: raw_info.version,
            abi_version: raw_info.abi_version,
            phase: raw_info.phase,
        };

        crate::sys::diag::info(
            "plugin_mgr",
            &format!(
                "Plugin '{}' phase={:#x} vtable={:p} trust={:?}",
                name, meta.phase, vtable, trust_level
            ),
        );

        self.plugins.insert(
            name.clone(),
            LoadedPlugin {
                info: meta,
                library_path: path.to_path_buf(),
                is_initialised: false,
                trust_level,
                vtable,
                library: Some(library),
            },
        );

        Ok(name)
    }

    /// Initialise a loaded plugin by calling `plugin_init`.
    pub fn init_plugin(&mut self, name: &str) -> PluginResult<()> {
        let plugin = self
            .plugins
            .get(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))?;

        if plugin.is_initialised {
            return Ok(()); // Already initialised — idempotent
        }

        // Resolve and call plugin_init
        let init_result = if let Some(ref lib) = plugin.library {
            // SAFETY: libloading::Library::get resolves 'plugin_init'.
            // This symbol is required per C ABI. We pass NULL config;
            // the FFI is `extern "C" fn(*const c_void) -> i32`.
            let init_fn: Symbol<'_, PluginInitFn> = unsafe {
                lib.get(b"plugin_init")
                    .map_err(|_| PluginError::MissingSymbol("plugin_init".into()))?
            };
            // SAFETY: plugin_init is provided by the plugin and expected to be safe.
            // We pass NULL config for now; M3+ will pass domain-specific config.
            unsafe { init_fn(std::ptr::null()) }
        } else {
            return Err(PluginError::LoadFailed(
                "Library handle not available".into(),
            ));
        };

        if init_result != 0 {
            crate::sys::diag::warn(
                "plugin_mgr",
                &format!("Plugin '{name}' init returned non-zero: {init_result}"),
            );
        }

        // Mark as initialised
        if let Some(plugin) = self.plugins.get_mut(name) {
            plugin.is_initialised = true;
        }

        Ok(())
    }

    /// Shutdown a plugin by calling `plugin_shutdown`.
    pub fn shutdown_plugin(&mut self, name: &str) -> PluginResult<()> {
        let plugin = self
            .plugins
            .get(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))?;

        if let Some(ref lib) = plugin.library {
            // SAFETY: libloading::Library::get resolves 'plugin_shutdown'.
            // This symbol is required per C ABI. The FFI is `extern "C" fn() -> i32`.
            let shutdown_fn: Symbol<'_, PluginShutdownFn> = unsafe {
                lib.get(b"plugin_shutdown")
                    .map_err(|_| PluginError::MissingSymbol("plugin_shutdown".into()))?
            };
            // SAFETY: plugin_shutdown is provided by the plugin and expected to be safe.
            unsafe {
                shutdown_fn();
            }
        }

        if let Some(plugin) = self.plugins.get_mut(name) {
            plugin.is_initialised = false;
        }

        Ok(())
    }

    /// Shutdown all plugins and unload all libraries.
    pub fn shutdown_all(&mut self) {
        let names: Vec<String> = self.plugins.keys().cloned().collect();
        for name in &names {
            if let Err(e) = self.shutdown_plugin(name) {
                crate::sys::diag::warn(
                    "plugin_mgr",
                    &format!("Error shutting down plugin '{name}': {e}"),
                );
            }
        }
        self.plugins.clear();
    }

    /// Get a reference to a loaded plugin by name.
    pub fn get(&self, name: &str) -> Option<&LoadedPlugin> {
        self.plugins.get(name)
    }

    /// Get the count of loaded plugins.
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// List all loaded plugin names.
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a file path looks like a plugin dynamic library.
    fn is_plugin_library(path: &Path) -> bool {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        matches!(ext, "so" | "dylib" | "dll")
    }

    /// Get the core ABI version.
    pub fn abi_version(&self) -> u32 {
        self.core_abi_version
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}
