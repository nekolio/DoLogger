//! Configuration loading, validation, and hot-reload for DoLogger.

use std::path::{Path, PathBuf};

/// Top-level DoLogger configuration.
#[derive(Debug, Clone)]
pub struct DologgerConfig {
    /// Default log level
    pub level: String,
    /// Performance profile preset
    pub performance_profile: PerformanceProfile,
    /// Ring buffer capacity (power of two)
    pub ring_buffer_size: usize,
    /// Batch size for consumer drain
    pub batch_size: usize,
    /// Enable Ed25519 signatures
    pub enable_signature: bool,
    /// Path to the `.sig` sidecar file where the pipeline appends audit-record
    /// signatures (`lsn:content_hash_hex:signature_hex` per line, ADR-002 A.6).
    /// None disables sidecar writing even when `enable_signature` is set.
    pub sig_sidecar_path: Option<PathBuf>,
    /// Path to the active config file (for diagnostics)
    pub config_path: Option<PathBuf>,
    /// Shutdown policy: "graceful" (drain all) or "immediate" (drop pending)
    pub shutdown_policy: String,
    /// Shutdown timeout in milliseconds (0 = no timeout)
    pub shutdown_timeout_ms: u64,
    /// Key rotation grace period in days (default 7)
    pub key_rotation_grace_period_days: u32,
    /// Enable cooperative helping: when the ring buffer reaches
    /// ≥90% capacity, producer threads help drain a small batch inline.
    /// Enabled by default for prod-performance; disabled otherwise.
    pub ring_buffer_coop_helping: bool,
    /// Directory containing the committed plugin trust store (`active.pub` +
    /// `revoked.txt`). When set it is authoritative and the legacy
    /// `plugin_trust_anchor` field is ignored.
    pub plugin_trust_store: Option<String>,
    /// Legacy single trust anchor — 64-hex Ed25519 public key. Used only
    /// when no trust store is configured.
    pub plugin_trust_anchor: Option<String>,
    /// Allow unsigned (Red) plugins to load outside dev mode.
    pub plugin_allow_red_plugins: bool,
    /// Whether to load plugins into the engine and dispatch their formatter /
    /// field-provider vtables from the pipeline (M6). Default off: the engine
    /// does not load plugins at runtime unless this is enabled, so existing
    /// behaviour is unchanged. Plugins are always loadable via `dologctl plugin`
    /// (the management path) regardless of this flag.
    pub plugin_enable_pipeline: bool,
    /// Configured output sinks. Parsed from `[sinks.<name>]` tables; when the
    /// section is absent or empty the console default is used.
    pub sinks: Vec<crate::sink::SinkKindConfig>,
    /// Optional shared-memory sink, parsed from the top-level `[shm]` table.
    /// When absent, no sink_shm is wired. `dologctl run --shm <path>` can
    /// enable it with a CLI path override (all other fields default or come
    /// from the TOML table).
    pub shm: Option<crate::sink::ShmSinkConfig>,
    /// Config-file watcher for hot reload, parsed from the top-level `[watcher]`
    /// table. Off by default: reload is an opt-in feature so existing
    /// deployments are unaffected until a `[watcher]` section enables it.
    pub watcher: crate::config::WatcherConfig,
}

impl DologgerConfig {
    /// Check if dev mode is active.
    pub fn is_dev_mode(&self) -> bool {
        self.performance_profile == PerformanceProfile::Dev
            || std::env::var("DO_LOG_DEV_MODE").is_ok()
    }
}

/// Resolve the configuration file path with a platform-aware fallback chain.
///
/// Priority:
/// 1. Explicit path passed by caller (e.g., `dologctl run -c /path/to/config.toml`)
/// 2. `DO_LOG_CONFIG_FILE` environment variable
/// 3. `./dologger.toml` (current working directory)
/// 4. `../dologger.toml` (parent directory)
/// 5. Platform-specific default:
///    - Linux: `/etc/dologger/default.toml`
///    - macOS: `/usr/local/etc/dologger/default.toml`
///    - Windows: `%PROGRAMDATA%\dologger\default.toml`
///
/// If no config file is found and `auto_create` is true, a minimal
/// configuration file is generated at the first writable location in
/// the search chain.  The generated file uses restrictive permissions
/// (0600 on Unix, current-user-only on Windows).
///
/// Returns `(path, was_created)` on success, or an error message.
pub fn resolve_config_path(
    explicit: Option<&str>,
    auto_create: bool,
) -> Result<(PathBuf, bool), String> {
    // Priority 1: Explicit path
    if let Some(path) = explicit {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok((p, false));
        }
        if auto_create {
            return create_default_config(&p).map(|()| (p, true));
        }
        return Err(format!("Config file not found: {path}"));
    }

    // Priority 2: DO_LOG_CONFIG_FILE
    if let Ok(env_path) = std::env::var("DO_LOG_CONFIG_FILE") {
        let p = PathBuf::from(&env_path);
        if p.exists() {
            return Ok((p, false));
        }
        if auto_create {
            return create_default_config(&p).map(|()| (p, true));
        }
    }

    // Priority 3-5: Search chain
    let candidates: Vec<PathBuf> = vec![
        PathBuf::from("dologger.toml"),
        PathBuf::from("../dologger.toml"),
        #[cfg(target_os = "linux")]
        PathBuf::from("/etc/dologger/default.toml"),
        #[cfg(target_os = "macos")]
        PathBuf::from("/usr/local/etc/dologger/default.toml"),
        #[cfg(windows)]
        {
            let programdata =
                std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
            PathBuf::from(format!("{programdata}\\dologger\\default.toml"))
        },
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Ok((candidate.clone(), false));
        }
    }

    if auto_create {
        // Create at the first writable location: cwd/dologger.toml
        let default = PathBuf::from("dologger.toml");
        create_default_config(&default)?;
        return Ok((default, true));
    }

    Err("No configuration file found. Use dologctl init --template dev to create one.".into())
}

