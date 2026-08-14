//! Plugin manager — discovery, loading, sandbox, and lifecycle.
//!
//! # Implementation
//!
//! - Scans plugin directories for dynamic libraries (`.so`/`.dylib`/`.dll`)
//! - Uses `libloading` to load libraries and call `plugin_query` /
//!   `plugin_query_multi`
//! - Validates plugin compatibility (ABI version, platform, trust level)
//! - Verifies the Ed25519 `.sig` sidecar against the active trust anchors
//!   (multi-key), rejects signatures from revoked keys, and enforces the Red
//!   gate (unsigned plugins rejected outside dev mode)
//! - Manages plugin lifecycle (init → run → shutdown)
//! - Stores loaded library handles; unloads on `PluginManager::drop`
//!
//! # Deferred
//!
//! Runtime sandbox enforcement (seccomp-bpf / AppContainer / Sandbox) and
//! per-plugin quotas are not wired here yet: the sandbox modules and quota
//! types exist as dead code in the tree, and no plugin is actually confined
//! at v0.1.0. The trust gate above is the only load-time security boundary
//! that is enforced today.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_void, CStr};
use std::path::{Path, PathBuf};

use crate::security::fingerprint_key;
use ed25519_dalek::{Signature, VerifyingKey};
use libloading::{Library, Symbol};
use std::sync::Arc;

// ===========================================================================
// FFI types matching C header `dologger_plugin_info_t`
// ===========================================================================

/// Signature of `plugin_query(core_abi_version: u32) -> *const DologgerPluginInfo`.
/// Single-plugin libraries (third-party) export this.
type PluginQueryFn = unsafe extern "C" fn(u32) -> *const crate::ffi::DologgerPluginInfo;

/// Signature of `plugin_query_multi(core_abi_version: u32) -> *const DologgerPluginInfoList`.
/// Bundle libraries (the official plugins) export this to host several plugins
/// in one dynamic library — see `dologger_plugin_info_list_t` in the C header.
type PluginQueryMultiFn = unsafe extern "C" fn(u32) -> *const crate::ffi::DologgerPluginInfoList;

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
    /// Loaded library handle (kept alive until unload). Arc so every plugin
    /// registered from a shared bundle library holds its own reference and
    /// the library is unloaded only when the last one drops.
    #[allow(dead_code)]
    pub(crate) library: Option<Arc<Library>>,
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
    /// Ed25519 signature sidecar present but failed to verify
    SignatureInvalid {
        /// Plugin name
        plugin: String,
        /// Why the signature was rejected
        reason: String,
    },
    /// Unsigned (Red) plugin loaded in a mode that forbids it
    UnsignedRejected {
        /// Plugin name
        plugin: String,
    },
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
            Self::SignatureInvalid { plugin, reason } => {
                write!(f, "Signature invalid for {plugin}: {reason}")
            }
            Self::UnsignedRejected { plugin } => write!(
                f,
                "Unsigned plugin rejected (set allow_red_plugins or dev mode): {plugin}"
            ),
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
    /// Active trust anchors (Ed25519 public keys) that grant Blue trust to
    /// plugins whose `.sig` sidecar verifies against ANY of them. Empty = no
    /// verification; every plugin is treated as unsigned (Red).
    trust_anchors: Vec<[u8; 32]>,
    /// SHA-256 fingerprints of revoked signing keys. A revoked key can never
    /// grant Blue, even if its public key is still listed in `trust_anchors`
    /// (overlap defense — see [`PluginManager::revoke_trust_anchor`]).
    revoked: HashSet<[u8; 32]>,
    /// Whether unsigned (Red) plugins may load outside dev mode.
    allow_red_plugins: bool,
}