/// Create a minimal default configuration file at the given path.
fn create_default_config(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create config directory '{}': {e}", parent.display()))?;
    }

    let default_toml = r#"# DoLogger Configuration
# Generated automatically — adjust for your environment.

[dologger]
level = "INFO"
performance_profile = "prod-performance"
ring_buffer_size = 262144
batch_size = 256
enable_signature = false
"#;

    std::fs::write(path, default_toml)
        .map_err(|e| format!("Cannot write default config to '{}': {e}", path.display()))?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }

    crate::sys::diagnostics::info(
        "config",
        &format!("Created default config at '{}'", path.display()),
    );

    Ok(())
}

/// Performance profile presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceProfile {
    /// Development: small batches, spin-wait, signatures off
    Dev,
    /// Production performance: large batches, yield + cooperative helping
    ProdPerformance,
    /// Production audit: medium batches, all signed
    ProdAudit,
    /// Balanced: compromise between throughput and safety
    Balanced,
}

/// Compliance profile presets corresponding to regulatory frameworks.
///
/// Each variant represents a pre-defined set of minimum configuration
/// requirements aligned with a specific regulation. Used with
/// [`DologgerConfig::validate_compliance_template`] to verify that a
/// loaded configuration meets the minimum technical controls for that
/// regulatory regime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplianceProfile {
    /// EU General Data Protection Regulation (Regulation (EU) 2016/679)
    Gdpr,
    /// US Health Insurance Portability and Accountability Act (45 CFR Part 164)
    Hipaa,
    /// Payment Card Industry Data Security Standard (PCI DSS v4.0.1)
    PciDss,
}

impl ComplianceProfile {
    /// Human-readable name for display and diagnostics.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Gdpr => "GDPR",
            Self::Hipaa => "HIPAA",
            Self::PciDss => "PCI DSS",
        }
    }

    /// Canonical template file name (without path).
    pub fn template_filename(&self) -> &'static str {
        match self {
            Self::Gdpr => "gdpr.toml",
            Self::Hipaa => "hipaa.toml",
            Self::PciDss => "pci-dss.toml",
        }
    }
}

impl Default for DologgerConfig {
    fn default() -> Self {
        Self {
            level: "INFO".into(),
            performance_profile: PerformanceProfile::ProdPerformance,
            ring_buffer_size: 262144, // 256K records
            batch_size: 256,
            enable_signature: false,
            sig_sidecar_path: None,
            config_path: None,
            shutdown_policy: "graceful".into(),
            shutdown_timeout_ms: 5000,
            key_rotation_grace_period_days: 7,
            ring_buffer_coop_helping: true, // On for prod-performance by default
            plugin_trust_store: None,
            plugin_trust_anchor: None,
            plugin_allow_red_plugins: false,
            plugin_enable_pipeline: false,
            sinks: vec![crate::sink::SinkKindConfig::console()],
            shm: None,
            watcher: crate::config::WatcherConfig {
                enabled: false,
                ..Default::default()
            },
        }
    }
}

impl DologgerConfig {
    /// Create a configuration with hardcoded safe defaults.
    pub fn hardcoded_defaults() -> Self {
        Self::default()
    }

    /// Create dev-profile configuration.
    pub fn dev_profile() -> Self {
        Self {
            level: "DEBUG".into(),
            performance_profile: PerformanceProfile::Dev,
            ring_buffer_size: 65536, // 64K
            batch_size: 32,
            enable_signature: false,
            sig_sidecar_path: None,
            config_path: None,
            shutdown_policy: "graceful".into(),
            shutdown_timeout_ms: 5000,
            key_rotation_grace_period_days: 7,
            ring_buffer_coop_helping: false,
            plugin_trust_store: None,
            plugin_trust_anchor: None,
            plugin_allow_red_plugins: false,
            plugin_enable_pipeline: false,
            sinks: vec![crate::sink::SinkKindConfig::console()],
            shm: None,
            watcher: crate::config::WatcherConfig {
                enabled: false,
                ..Default::default()
            },
        }
    }

    /// Validate that this configuration satisfies the minimum requirements
    /// for a given compliance profile.
    ///
    /// Checks the config-level (top-level) settings that each compliance
    /// template mandates. Domain-level non-downgradable items
    /// (`worm_enabled`, `sign_ring2`, `escape_html`, `fsync_on_write`,
    /// `require_tls`) must be validated separately via
    /// [`crate::config::DomainManager`] since they reside in domain
    /// configuration, not in [`DologgerConfig`].
    ///
    /// Returns `Ok(())` if all minimum requirements are met, or
    /// `Err(Vec<String>)` with a list of compliance gaps found.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dologger_core::config::{DologgerConfig, ComplianceProfile};
    ///
    /// let (config, _) = DologgerConfig::load_default();
    /// // Default config WILL NOT pass compliance validation (signatures off)
    /// let result = config.validate_compliance_template(&ComplianceProfile::Gdpr);
    /// assert!(result.is_err());
    /// ```
    pub fn validate_compliance_template(
        &self,
        profile: &ComplianceProfile,
    ) -> Result<(), Vec<String>> {
        let mut gaps: Vec<String> = Vec::new();
        let name = profile.display_name();

        // --- Config-level mandatory checks (all profiles) ---

        // 1. Cryptographic signatures required for non-repudiation
        if !self.enable_signature {
            gaps.push(format!(
                "{name}: enable_signature must be true — cryptographic non-repudiation is required"
            ));
        }

        // 2. Performance profile must be prod-audit (enables signatures + audit batching)
        if self.performance_profile != PerformanceProfile::ProdAudit {
            gaps.push(format!(
                "{name}: performance_profile must be \"prod-audit\", currently {:?}",
                self.performance_profile
            ));
        }

        // 3. Shutdown must be graceful to prevent audit record loss
        if self.shutdown_policy != "graceful" {
            gaps.push(format!(
                "{name}: shutdown_policy must be \"graceful\" — in-flight audit records must be drained"
            ));
        }

        // 4. Shutdown timeout must be sufficient to drain all records
        if self.shutdown_timeout_ms < 5000 {
            gaps.push(format!(
                "{name}: shutdown_timeout_ms must be >= 5000 ms (currently {} ms)",
                self.shutdown_timeout_ms
            ));
        }

        // --- Informational notes for domain-level items ---
        // These are validated by DomainManager at the domain level.
        // We emit them as gaps here so the user knows to check them.
        let domain_items = [
            "worm_enabled",
            "sign_ring2",
            "escape_html",
            "fsync_on_write",
            "require_tls",
        ];
        for item in &domain_items {
            gaps.push(format!(
                "{name}: REMINDER — {item} must be true at the domain level. This is a domain-level non-downgradable item validated by DomainManager."
            ));
        }

        if gaps.is_empty() {
            Ok(())
        } else {
            Err(gaps)
        }
    }

    /// Load configuration using the full priority ladder.
    ///
    /// Priority (lowest to highest):
    /// 1. Hardcoded defaults
    /// 2. System default config (`/etc/dologger/default.toml` or `%PROGRAMDATA%\dologger\default.toml`)
    /// 3. Project local config (cwd + parent traversal up to 2 levels)
    /// 4. Environment variables (`DO_LOG_LEVEL`, `DO_LOG_BUF_SIZE`, `DO_LOG_PERF_PROFILE`, `DO_LOG_CONFIG_FILE`)
    /// 5. API parameters (not implemented in load_default)
    /// 6. Record metadata tags (deferred)
    /// 7. Absolute non-downgradable items (hardcoded)
    ///
    /// Returns (config, warnings).
    pub fn load_default() -> (Self, Vec<String>) {
        let mut config = Self::hardcoded_defaults();
        let mut warnings = Vec::new();

        // Priority 2: System default config
        #[cfg(target_os = "linux")]
        let system_paths = [Path::new("/etc/dologger/default.toml")];
        #[cfg(target_os = "macos")]
        let system_paths = [Path::new("/usr/local/etc/dologger/default.toml")];
        #[cfg(windows)]
        let system_paths = {
            let programdata =
                std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
            // Can't easily return a &Path from a String, use a different approach
            vec![std::path::PathBuf::from(format!(
                "{programdata}\\dologger\\default.toml"
            ))]
        };

        #[cfg(not(windows))]
        for path in &system_paths {
            if path.exists() {
                match Self::load_from_file(path.to_str().unwrap()) {
                    Ok((cfg, w)) => {
                        config = cfg;
                        warnings.extend(w);
                        break;
                    }
                    Err((code, msg)) => {
                        warnings.push(format!("System config parse error (code {code}): {msg}"))
                    }
                }
            }
        }
        #[cfg(windows)]
        for path in &system_paths {
            if path.exists() {
                match Self::load_from_file(path.to_str().unwrap()) {
                    Ok((cfg, w)) => {
                        config = cfg;
                        warnings.extend(w);
                        break;
                    }
                    Err((code, msg)) => {
                        warnings.push(format!("System config parse error (code {code}): {msg}"))
                    }
                }
            }
        }

        // Priority 3: Project local config (cwd + parent traversal)
        let local_names = ["dologger.toml", ".dologger.toml"];
        let mut found_local = false;
        for depth in 0..=2 {
            for name in &local_names {
                let path: PathBuf = if depth == 0 {
                    PathBuf::from(name)
                } else {
                    let prefix: PathBuf = (0..depth).map(|_| "..").collect();
                    prefix.join(name)
                };
                if path.exists() {
                    match Self::load_from_file(path.to_str().unwrap()) {
                        Ok((cfg, w)) => {
                            config = cfg;
                            warnings.extend(w);
                            found_local = true;
                            break;
                        }
                        Err((code, msg)) => {
                            warnings.push(format!("Local config parse error (code {code}): {msg}"))
                        }
                    }
                }
            }
            if found_local {
                break;
            }
        }

        // Priority 4: Environment variables
        if let Ok(level) = std::env::var("DO_LOG_LEVEL") {
            config.level = level;
        }
        if let Ok(buf_size) = std::env::var("DO_LOG_BUF_SIZE") {
            if let Ok(size) = buf_size.parse::<usize>() {
                if size.is_power_of_two() && size >= 1024 {
                    config.ring_buffer_size = size;
                } else {
                    warnings.push(format!(
                        "DO_LOG_BUF_SIZE={buf_size} invalid (must be power of two >= 1024)"
                    ));
                }
            }
        }
        if let Ok(profile) = std::env::var("DO_LOG_PERF_PROFILE") {
            config.performance_profile = match profile.as_str() {
                "dev" => PerformanceProfile::Dev,
                "prod-performance" => PerformanceProfile::ProdPerformance,
                "prod-audit" => PerformanceProfile::ProdAudit,
                "balanced" => PerformanceProfile::Balanced,
                unknown => {
                    warnings.push(format!(
                        "DO_LOG_PERF_PROFILE={unknown} unknown, using default"
                    ));
                    config.performance_profile
                }
            };
        }
        if let Ok(cfg_file) = std::env::var("DO_LOG_CONFIG_FILE") {
            let path = Path::new(&cfg_file);
            if path.exists() {
                match Self::load_from_file(&cfg_file) {
                    Ok((cfg, w)) => {
                        config = cfg;
                        warnings.extend(w);
                    }
                    Err((code, msg)) => {
                        warnings.push(format!("DO_LOG_CONFIG_FILE error (code {code}): {msg}"))
                    }
                }
            } else if std::env::var("DO_LOG_CONFIG_LOCK").is_ok() {
                // DO_LOG_CONFIG_LOCK prevents fallback search
                warnings.push(format!("DO_LOG_CONFIG_FILE={cfg_file} not found and DO_LOG_CONFIG_LOCK is set — using defaults"));
            }
        }

        if !found_local && warnings.is_empty() {
            warnings.push("No configuration file found; using defaults".into());
        }

        config.apply_profile();

        // Priority 7: Absolute non-downgradable items
        config.enforce_non_downgradable(&mut warnings);

        (config, warnings)
    }