// SAFETY: The registry holds `libloading::Library` handles (which are
// conservatively `!Send + !Sync`). In practice a `PluginManager` is populated
// on the management path (CLI) and, once embedded in an `Engine`, is never
// used during the logging hot path — the engine does not call `discover` or
// load/unload at runtime. All accessor/loader methods take `&self`/`&mut self`
// and are serialized by the caller. Sending a fully-populated registry across
// threads and unloading libraries from a different thread than they were
// loaded on is not supported; the concurrent-sharing guarantee is limited to
// the inert registry embedded in an `Engine`. This matches the existing
// `unsafe impl Send/Sync` pattern for the ring buffer and record pool.
unsafe impl Send for PluginManager {}
// SAFETY: see Send impl — the registry is inert until the Engine wires its
// concurrency; accessor/loader methods are serialized by the caller.
unsafe impl Sync for PluginManager {}

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
/// - `dologger-plugin-formatter-json` — official formatter plugin "json"
/// - `dologger-plugin-field-container` — official FieldProvider plugin "container"
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
            trust_anchors: Vec::new(),
            revoked: HashSet::new(),
            allow_red_plugins: false,
        }
    }

    /// Set the active trust anchor to a single key, replacing the set.
    ///
    /// Legacy compatibility shim — the loader now supports multiple anchors
    /// ([`PluginManager::set_trust_anchors`], [`PluginManager::add_trust_anchor`],
    /// [`PluginManager::load_trust_store`]). Kept so the CLI env-anchor path
    /// and existing tests work unchanged. Does NOT clear the revoked set.
    pub fn set_trust_anchor(&mut self, pubkey: [u8; 32]) {
        self.trust_anchors = vec![pubkey];
    }

    /// Replace the active trust-anchor set in one call.
    pub fn set_trust_anchors(&mut self, anchors: Vec<[u8; 32]>) {
        self.trust_anchors = anchors;
    }

    /// Add one more trust anchor to the active set (deduplicated).
    pub fn add_trust_anchor(&mut self, pubkey: [u8; 32]) {
        if !self.trust_anchors.contains(&pubkey) {
            self.trust_anchors.push(pubkey);
        }
    }

    /// Permanently revoke a key fingerprint: add it to the denied set AND
    /// drop any active anchor whose fingerprint matches. A revoked key can
    /// never grant Blue, even if it is still listed in `active.pub`.
    pub fn revoke_trust_anchor(&mut self, fingerprint: [u8; 32]) {
        self.revoked.insert(fingerprint);
        self.trust_anchors.retain(|anchor| {
            VerifyingKey::from_bytes(anchor)
                .map(|vk| fingerprint_key(&vk) != fingerprint)
                .unwrap_or(true)
        });
    }

    /// Load a committed plugin trust store, replacing both the active-anchor
    /// set and the revoked set.
    ///
    /// File layout (`#` at the start of a line is a comment; blank lines are
    /// skipped; every line is trimmed):
    ///
    /// - `active.pub`  — one 64-hex Ed25519 public key per line.
    /// - `revoked.txt` — `<64-hex SHA-256 fingerprint> [reason] [unix-ts]`,
    ///   one per line. `reason` must be a known [`CrlReason`] string
    ///   (`compromised`, `superseded`, `deactivated`); the timestamp is
    ///   informational and is not validated.
    ///
    /// `active.pub` must exist; `revoked.txt` may be absent (treated as
    /// empty). A malformed line fails the whole load with a `file:line`
    /// message so a corrupt store can never silently weaken trust.
    pub fn load_trust_store(&mut self, dir: &Path) -> Result<(), String> {
        let mut anchors = Vec::new();
        let active = dir.join("active.pub");
        let text = std::fs::read_to_string(&active)
            .map_err(|e| format!("Cannot read '{}': {e}", active.display()))?;
        for (idx, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bytes = hex::decode(line).map_err(|e| {
                format!(
                    "{}:{}: not a valid hex public key ({e})",
                    active.display(),
                    idx + 1
                )
            })?;
            if bytes.len() != 32 {
                return Err(format!(
                    "{}:{}: expected 32 bytes (64 hex chars), got {}",
                    active.display(),
                    idx + 1,
                    bytes.len()
                ));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            anchors.push(key);
        }

        let mut revoked = HashSet::new();
        let crl = dir.join("revoked.txt");
        if crl.exists() {
            let text = std::fs::read_to_string(&crl)
                .map_err(|e| format!("Cannot read '{}': {e}", crl.display()))?;
            for (idx, raw) in text.lines().enumerate() {
                let line = raw.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut tokens = line.split_whitespace();
                let fp_hex = tokens.next().unwrap_or("");
                let bytes = hex::decode(fp_hex).map_err(|e| {
                    format!(
                        "{}:{}: not a valid hex fingerprint ({e})",
                        crl.display(),
                        idx + 1
                    )
                })?;
                if bytes.len() != 32 {
                    return Err(format!(
                        "{}:{}: expected 32-byte fingerprint",
                        crl.display(),
                        idx + 1
                    ));
                }
                if let Some(reason) = tokens.next() {
                    if crate::security::CrlReason::parse(reason).is_none() {
                        return Err(format!(
                            "{}:{}: unknown revocation reason '{reason}'",
                            crl.display(),
                            idx + 1
                        ));
                    }
                }
                let mut fp = [0u8; 32];
                fp.copy_from_slice(&bytes);
                revoked.insert(fp);
            }
        }

        self.trust_anchors = anchors;
        self.revoked = revoked;
        Ok(())
    }

    /// Allow unsigned (Red) plugins to load outside dev mode.
    pub fn set_allow_red_plugins(&mut self, allow: bool) {
        self.allow_red_plugins = allow;
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
                            Ok(names) => {
                                for name in &names {
                                    crate::sys::diagnostics::info(
                                        "plugin_mgr",
                                        &format!("Plugin loaded: {name}"),
                                    );
                                }
                            }
                            Err(e) => {
                                let name = file_path.to_string_lossy().into_owned();
                                crate::sys::diagnostics::warn(
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

    /// Load one or more plugins from a dynamic library file.
    ///
    /// A library may host several plugins via the `plugin_query_multi`
    /// registry (the official plugins bundle) or a single plugin via
    /// `plugin_query` (third-party libraries). Returns every plugin name
    /// registered from this library.
    pub fn load_plugin(&mut self, path: &Path) -> PluginResult<Vec<String>> {
        // Extract plugin name from filename stem (fallback name only — the
        // authoritative name comes from plugin_query / plugin_query_multi).
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

        // SAFETY: libloading is a cross-platform safe wrapper around dlopen/LoadLibrary.
        // We validate all symbols returned by the library before using them.
        let library = unsafe {
            Library::new(path).map_err(|e| {
                PluginError::LoadFailed(format!("Cannot load '{}': {e}", path.display()))
            })?
        };

        // 1) Multi-plugin registry — the official plugins bundle. One dlopen
        // registers every member; all of them share this library handle.
        // SAFETY: `plugin_query_multi` is an optional export; `library.get`
        // binds the C-ABI function pointer of the declared type.
        let multi_symbol =
            unsafe { library.get::<Symbol<'_, PluginQueryMultiFn>>(b"plugin_query_multi") };
        if let Ok(multi) = multi_symbol {
            // SAFETY: `multi` is a C-ABI function pointer; the library is
            // alive (held in `library`) and returns a pointer we null-check.
            let list_ptr = unsafe { multi(self.core_abi_version) };
            if list_ptr.is_null() {
                return Err(PluginError::QueryRejected(format!(
                    "Plugin bundle '{}' rejected ABI version {:#x}",
                    plugin_name, self.core_abi_version
                )));
            }
            // SAFETY: plugin_query_multi returned a non-null pointer to a
            // static DologgerPluginInfoList owned by the library.
            let list = unsafe { &*list_ptr };
            // SAFETY: `infos` is an array of `count` static DologgerPluginInfo
            // entries owned by the library, valid for its lifetime.
            let infos = unsafe { std::slice::from_raw_parts(list.infos, list.count as usize) };
            let lib = Arc::new(library);
            let mut names = Vec::with_capacity(infos.len());
            for info_ptr in infos {
                // SAFETY: each entry points to a static DologgerPluginInfo
                // owned by the library, valid for the library's lifetime.
                let info = unsafe { &**info_ptr };
                names.push(self.register_plugin(path, lib.clone(), info, &plugin_name)?);
            }
            return Ok(names);
        }

        // 2) Single-plugin contract (third-party / standalone libraries).
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

        // Call plugin_query to get the plugin info.
        // SAFETY: We invoke the function pointer returned by libloading and
        // validate the returned struct before any further use.
        let info_ptr = unsafe { query_fn(self.core_abi_version) };
        if info_ptr.is_null() {
            return Err(PluginError::QueryRejected(format!(
                "Plugin '{}' rejected ABI version {:#x}",
                plugin_name, self.core_abi_version
            )));
        }
        // SAFETY: plugin_query returns a valid, non-null pointer to a static
        // DologgerPluginInfo owned by the library.
        let info = unsafe { &*info_ptr };
        let name = self.register_plugin(path, Arc::new(library), info, &plugin_name)?;
        Ok(vec![name])
    }

    /// Validate a plugin info struct, insert it into the manager, and return
    /// its registered name. Shared by the single- and multi-plugin paths.
    fn register_plugin(
        &mut self,
        path: &Path,
        library: Arc<Library>,
        info: &crate::ffi::DologgerPluginInfo,
        fallback_name: &str,
    ) -> PluginResult<String> {
        // Validate ABI version
        if info.abi_version != self.core_abi_version {
            return Err(PluginError::IncompatibleAbi {
                plugin: fallback_name.to_string(),
                core_abi: self.core_abi_version,
                plugin_abi: info.abi_version,
            });
        }

        // Read the plugin name from the C string.
        // SAFETY: info.name points to a valid null-terminated UTF-8 string per
        // the C ABI contract. CStr::from_ptr reads up to the null terminator.
        let name = unsafe { CStr::from_ptr(info.name) }
            .to_str()
            .unwrap_or(fallback_name)
            .to_string();

        // Guard against duplicate loading
        if self.plugins.contains_key(&name) {
            return Err(PluginError::AlreadyLoaded(name));
        }

        // Determine trust from the Ed25519 `.sig` sidecar against the
        // configured trust anchor. A present-but-failing signature is a hard
        // error (SignatureInvalid); absent signature yields Red.
        let trust_level = self.determine_trust(path, &name)?;

        // Red gate — unsigned plugins load only in dev mode or when the
        // operator explicitly allows them via set_allow_red_plugins.
        if trust_level == TrustLevel::Red && !self.dev_mode && !self.allow_red_plugins {
            return Err(PluginError::UnsignedRejected { plugin: name });
        }

        let meta = PluginMeta {
            name: name.clone(),
            version: info.version,
            abi_version: info.abi_version,
            phase: info.phase,
        };

        crate::sys::diagnostics::info(
            "plugin_mgr",
            &format!(
                "Plugin '{}' phase={:#x} vtable={:p} trust={:?}",
                name, meta.phase, info.vtable, trust_level
            ),
        );

        self.plugins.insert(
            name.clone(),
            LoadedPlugin {
                info: meta,
                library_path: path.to_path_buf(),
                is_initialised: false,
                trust_level,
                vtable: info.vtable,
                library: Some(library),
            },
        );

        Ok(name)
    }

    /// Determine a plugin's trust tier from its Ed25519 signature sidecar.
    ///
    /// Sidecar naming: `<library>.sig` sits next to the library
    /// (e.g. `libfoo.so` → `libfoo.so.sig`, `dologger_official_plugins.dll`
    /// → `dologger_official_plugins.dll.sig`).
    ///
    /// A signature is granted Blue if it verifies against ANY active, non-
    /// revoked anchor (multi-key parallel verification). A signature that
    /// only matches a REVOKED anchor is rejected with `SignatureInvalid` —
    /// revocation is enforced here, so it propagates regardless of dev mode
    /// or `allow_red_plugins`.
    ///
    /// | Condition | Result |
    /// | :- | :- |
    /// | No active trust anchor | `Red` (nothing to verify against) |
    /// | Active anchor set, sidecar present, verifies against a non-revoked anchor | `Blue` |
    /// | Sidecar present but verifies only against a revoked anchor | `SignatureInvalid` (revoked) |
    /// | Sidecar present, signature fails against every active anchor | `SignatureInvalid` |
    /// | Active anchor set, no sidecar | `Red` |
    fn determine_trust(&self, path: &Path, name: &str) -> PluginResult<TrustLevel> {
        if self.trust_anchors.is_empty() {
            return Ok(TrustLevel::Red);
        }

        let sig_path = {
            let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            path.with_file_name(format!("{file_name}.sig"))
        };
        if !sig_path.exists() {
            return Ok(TrustLevel::Red);
        }

        let bytes = std::fs::read(path).map_err(|e| {
            PluginError::LoadFailed(format!(
                "Cannot read '{}' for signature verification: {e}",
                path.display()
            ))
        })?;
        let sig_bytes = std::fs::read(&sig_path).map_err(|e| {
            PluginError::LoadFailed(format!(
                "Cannot read signature '{}': {e}",
                sig_path.display()
            ))
        })?;

        // Every conversion failure below is a security failure: a corrupt or
        // non-matching sidecar must reject the plugin, never silently demote it.
        let sig = Signature::from_slice(&sig_bytes).map_err(|_| PluginError::SignatureInvalid {
            plugin: name.to_string(),
            reason: "signature sidecar is not a valid Ed25519 signature".into(),
        })?;

        // Single pass over the active set. A revoked anchor is still checked
        // (so a leaked key's signature reports "revoked" rather than being
        // silently demoted), but it can never grant Blue.
        let mut any_revoked_match = false;
        for anchor in &self.trust_anchors {
            let vk =
                VerifyingKey::from_bytes(anchor).map_err(|_| PluginError::SignatureInvalid {
                    plugin: name.to_string(),
                    reason: "trust anchor is not a valid Ed25519 public key".into(),
                })?;
            let fp = fingerprint_key(&vk);
            let matches = vk.verify_strict(&bytes, &sig).is_ok();
            if self.revoked.contains(&fp) {
                if matches {
                    any_revoked_match = true;
                }
                continue;
            }
            if matches {
                return Ok(TrustLevel::Blue);
            }
        }

        if any_revoked_match {
            Err(PluginError::SignatureInvalid {
                plugin: name.to_string(),
                reason: "signature is from a revoked key".into(),
            })
        } else {
            Err(PluginError::SignatureInvalid {
                plugin: name.to_string(),
                reason: "signature does not verify against any active trust anchor".into(),
            })
        }
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
            // We pass NULL config for now; a later version will pass domain-specific config.
            unsafe { init_fn(std::ptr::null()) }
        } else {
            return Err(PluginError::LoadFailed(
                "Library handle not available".into(),
            ));
        };

        if init_result != 0 {
            crate::sys::diagnostics::warn(
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
                crate::sys::diagnostics::warn(
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn validate_plugin_name_accepts_valid_names() {
        for name in [
            "formatter-json",
            "filter-level",
            "sink-kafka",
            "acme.csv_formatter",
            "a_b_1",
            "x",
        ] {
            assert!(validate_plugin_name(name).is_ok(), "should accept {name:?}");
        }
    }

    #[test]
    fn validate_plugin_name_rejects_invalid_names() {
        assert!(validate_plugin_name("").is_err());
        assert!(validate_plugin_name(&"x".repeat(129)).is_err());
        assert!(validate_plugin_name("BadName").is_err()); // uppercase
        assert!(validate_plugin_name("has space").is_err());
        assert!(validate_plugin_name("slash/name").is_err());
        assert!(validate_plugin_name("tab\tname").is_err());
    }

    #[test]
    fn error_display_exposes_key_facts() {
        let abi = PluginError::IncompatibleAbi {
            plugin: "formatter-json".into(),
            core_abi: 0x000100,
            plugin_abi: 1,
        };
        let s = abi.to_string();
        assert!(
            s.contains("formatter-json") && s.contains("0x100"),
            "got: {s}"
        );

        let dup = PluginError::AlreadyLoaded("formatter-json".into()).to_string();
        assert!(dup.contains("already loaded"), "got: {dup}");

        let miss = PluginError::MissingSymbol("plugin_query".into()).to_string();
        assert!(miss.contains("plugin_query"), "got: {miss}");

        let notfound = PluginError::NotFound("nonexistent".into()).to_string();
        assert!(notfound.contains("nonexistent"), "got: {notfound}");
    }

    #[test]
    fn default_plugin_paths_include_local_and_system_dirs() {
        let paths = default_plugin_paths();
        assert!(
            paths.iter().any(|p| p == Path::new("./plugins")),
            "local ./plugins dir must be a search path: {paths:?}"
        );
        #[cfg(unix)]
        assert!(
            paths.iter().any(|p| p.ends_with("dologger/plugins")),
            "system plugin dir must be a search path: {paths:?}"
        );
        #[cfg(windows)]
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with("dologger\\plugins") || p.ends_with("dologger/plugins")),
            "system plugin dir must be a search path: {paths:?}"
        );
    }
}