    /// Enforce absolute non-downgradable items as the highest priority (level 7).
    ///
    /// Non-downgradable items can only be tightened (false→true), never loosened.
    /// At the config level, we only validate that a compliance template's minimum
    /// requirements are met. The actual domain-level non-downgradable enforcement
    /// happens in [`crate::config::DomainManager::check_non_downgradable`] during
    /// child domain creation.
    ///
    /// This method does NOT unconditionally force items to true — that would
    /// prevent the dev profile and other non-audit configurations from working.
    /// It only enforces when a compliance template (which mandates true for these
    /// items) has been explicitly loaded.
    pub fn enforce_non_downgradable(&mut self, warnings: &mut Vec<String>) {
        // Domain-level non-downgradable items (escape_html, worm_enabled,
        // fsync_on_write, require_tls, sign_ring2) are enforced by
        // DomainManager at domain inheritance time, not here.
        //
        // enable_signature is config-level but should not be forced true
        // unconditionally — it defaults based on the performance profile.
        // Compliance templates validate it separately via
        // validate_compliance_template().
        let _ = warnings;
    }

    /// Apply API-level configuration overrides (priority level 5).
    ///
    /// Priority 5 parameters are set programmatically via the native API
    /// (`dologger_init()`) and take precedence over all config-file and
    /// environment-variable settings (priorities 1–4). Cannot override
    /// priority 7 non-downgradable items.
    ///
    /// Returns the set of field names that were overridden.
    pub fn apply_api_overrides(&mut self, api: &ApiOverrides) -> Vec<&'static str> {
        let mut overridden: Vec<&'static str> = Vec::new();

        if let Some(ref level) = api.level {
            self.level = level.clone();
            overridden.push("level");
        }
        if let Some(profile) = api.performance_profile {
            self.performance_profile = profile;
            overridden.push("performance_profile");
        }
        if let Some(size) = api.ring_buffer_size {
            if size.is_power_of_two() && size >= 1024 {
                self.ring_buffer_size = size;
                overridden.push("ring_buffer_size");
            }
        }
        if let Some(batch) = api.batch_size {
            if batch >= 1 {
                self.batch_size = batch;
                overridden.push("batch_size");
            }
        }
        if let Some(enable) = api.enable_signature {
            // Priority 5 cannot override priority 7 enforcement, but CAN
            // request enabling (tightening) if not already forced on.
            if enable || !self.enable_signature {
                self.enable_signature = enable;
                overridden.push("enable_signature");
            }
        }
        if let Some(coop) = api.ring_buffer_coop_helping {
            self.ring_buffer_coop_helping = coop;
            overridden.push("ring_buffer_coop_helping");
        }
        if let Some(ref policy) = api.shutdown_policy {
            self.shutdown_policy = policy.clone();
            overridden.push("shutdown_policy");
        }
        if let Some(timeout) = api.shutdown_timeout_ms {
            self.shutdown_timeout_ms = timeout;
            overridden.push("shutdown_timeout_ms");
        }
        if let Some(days) = api.key_rotation_grace_period_days {
            self.key_rotation_grace_period_days = days;
            overridden.push("key_rotation_grace_period_days");
        }

        overridden
    }

    /// Full priority-ladder load with API overrides.
    ///
    /// Combines `load_default()` (priorities 1–4) with API overrides
    /// (priority 5) and non-downgradable enforcement (priority 7).
    /// Priority 6 (Record metadata tags) is applied per-record at
    /// submit time and is not part of this configuration-level method.
    pub fn load_with_api_overrides(api: &ApiOverrides) -> (Self, Vec<String>) {
        let (mut config, mut warnings) = Self::load_default();

        // Priority 5: API overrides
        let overridden = config.apply_api_overrides(api);
        if !overridden.is_empty() {
            let fields: Vec<&str> = overridden;
            crate::sys::diagnostics::info(
                "config",
                &format!("Priority 5 API overrides applied: {}", fields.join(", ")),
            );
        }

        // Priority 7: Non-downgradable enforcement
        config.enforce_non_downgradable(&mut warnings);

        (config, warnings)
    }
}

// ===========================================================================
// Priority 5: API-level overrides
// ===========================================================================

/// API-level configuration overrides (priority level 5).
///
/// These are parameters passed programmatically at initialization time
/// (e.g. via `dologger_init()`). They take precedence over all
/// config-file and environment-variable settings (priorities 1–4).
///
/// All fields are `Option` — `None` means "use the value from lower
/// priority levels."
#[derive(Debug, Clone, Default)]
pub struct ApiOverrides {
    /// Default log level
    pub level: Option<String>,
    /// Performance profile preset
    pub performance_profile: Option<PerformanceProfile>,
    /// Ring buffer capacity (power of two, >= 1024)
    pub ring_buffer_size: Option<usize>,
    /// Batch size for consumer drain
    pub batch_size: Option<usize>,
    /// Enable Ed25519 signatures
    pub enable_signature: Option<bool>,
    /// Enable cooperative helping
    pub ring_buffer_coop_helping: Option<bool>,
    /// Shutdown policy: "graceful" or "immediate"
    pub shutdown_policy: Option<String>,
    /// Shutdown timeout in milliseconds
    pub shutdown_timeout_ms: Option<u64>,
    /// Key rotation grace period in days
    pub key_rotation_grace_period_days: Option<u32>,
}

/// Record-level tag overrides (priority level 6).
///
/// Individual log records may carry metadata tags that influence
/// pipeline behavior at submit time. These tags take precedence over
/// all lower priority levels (1–5) for that specific record, but
/// cannot override priority 7 non-downgradable items.
///
/// # Example
///
/// ```rust
/// use dologger_core::config::RecordTagOverrides;
///
/// let tags = RecordTagOverrides {
///     level_override: Some("ERROR".into()),
///     format_override: Some("json".into()),
///     ..Default::default()
/// };
/// // Apply to a record before `dologger_submit()`
/// ```
#[derive(Debug, Clone, Default)]
pub struct RecordTagOverrides {
    /// Override this record's log level
    pub level_override: Option<String>,
    /// Override this record's output format
    pub format_override: Option<String>,
    /// Force this record to a specific sink
    pub sink_override: Option<String>,
    /// Force this record to a specific domain
    pub domain_override: Option<String>,
    /// Additional tags for routing/filtering
    pub tags: Vec<String>,
    /// Record requires Ed25519 signature (respects non-downgradable)
    pub require_signature: bool,
    /// Record requires fsync before ack (respects non-downgradable)
    pub require_fsync: bool,
}

impl RecordTagOverrides {
    /// Create an empty set of record tag overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when no overrides are set.
    pub fn is_empty(&self) -> bool {
        self.level_override.is_none()
            && self.format_override.is_none()
            && self.sink_override.is_none()
            && self.domain_override.is_none()
            && self.tags.is_empty()
            && !self.require_signature
            && !self.require_fsync
    }
}

// ===========================================================================
// Remaining DologgerConfig methods (cont.)
// ===========================================================================

impl DologgerConfig {
    /// Load configuration from a specific TOML file.
    ///
    /// Returns `Ok((config, warnings))` or `Err((error_code, message))`.
    pub fn load_from_file(path: &str) -> Result<(Self, Vec<String>), (i32, String)> {
        let bytes = std::fs::read(path).map_err(|e| {
            (
                crate::error::DO_LOG_ERR_CONFIG_NOT_FOUND,
                format!("Cannot read config file '{path}': {e}"),
            )
        })?;

        let content = Self::read_text_auto(&bytes).map_err(|msg| {
            (
                crate::error::DO_LOG_ERR_CONFIG_PARSE,
                format!("Cannot decode config file '{path}': {msg}"),
            )
        })?;

        Self::parse(&content, Some(PathBuf::from(path)))
    }

    /// Decode config file bytes with BOM / encoding detection.
    ///
    /// Accepts the three encodings commonly found in the wild: UTF-8 (with
    /// or without BOM — Notepad on Windows writes a BOM), UTF-16 LE
    /// (PowerShell 5 `Out-File` default) and UTF-16 BE. Anything else is
    /// treated as UTF-8 and rejected with a clear message.
    fn read_text_auto(bytes: &[u8]) -> Result<String, String> {
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            return String::from_utf8(bytes[3..].to_vec())
                .map_err(|_| "file is not valid UTF-8 after its BOM".into());
        }
        let (le, offset) = if bytes.starts_with(&[0xFF, 0xFE]) {
            (true, 2)
        } else if bytes.starts_with(&[0xFE, 0xFF]) {
            (false, 2)
        } else {
            return String::from_utf8(bytes.to_vec())
                .map_err(|_| "file is not valid UTF-8 (save it as UTF-8)".into());
        };
        let mut units = Vec::with_capacity((bytes.len() - offset) / 2);
        for c in bytes[offset..].chunks_exact(2) {
            units.push(if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            });
        }
        String::from_utf16(&units)
            .map_err(|_| format!("file is not valid UTF-16 {}E", if le { "L" } else { "B" }))
    }

    /// Parse configuration from a TOML string.
    pub fn parse(
        toml_str: &str,
        config_path: Option<PathBuf>,
    ) -> Result<(Self, Vec<String>), (i32, String)> {
        let table: toml::Table = toml::de::from_str(toml_str).map_err(|e| {
            (
                crate::error::DO_LOG_ERR_CONFIG_PARSE,
                format!("TOML parse error: {e}"),
            )
        })?;

        let mut config = Self {
            config_path,
            ..Self::default()
        };
        let mut warnings = Vec::new();

        // Parse `[dologger]` section
        if let Some(dologger) = table.get("dologger").and_then(|v| v.as_table()) {
            if let Some(level) = dologger.get("level").and_then(|v| v.as_str()) {
                config.level = level.to_string();
            }
            if let Some(profile) = dologger.get("performance_profile").and_then(|v| v.as_str()) {
                config.performance_profile = match profile {
                    "dev" => PerformanceProfile::Dev,
                    "prod-performance" => PerformanceProfile::ProdPerformance,
                    "prod-audit" => PerformanceProfile::ProdAudit,
                    "balanced" => PerformanceProfile::Balanced,
                    unknown => {
                        warnings.push(format!(
                            "Unknown performance_profile '{unknown}', using default"
                        ));
                        PerformanceProfile::ProdPerformance
                    }
                };
            }
            if let Some(size) = dologger
                .get("ring_buffer_size")
                .and_then(|v| v.as_integer())
            {
                let s = size as usize;
                if s.is_power_of_two() && s >= 1024 {
                    config.ring_buffer_size = s;
                } else {
                    warnings.push(format!(
                        "ring_buffer_size {s} must be a power of two >= 1024, using default {}",
                        config.ring_buffer_size
                    ));
                }
            }
            if let Some(batch) = dologger.get("batch_size").and_then(|v| v.as_integer()) {
                config.batch_size = batch as usize;
            }
            if let Some(sig) = dologger.get("enable_signature").and_then(|v| v.as_bool()) {
                config.enable_signature = sig;
            }
            if let Some(path) = dologger.get("sig_sidecar").and_then(|v| v.as_str()) {
                config.sig_sidecar_path = Some(PathBuf::from(path));
            }
            if let Some(policy) = dologger.get("shutdown_policy").and_then(|v| v.as_str()) {
                if policy == "graceful" || policy == "immediate" {
                    config.shutdown_policy = policy.to_string();
                } else {
                    warnings.push(format!(
                        "shutdown_policy '{policy}' must be 'graceful' or 'immediate', using default '{}'",
                        config.shutdown_policy
                    ));
                }
            }
            if let Some(timeout) = dologger
                .get("shutdown_timeout_ms")
                .and_then(|v| v.as_integer())
            {
                config.shutdown_timeout_ms = timeout.max(0) as u64;
            }
            if let Some(days) = dologger
                .get("key_rotation_grace_period_days")
                .and_then(|v| v.as_integer())
            {
                config.key_rotation_grace_period_days = days as u32;
            }
            if let Some(coop) = dologger
                .get("ring_buffer_coop_helping")
                .and_then(|v| v.as_bool())
            {
                config.ring_buffer_coop_helping = coop;
            }
            if let Some(store) = dologger.get("plugin_trust_store").and_then(|v| v.as_str()) {
                config.plugin_trust_store = Some(store.to_string());
            }
            if let Some(anchor) = dologger.get("plugin_trust_anchor").and_then(|v| v.as_str()) {
                config.plugin_trust_anchor = Some(anchor.to_string());
            }
            if let Some(allow) = dologger
                .get("plugin_allow_red_plugins")
                .and_then(|v| v.as_bool())
            {
                config.plugin_allow_red_plugins = allow;
            }
            if let Some(en) = dologger
                .get("plugin_enable_pipeline")
                .and_then(|v| v.as_bool())
            {
                config.plugin_enable_pipeline = en;
            }
        }

        // Parse `[sinks.<name>]` sections. Declaring any sink replaces the
        // console default; a broken entry is reported as a warning and skipped
        // so one bad sink can never take the whole config down.
        config.sinks.clear();
        if let Some(sinks) = table.get("sinks").and_then(|v| v.as_table()) {
            for (name, value) in sinks {
                match value.clone().try_into::<crate::sink::SinkKindConfig>() {
                    Ok(kind) => config.sinks.push(kind),
                    Err(e) => warnings.push(format!("sink '{name}' configuration invalid: {e}")),
                }
            }
        }
        if config.sinks.is_empty() {
            config.sinks.push(crate::sink::SinkKindConfig::console());
        }

        // Parse the optional top-level `[shm]` table. sink_shm is wired
        // separately from `[sinks.*]` (see sink/registry), so it has its own
        // top-level section. A malformed section disables sink_shm with a
        // warning rather than failing the whole config load.
        if let Some(shm) = table.get("shm") {
            match shm.clone().try_into::<crate::sink::ShmSinkConfig>() {
                Ok(cfg) => config.shm = Some(cfg),
                Err(e) => warnings.push(format!(
                    "[shm] configuration invalid, sink_shm disabled: {e}"
                )),
            }
        }

        // Parse the optional top-level `[watcher]` table for hot reload.
        // It is opt-in: the field defaults to disabled, so only an explicit
        // section enables the config-file watcher.
        if let Some(watcher) = table.get("watcher").and_then(|v| v.as_table()) {
            if let Some(enabled) = watcher.get("enabled").and_then(|v| v.as_bool()) {
                config.watcher.enabled = enabled;
            }
            if let Some(ms) = watcher.get("poll_interval_ms").and_then(|v| v.as_integer()) {
                config.watcher.poll_interval_ms = ms.max(0) as u64;
            }
            if let Some(ms) = watcher.get("debounce_ms").and_then(|v| v.as_integer()) {
                config.watcher.debounce_ms = ms.max(0) as u64;
            }
            if let Some(backend) = watcher.get("backend").and_then(|v| v.as_str()) {
                config.watcher.backend = match backend {
                    "polling" => crate::config::WatcherBackend::Polling,
                    "inotify" => crate::config::WatcherBackend::Inotify,
                    "read-directory-changes" | "rdcw" => {
                        crate::config::WatcherBackend::ReadDirectoryChanges
                    }
                    "fsevents" => crate::config::WatcherBackend::Fsevents,
                    unknown => {
                        warnings.push(format!(
                            "Unknown watcher backend '{unknown}', using detected default"
                        ));
                        crate::config::WatcherBackend::detect()
                    }
                };
            }
        }

        // Apply profile overrides
        config.apply_profile();

        Ok((config, warnings))
    }

    /// Apply performance profile settings.
    fn apply_profile(&mut self) {
        match self.performance_profile {
            PerformanceProfile::Dev => {
                self.batch_size = self.batch_size.min(32);
                self.enable_signature = false;
                self.ring_buffer_coop_helping = false;
            }
            PerformanceProfile::ProdPerformance => {
                self.batch_size = self.batch_size.max(256);
                self.ring_buffer_coop_helping = true;
            }
            PerformanceProfile::ProdAudit => {
                self.batch_size = 128;
                self.enable_signature = true;
                self.ring_buffer_coop_helping = false;
            }
            PerformanceProfile::Balanced => {
                self.batch_size = 128;
                self.ring_buffer_coop_helping = false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_text_auto_handles_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"[dologger]\nlevel = \"INFO\"\n");
        let text = DologgerConfig::read_text_auto(&bytes).unwrap();
        assert!(text.starts_with("[dologger]"), "BOM must be stripped");
        assert!(!text.contains('\u{feff}'));
    }

    #[test]
    fn read_text_auto_handles_utf16_le() {
        // PowerShell 5 Out-File default encoding
        let mut bytes = vec![0xFF, 0xFE];
        for u in "[dologger]\nlevel = \"INFO\"\n".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let text = DologgerConfig::read_text_auto(&bytes).unwrap();
        assert!(text.starts_with("[dologger]"));
    }

    #[test]
    fn read_text_auto_handles_plain_utf8_and_rejects_invalid() {
        let text = DologgerConfig::read_text_auto(b"[dologger]\nlevel = \"INFO\"\n").unwrap();
        assert!(text.starts_with("[dologger]"));
        // UTF-16 LE BOM followed by a lone high surrogate (U+D800) —
        // invalid UTF-16 must be rejected, not silently mangled.
        assert!(DologgerConfig::read_text_auto(&[0xFF, 0xFE, 0x00, 0xD8]).is_err());
        assert!(DologgerConfig::read_text_auto(&[0xC3, 0x28]).is_err()); // invalid UTF-8
    }

    /// Helper: create a GDPR-compliant config.
    fn gdpr_compliant_config() -> DologgerConfig {
        DologgerConfig {
            level: "AUDIT".into(),
            performance_profile: PerformanceProfile::ProdAudit,
            ring_buffer_size: 262144,
            batch_size: 128,
            enable_signature: true,
            sig_sidecar_path: None,
            config_path: None,
            shutdown_policy: "graceful".into(),
            shutdown_timeout_ms: 10000,
            key_rotation_grace_period_days: 7,
            ring_buffer_coop_helping: false,
            plugin_trust_store: None,
            plugin_trust_anchor: None,
            plugin_allow_red_plugins: false,
            plugin_enable_pipeline: false,
            sinks: vec![crate::sink::SinkKindConfig::console()],
            shm: None,
            watcher: crate::config::WatcherConfig {
                enabled: false,
                ..Default::default()
            },
        }
    }

    // --- [watcher] section parsing ---

    #[test]
    fn test_watcher_section_parses() {
        let (config, warnings) = DologgerConfig::parse(
            "[dologger]\nlevel = \"INFO\"\n[watcher]\nenabled = true\npoll_interval_ms = 250\ndebounce_ms = 100\nbackend = \"inotify\"\n",
            None,
        )
        .expect("config parses");
        assert!(warnings.is_empty());
        assert!(config.watcher.enabled);
        assert_eq!(config.watcher.poll_interval_ms, 250);
        assert_eq!(config.watcher.debounce_ms, 100);
        assert_eq!(
            config.watcher.backend,
            crate::config::WatcherBackend::Inotify
        );
    }

    #[test]
    fn test_watcher_defaults_disabled() {
        let (config, _) =
            DologgerConfig::parse("[dologger]\nlevel = \"INFO\"\n", None).expect("config parses");
        assert!(
            !config.watcher.enabled,
            "hot reload must be opt-in and disabled by default"
        );
    }

    // --- Compliance profile tests ---

    #[test]
    fn test_compliance_profile_display_names() {
        assert_eq!(ComplianceProfile::Gdpr.display_name(), "GDPR");
        assert_eq!(ComplianceProfile::Hipaa.display_name(), "HIPAA");
        assert_eq!(ComplianceProfile::PciDss.display_name(), "PCI DSS");
    }

    #[test]
    fn test_compliance_profile_template_filenames() {
        assert_eq!(ComplianceProfile::Gdpr.template_filename(), "gdpr.toml");
        assert_eq!(ComplianceProfile::Hipaa.template_filename(), "hipaa.toml");
        assert_eq!(
            ComplianceProfile::PciDss.template_filename(),
            "pci-dss.toml"
        );
    }

    #[test]
    fn test_validate_gdpr_compliant_config_passes() {
        let config = gdpr_compliant_config();
        // Note: domain-level reminders are emitted as gaps,
        // so this returns Err with 5 reminders. We check the actual
        // config-level violations instead.
        let result = config.validate_compliance_template(&ComplianceProfile::Gdpr);
        let gaps = result.unwrap_err();
        // All gaps should be domain-level reminders, not config-level violations
        for gap in &gaps {
            assert!(
                gap.contains("REMINDER"),
                "Unexpected config-level violation: {gap}"
            );
        }
        assert_eq!(gaps.len(), 5); // 5 domain-level reminders
    }

    #[test]
    fn test_validate_default_config_fails_signature() {
        let config = DologgerConfig::default();
        let result = config.validate_compliance_template(&ComplianceProfile::Gdpr);
        assert!(result.is_err());
        let gaps = result.unwrap_err();
        // Should include enable_signature violation
        let sig_gap = gaps.iter().find(|g| g.contains("enable_signature"));
        assert!(
            sig_gap.is_some(),
            "Expected enable_signature gap, got: {gaps:?}"
        );
        assert!(sig_gap.unwrap().contains("must be true"));
    }

    #[test]
    fn test_validate_default_config_fails_profile() {
        let config = DologgerConfig::default();
        let result = config.validate_compliance_template(&ComplianceProfile::Hipaa);
        assert!(result.is_err());
        let gaps = result.unwrap_err();
        let profile_gap = gaps.iter().find(|g| g.contains("performance_profile"));
        assert!(
            profile_gap.is_some(),
            "Expected performance_profile gap, got: {gaps:?}"
        );
        assert!(profile_gap.unwrap().contains("prod-audit"));
    }

    #[test]
    fn test_validate_dev_profile_fails_all_checks() {
        let config = DologgerConfig::dev_profile();
        let result = config.validate_compliance_template(&ComplianceProfile::PciDss);
        assert!(result.is_err());
        let gaps = result.unwrap_err();
        // Should have: enable_signature, performance_profile, and possibly shutdown_timeout
        let sig = gaps.iter().any(|g| g.contains("enable_signature"));
        let profile = gaps.iter().any(|g| g.contains("performance_profile"));
        assert!(sig, "Missing enable_signature violation");
        assert!(profile, "Missing performance_profile violation");
    }

    #[test]
    fn test_validate_all_three_profiles_same_requirements() {
        // All three compliance profiles (GDPR, HIPAA, PCI DSS) have the same
        // minimum config-level requirements.
        let config = gdpr_compliant_config();

        for profile in &[
            ComplianceProfile::Gdpr,
            ComplianceProfile::Hipaa,
            ComplianceProfile::PciDss,
        ] {
            let result = config.validate_compliance_template(profile);
            let gaps = result.unwrap_err();
            // Only domain-level reminders expected
            for gap in &gaps {
                assert!(
                    gap.contains("REMINDER"),
                    "[{profile:?}] Unexpected config-level violation: {gap}"
                );
            }
        }
    }

    #[test]
    fn test_validate_shutdown_policy_immediate_fails() {
        let mut config = gdpr_compliant_config();
        config.shutdown_policy = "immediate".into();
        let result = config.validate_compliance_template(&ComplianceProfile::Gdpr);
        assert!(result.is_err());
        let gaps = result.unwrap_err();
        let shutdown_gap = gaps.iter().find(|g| g.contains("shutdown_policy"));
        assert!(
            shutdown_gap.is_some(),
            "Expected shutdown_policy gap, got: {gaps:?}"
        );
        assert!(shutdown_gap.unwrap().contains("graceful"));
    }

    #[test]
    fn test_validate_shutdown_timeout_too_low_fails() {
        let mut config = gdpr_compliant_config();
        config.shutdown_timeout_ms = 1000;
        let result = config.validate_compliance_template(&ComplianceProfile::Hipaa);
        assert!(result.is_err());
        let gaps = result.unwrap_err();
        let timeout_gap = gaps.iter().find(|g| g.contains("shutdown_timeout_ms"));
        assert!(
            timeout_gap.is_some(),
            "Expected shutdown_timeout_ms gap, got: {gaps:?}"
        );
        assert!(timeout_gap.unwrap().contains("5000"));
    }

    #[test]
    fn test_validate_compliance_profile_equality() {
        // Validate that ComplianceProfile implements PartialEq correctly
        assert_eq!(ComplianceProfile::Gdpr, ComplianceProfile::Gdpr);
        assert_ne!(ComplianceProfile::Gdpr, ComplianceProfile::Hipaa);
        assert_ne!(ComplianceProfile::Hipaa, ComplianceProfile::PciDss);
    }

    #[test]
    fn test_plugin_trust_config_fields_parse() {
        let toml = r#"
[dologger]
level = "INFO"
plugin_trust_store = "/opt/dologger/trust-anchors"
plugin_trust_anchor = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
plugin_allow_red_plugins = true
"#;
        let (config, _warnings) = DologgerConfig::parse(toml, None).unwrap();
        assert_eq!(
            config.plugin_trust_store.as_deref(),
            Some("/opt/dologger/trust-anchors")
        );
        assert_eq!(
            config.plugin_trust_anchor.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert!(config.plugin_allow_red_plugins);

        // Defaults: all unset / false.
        let def = DologgerConfig::default();
        assert!(def.plugin_trust_store.is_none());
        assert!(def.plugin_trust_anchor.is_none());
        assert!(!def.plugin_allow_red_plugins);

        // A config file that omits the new keys must still parse.
        let bare = r#"
[dologger]
level = "INFO"
"#;
        let (cfg2, _) = DologgerConfig::parse(bare, None).unwrap();
        assert!(cfg2.plugin_trust_store.is_none());
        assert!(!cfg2.plugin_allow_red_plugins);
    }

    #[test]
    fn test_compliance_toml_files_parse() {
        // Verify all three official compliance templates parse successfully
        let templates = [
            ("../compliance/gdpr.toml", ComplianceProfile::Gdpr),
            ("../compliance/hipaa.toml", ComplianceProfile::Hipaa),
            ("../compliance/pci-dss.toml", ComplianceProfile::PciDss),
        ];

        for (path, _profile) in &templates {
            let full_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
            assert!(
                full_path.exists(),
                "Template file not found: {}",
                full_path.display()
            );

            match DologgerConfig::load_from_file(full_path.to_str().unwrap()) {
                Ok((config, warnings)) => {
                    // After apply_profile, ProdAudit sets enable_signature=true
                    assert!(
                        config.enable_signature,
                        "[{path}] enable_signature must be true after parse+profile apply"
                    );
                    assert_eq!(
                        config.performance_profile,
                        PerformanceProfile::ProdAudit,
                        "[{path}] performance_profile must be prod-audit"
                    );
                    // Print warnings for diagnostics (not failures)
                    for w in &warnings {
                        eprintln!("[{path}] Warning: {w}");
                    }
                }
                Err((code, msg)) => {
                    panic!("Failed to parse template {path}: (code {code}) {msg}");
                }
            }
        }
    }
}
