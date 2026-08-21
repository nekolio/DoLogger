//! Error codes and domain event structures for DoLogger.
//!
//! All error codes are negative `i32` values. `0` (`DO_LOG_OK`) means success.
//!
//! # Coding scheme (v2 — supersedes the v0.1.x allocation)
//!
//! The error code space is a signed negative 16-bit-magnitude nibble scheme:
//! each category owns one hex byte (`0xNNxx`), mirroring the *journey of a
//! record* through the engine so operators can order failures by phase:
//!
//! | Byte   | Category                        | Execution phase |
//! |--------|---------------------------------|-----------------|
//! | `0x01` | General / API                   | argument + lifecycle checks at the caller boundary |
//! | `0x02` | Configuration                   | config load / parse / validate / merge / hot reload |
//! | `0x03` | Plugin                          | plugin registry and runtime (load, ABI, state, calls) |
//! | `0x04` | Record / Field                  | record invariants and field access |
//! | `0x05` | Buffer / Pipeline               | ingest, backpressure, pipeline stage |
//! | `0x06` | Signature / Audit chain        | key service, signing, LSN chain, audit-domain policy |
//! | `0x07` | Security / Sandbox             | plugin execution protection |
//! | `0x08` | Sink / IO                       | local + shared-memory output (incl. WORM, SHM) |
//! | `0x09` | Network / Remote               | remote sinks (Kafka/Syslog/Webhook): connect, TLS/SASL, breaker |
//! | `0x0A` | Resource / Quota               | memory / CPU / recursion limits |
//! | `0x0B` | Compliance                     | non-downgradable guarantees, audit durability |
//! | `0x0C` | Clock / Time safety            | monotonic-clock backward jumps, 2FA time skew |
//! | `0x0D` | SIF / Serialization            | frame validity, schema version |
//! | `0x0E` | Internal / Fatal               | engine-fatal conditions |
//! | `0x0F` | Reserved for core expansion    | — |
//!
//! Within a category codes ascend in encounter order with headroom for growth.
//!
//! # Plugin-defined codes
//!
//! Core only ever uses the `0x01xx`–`0x0Exx` space above. Plugin authors use
//! the high-bit range `0x80000000`–`0xFFFFFFFF` (`-0x80000000` and below) for
//! their own semantics; the core passes such values through untouched and
//! wraps them in a `DologgerDomainEvent` for sysmon.
//!
//! # Naming
//!
//! `DO_LOG_ERR_<SUBSYSTEM>_<CONDITION>` in `UPPER_SNAKE_CASE`; the condition
//! names the failure, not the recovery. Every code that crosses the C ABI is
//! mirrored in [`dologger_core.h`](../../core/include/dologger_core.h) and in
//! the reference table (`docs/.../guides/ErrorCodesReference.md`). Never shift
//! an assigned number — append at the end of the category instead.

/// Core error type for DoLogger.
///
/// All C ABI functions that can fail populate this struct
/// with an error code and human-readable message.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct DologgerError {
    /// Error code (negative i32, see constants below)
    pub code: i32,
    /// Null-terminated UTF-8 error message (owned by core)
    pub message: [u8; 256],
    /// Source file where error originated (for internal diagnostics)
    pub source_file: [u8; 128],
    /// Source line number
    pub source_line: u32,
    /// Reserved ABI space for future structured error metadata.
    pub _reserved: [u8; 12],
}

impl DologgerError {
    /// Populate this ABI error from a structured report.
    pub fn set_report(&mut self, report: &ErrorReport) {
        self.code = report.code;
        let message = report.diagnostic_message();
        self.message.fill(0);
        let bytes = message.as_bytes();
        let len = bytes.len().min(self.message.len().saturating_sub(1));
        self.message[..len].copy_from_slice(&bytes[..len]);
    }
    /// Create a new empty error (code = 0 indicates success/no-error).
    pub const fn new() -> Self {
        Self {
            code: 0,
            message: [0u8; 256],
            source_file: [0u8; 128],
            source_line: 0,
            _reserved: [0u8; 12],
        }
    }
}

impl Default for DologgerError {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 0x01xx General / API — caller-boundary checks
// ---------------------------------------------------------------------------

/// Success (no error).
pub const DO_LOG_OK: i32 = 0;
/// Invalid argument passed to API.
pub const DO_LOG_ERR_INVALID_ARG: i32 = -0x0101;
/// Operation not supported on this platform / by this build.
pub const DO_LOG_ERR_NOT_SUPPORTED: i32 = -0x0102;
/// Core engine not initialized.
pub const DO_LOG_ERR_NOT_INITIALIZED: i32 = -0x0103;
/// Core engine already initialized.
pub const DO_LOG_ERR_ALREADY_INITIALIZED: i32 = -0x0104;
/// Memory allocation failure.
pub const DO_LOG_ERR_OUT_OF_MEMORY: i32 = -0x0105;
/// Caller-provided buffer is too small for the result.
pub const DO_LOG_ERR_BUFFER_TOO_SMALL: i32 = -0x0106;
/// Operation timed out.
pub const DO_LOG_ERR_TIMEOUT: i32 = -0x0107;
/// Generic internal error (no more specific code applies).
pub const DO_LOG_ERR_INTERNAL: i32 = -0x0108;
/// Engine initialization failed with an internal fatal error.
pub const DO_LOG_ERR_INIT_FAILED: i32 = -0x0109;

// ---------------------------------------------------------------------------
// 0x02xx Configuration — load / parse / validate / merge / hot reload
// ---------------------------------------------------------------------------

/// Configuration file not found.
pub const DO_LOG_ERR_CONFIG_NOT_FOUND: i32 = -0x0201;
/// Configuration file permission denied.
pub const DO_LOG_ERR_CONFIG_PERMISSION: i32 = -0x0202;
/// Configuration parse (TOML syntax) error.
pub const DO_LOG_ERR_CONFIG_PARSE: i32 = -0x0203;
/// Configuration semantic validation failed.
pub const DO_LOG_ERR_CONFIG_VALIDATION: i32 = -0x0204;
/// Configuration merge conflict (domain inheritance).
pub const DO_LOG_ERR_CONFIG_MERGE: i32 = -0x0205;
/// Hot reload failed; the previous configuration stays in effect.
pub const DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED: i32 = -0x0206;
/// Hot reload configuration hash mismatch (file changed mid-check).
pub const DO_LOG_ERR_CONFIG_HASH_MISMATCH: i32 = -0x0207;
/// New configuration submitted for hot reload failed validation.
pub const DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID: i32 = -0x0208;
/// Reload applied non-encoding changes; protected encoding changes require restart.
pub const DO_LOG_ERR_CONFIG_RESTART_REQUIRED: i32 = -0x0209;

// ---------------------------------------------------------------------------
// 0x03xx Plugin — registry and runtime
// ---------------------------------------------------------------------------

/// Plugin not found in any search path.
pub const DO_LOG_ERR_PLUGIN_NOT_FOUND: i32 = -0x0301;
/// Plugin dynamic-library load failed (missing symbol, platform mismatch).
pub const DO_LOG_ERR_PLUGIN_LOAD_FAILED: i32 = -0x0302;
/// Plugin manifest validation failed.
pub const DO_LOG_ERR_PLUGIN_MANIFEST_INVALID: i32 = -0x0303;
/// Plugin version incompatible with the core ABI.
pub const DO_LOG_ERR_PLUGIN_VERSION_MISMATCH: i32 = -0x0304;
/// Plugin ABI incompatible with the core.
pub const DO_LOG_ERR_PLUGIN_ABI: i32 = -0x0305;
/// Plugin dependency not satisfied.
pub const DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING: i32 = -0x0306;
/// Plugin lock file mismatch (deterministic loading).
pub const DO_LOG_ERR_PLUGIN_LOCK_MISMATCH: i32 = -0x0307;
/// Plugin signature verification failed.
pub const DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID: i32 = -0x0308;
/// Plugin depends on a capability no provider offers.
pub const DO_LOG_ERR_MISSING_CAPABILITY: i32 = -0x0309;
/// Circular dependency detected in the plugin graph.
pub const DO_LOG_ERR_CIRCULAR_DEPENDENCY: i32 = -0x030A;
/// Cross-plugin call capability token chain depth exceeded.
pub const DO_LOG_ERR_TOKEN_EXCEEDED_DEPTH: i32 = -0x030B;
/// Cross-plugin call detected a deadlock (cyclic wait).
pub const DO_LOG_ERR_CALL_DEADLOCK: i32 = -0x030C;
/// Plugin state format version not supported.
pub const DO_LOG_ERR_STATE_FORMAT_UNSUPPORTED: i32 = -0x030D;
/// Plugin state migration rejected a rollback (epoch anti-rollback).
pub const DO_LOG_ERR_STATE_ROLLBACK_REJECTED: i32 = -0x030E;
/// Plugin state serialize/deserialize migration failed during reload.
pub const DO_LOG_ERR_STATE_MIGRATE_FAILED: i32 = -0x030F;

// ---------------------------------------------------------------------------
// 0x04xx Record / Field
// ---------------------------------------------------------------------------

/// Record is in an invalid state.
pub const DO_LOG_ERR_RECORD_INVALID: i32 = -0x0401;
/// Field not found in record.
pub const DO_LOG_ERR_FIELD_NOT_FOUND: i32 = -0x0402;
/// Field access denied (Ring permission violation).
pub const DO_LOG_ERR_FIELD_PERMISSION_DENIED: i32 = -0x0403;
/// Field type mismatch.
pub const DO_LOG_ERR_FIELD_TYPE_MISMATCH: i32 = -0x0404;
/// Plugin-required field not provided by an earlier pipeline stage.
pub const DO_LOG_ERR_FIELD_DEPENDENCY_NOT_MET: i32 = -0x0405;
/// A legacy text ABI input was not valid UTF-8.
pub const DO_LOG_ERR_RECORD_INVALID_ENCODING: i32 = -0x0406;

// ---------------------------------------------------------------------------
// 0x05xx Buffer / Pipeline — ingest, backpressure, pipeline stage
// ---------------------------------------------------------------------------

/// Ring buffer full and the configured strategy forbids drop/block-free.
pub const DO_LOG_ERR_BUFFER_FULL: i32 = -0x0501;
/// Pipeline stage error.
pub const DO_LOG_ERR_PIPELINE_STAGE: i32 = -0x0502;
/// Audit-domain queue full with a no-drop policy.
pub const DO_LOG_ERR_AUDIT_QUEUE_FULL: i32 = -0x0503;

// ---------------------------------------------------------------------------
// 0x06xx Signature / Audit chain
// ---------------------------------------------------------------------------

/// Signature generation failed (Assembly stage, internal error).
pub const DO_LOG_ERR_SIGN_FAILED: i32 = -0x0601;
/// Signature verification failed (possible tampering).
pub const DO_LOG_ERR_VERIFY_FAILED: i32 = -0x0602;
/// LSN chain broken (tampering detected).
pub const DO_LOG_ERR_LSN_CHAIN_BROKEN: i32 = -0x0603;
/// LSN gap detected (reorder window exceeded).
pub const DO_LOG_ERR_LSN_GAP_DETECTED: i32 = -0x0604;
/// Required key not available for signing.
pub const DO_LOG_ERR_KEY_NOT_AVAILABLE: i32 = -0x0605;
/// KeyProvider plugin open/read/sign operation failed.
pub const DO_LOG_ERR_KEY_PROVIDER_FAILED: i32 = -0x0606;
/// AUDIT domain configured with a drop strategy — forbidden.
pub const DO_LOG_ERR_AUDIT_DROP_FORBIDDEN: i32 = -0x0607;
/// AUDIT domain configured with only a callback sink — insufficient.
pub const DO_LOG_ERR_AUDIT_CALLBACK_ONLY: i32 = -0x0608;
/// AUDIT domain has no persistent primary sink.
pub const DO_LOG_ERR_AUDIT_NO_PERSISTENT_SINK: i32 = -0x0609;

// ---------------------------------------------------------------------------
// 0x07xx Security / Sandbox
// ---------------------------------------------------------------------------

/// Sandbox initialization failed.
pub const DO_LOG_ERR_SANDBOX_INIT_FAILED: i32 = -0x0701;
/// Sandbox policy violation (forbidden syscall blocked).
pub const DO_LOG_ERR_SANDBOX_VIOLATION: i32 = -0x0702;
/// Attempted to load an unsigned (Red) plugin in production mode.
pub const DO_LOG_ERR_UNTRUSTED_PLUGIN: i32 = -0x0703;

// ---------------------------------------------------------------------------
// 0x08xx Sink / IO — local and shared-memory output
// ---------------------------------------------------------------------------

/// Sink write failed (full or partial write).
pub const DO_LOG_ERR_SINK_WRITE_FAILED: i32 = -0x0801;
/// Sink failed to connect its target (file, network, broker).
pub const DO_LOG_ERR_SINK_CONNECTION_FAILED: i32 = -0x0802;
/// Sink connection lost after establishment.
pub const DO_LOG_ERR_SINK_CONNECTION_LOST: i32 = -0x0803;
/// Sink output format configuration invalid or unsupported.
pub const DO_LOG_ERR_SINK_FORMAT_INVALID: i32 = -0x0804;
/// Sink configuration rejected (invalid value, e.g. `full_policy = "block"`).
pub const DO_LOG_ERR_SINK_CONFIG_INVALID: i32 = -0x0805;
/// Sink does not support a fallback chain (e.g. `sink_shm`).
pub const DO_LOG_ERR_SINK_NO_FALLBACK: i32 = -0x0806;
/// Callback sink host invocation timed out.
pub const DO_LOG_ERR_CALLBACK_TIMEOUT: i32 = -0x0807;
/// WORM write failed (disk full, permission).
pub const DO_LOG_ERR_WORM_WRITE_FAILED: i32 = -0x0808;
/// Shared-memory object create/map failed (permission, space).
pub const DO_LOG_ERR_SHM_INIT_FAILED: i32 = -0x0809;
/// Shared-memory ring buffer full (only surfaced with a block policy).
pub const DO_LOG_ERR_SHM_RING_FULL: i32 = -0x080A;
/// `sink_shm` configured for an AUDIT domain — forbidden.
pub const DO_LOG_ERR_AUDIT_SHM_FORBIDDEN: i32 = -0x080B;

// ---------------------------------------------------------------------------
// 0x09xx Network / Remote — remote sinks
// ---------------------------------------------------------------------------

/// Remote-sink circuit breaker is OPEN; writes are rejected.
pub const DO_LOG_ERR_CIRCUIT_OPEN: i32 = -0x0901;
/// TLS handshake / certificate failure on a remote sink.
pub const DO_LOG_ERR_TLS_FAILED: i32 = -0x0902;
/// SASL authentication failure on a remote sink.
pub const DO_LOG_ERR_SASL_FAILED: i32 = -0x0903;
/// Remote sink operation timed out (producer call, batch ack).
pub const DO_LOG_ERR_REMOTE_TIMEOUT: i32 = -0x0904;

// ---------------------------------------------------------------------------
// 0x0Axx Resource / Quota
// ---------------------------------------------------------------------------

/// Plugin memory usage exceeded its configured quota.
pub const DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED: i32 = -0x0A01;
/// Plugin CPU usage exceeded its configured quota.
pub const DO_LOG_ERR_QUOTA_CPU_EXCEEDED: i32 = -0x0A02;
/// Logging self-reference recursion depth exceeded.
pub const DO_LOG_ERR_RECURSION_DEPTH_EXCEEDED: i32 = -0x0A03;

// ---------------------------------------------------------------------------
// 0x0Bxx Compliance
// ---------------------------------------------------------------------------

/// Compliance violation (template vs manual config conflict, or a
/// non-downgradable item relaxed).
pub const DO_LOG_ERR_COMPLIANCE_VIOLATION: i32 = -0x0B01;
/// AUDIT domain sink durability below the required MEDIA level.
pub const DO_LOG_ERR_AUDIT_DURABILITY_INSUFFICIENT: i32 = -0x0B02;

// ---------------------------------------------------------------------------
// 0x0Cxx Clock / Time safety
// ---------------------------------------------------------------------------

/// Monotonic clock jumped backward; AUDIT domain frozen.
pub const DO_LOG_ERR_TIME_BACKWARD: i32 = -0x0C01;

// ---------------------------------------------------------------------------
// 0x0Dxx SIF / Serialization
// ---------------------------------------------------------------------------

/// SIF frame malformed (bad magic, version, or length) or failed FlatBuffer
/// structural verification.
pub const DO_LOG_ERR_SIF_INVALID: i32 = -0x0D01;
/// SIF schema version declared by a plugin is not supported by the core.
pub const DO_LOG_ERR_SIF_VERSION_UNSUPPORTED: i32 = -0x0D02;
/// KV frame is malformed or violates resource limits.
pub const DO_LOG_ERR_KV_INVALID: i32 = -0x0D03;
/// KV frame version is not supported by this core.
pub const DO_LOG_ERR_KV_VERSION_UNSUPPORTED: i32 = -0x0D04;
/// KV frame content hash does not match canonical record bytes.
pub const DO_LOG_ERR_KV_HASH_MISMATCH: i32 = -0x0D05;

// ---------------------------------------------------------------------------
/// The stage or boundary that produced an error report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorOrigin {
    /// Public API or FFI argument boundary.
    Api,
    /// Configuration load or hot reload.
    Config,
    /// Plugin registry or plugin callback.
    Plugin,
    /// Record and field API.
    Record,
    /// Pipeline execution.
    Pipeline,
    /// Security and audit path.
    Security,
    /// Serialization and wire validation.
    Serialization,
    /// Sink and operating-system I/O.
    Sink,
    /// Internal engine invariant.
    Internal,
}

/// Structured context attached to one error exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorContext {
    /// Error subsystem.
    pub origin: ErrorOrigin,
    /// Stable operation identifier, never a localized sentence.
    pub operation: &'static str,
    /// Optional bounded diagnostic detail for logs, not for control flow.
    pub detail: Option<String>,
}

impl ErrorContext {
    /// Create context without allocating diagnostic detail.
    pub const fn new(origin: ErrorOrigin, operation: &'static str) -> Self {
        Self {
            origin,
            operation,
            detail: None,
        }
    }

    /// Attach bounded detail for diagnostics.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let mut detail = detail.into();
        detail.truncate(512);
        self.detail = Some(detail);
        self
    }
}

/// Structured error exit used between core subsystems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReport {
    /// Stable numeric error code.
    pub code: i32,
    /// Stable descriptor used by localization and automation.
    pub descriptor: ErrorDescriptor,
    /// Origin and operation context.
    pub context: ErrorContext,
}

impl ErrorReport {
    /// Build an error report from a stable code and operation key.
    pub fn new(code: i32, context: ErrorContext) -> Self {
        Self {
            code,
            descriptor: error_descriptor(code),
            context,
        }
    }

    /// Return the locale-independent message key.
    pub const fn key(&self) -> &'static str {
        self.descriptor.key
    }

    /// Return the English fallback without exposing dynamic detail as text.
    pub const fn fallback_message(&self) -> &'static str {
        self.descriptor.default_message
    }

    /// Render a bounded diagnostic string for internal logs only.
    pub fn diagnostic_message(&self) -> String {
        match &self.context.detail {
            Some(detail) if !detail.is_empty() => {
                format!("{}: {detail}", self.descriptor.default_message)
            }
            _ => self.descriptor.default_message.to_string(),
        }
    }
}

/// A result extension that gives every subsystem a stable error exit.
pub trait ErrorExit<T> {
    /// Convert an error code and operation into an [`ErrorReport`].
    fn report(self, code: i32, context: ErrorContext) -> Result<T, ErrorReport>;
}

impl<T, E> ErrorExit<T> for Result<T, E>
where
    E: std::fmt::Display,
{
    fn report(self, code: i32, context: ErrorContext) -> Result<T, ErrorReport> {
        self.map_err(|error| ErrorReport::new(code, context.with_detail(error.to_string())))
    }
}
// 0x0Exx Internal / Fatal
// ---------------------------------------------------------------------------

/// Engine-fatal condition (plugin unloaded; sink triggers `SINK_CIRCUIT_OPEN`).
pub const DO_LOG_ERR_FATAL: i32 = -0x0E01;

/// Stable metadata for an error code.
///
/// `key` is the locale-independent lookup key. `default_message` is the
/// English fallback used when no message catalog is installed. Callers must
/// branch on `code`, never parse either string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorDescriptor {
    /// Machine-stable error code.
    pub code: i32,
    /// Locale-independent message key.
    pub key: &'static str,
    /// English fallback message.
    pub default_message: &'static str,
}

/// Return the stable descriptor for a core or plugin-defined error code.
pub const fn error_descriptor(code: i32) -> ErrorDescriptor {
    let (key, default_message) = match code {
        DO_LOG_OK => ("ok", "ok"),
        DO_LOG_ERR_INVALID_ARG => ("error.invalid_arg", "invalid argument"),
        DO_LOG_ERR_NOT_SUPPORTED => ("error.not_supported", "operation not supported"),
        DO_LOG_ERR_NOT_INITIALIZED => ("error.not_initialized", "engine not initialized"),
        DO_LOG_ERR_ALREADY_INITIALIZED => {
            ("error.already_initialized", "engine already initialized")
        }
        DO_LOG_ERR_OUT_OF_MEMORY => ("error.out_of_memory", "memory allocation failed"),
        DO_LOG_ERR_BUFFER_TOO_SMALL => ("error.buffer_too_small", "buffer too small"),
        DO_LOG_ERR_TIMEOUT => ("error.timeout", "operation timed out"),
        DO_LOG_ERR_INTERNAL => ("error.internal", "internal error"),
        DO_LOG_ERR_INIT_FAILED => ("error.init_failed", "engine initialization failed"),
        DO_LOG_ERR_CONFIG_NOT_FOUND => ("config.not_found", "configuration not found"),
        DO_LOG_ERR_CONFIG_PERMISSION => ("config.permission", "configuration permission denied"),
        DO_LOG_ERR_CONFIG_PARSE => ("config.parse", "configuration parse failed"),
        DO_LOG_ERR_CONFIG_VALIDATION => ("config.validation", "configuration validation failed"),
        DO_LOG_ERR_CONFIG_MERGE => ("config.merge", "configuration merge failed"),
        DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED => (
            "config.hot_reload_failed",
            "configuration hot reload failed",
        ),
        DO_LOG_ERR_CONFIG_HASH_MISMATCH => ("config.hash_mismatch", "configuration hash mismatch"),
        DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID => (
            "config.hot_reload_invalid",
            "hot reload configuration is invalid",
        ),
        DO_LOG_ERR_CONFIG_RESTART_REQUIRED => (
            "config.restart_required",
            "configuration restart required for protected changes",
        ),
        DO_LOG_ERR_PLUGIN_NOT_FOUND => ("plugin.not_found", "plugin not found"),
        DO_LOG_ERR_PLUGIN_LOAD_FAILED => ("plugin.load_failed", "plugin load failed"),
        DO_LOG_ERR_PLUGIN_MANIFEST_INVALID => {
            ("plugin.manifest_invalid", "plugin manifest is invalid")
        }
        DO_LOG_ERR_PLUGIN_VERSION_MISMATCH => {
            ("plugin.version_mismatch", "plugin version mismatch")
        }
        DO_LOG_ERR_PLUGIN_ABI => ("plugin.abi", "plugin ABI mismatch"),
        DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING => {
            ("plugin.dependency_missing", "plugin dependency missing")
        }
        DO_LOG_ERR_PLUGIN_LOCK_MISMATCH => ("plugin.lock_mismatch", "plugin lock mismatch"),
        DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID => {
            ("plugin.signature_invalid", "plugin signature invalid")
        }
        DO_LOG_ERR_MISSING_CAPABILITY => ("plugin.capability_missing", "plugin capability missing"),
        DO_LOG_ERR_CIRCULAR_DEPENDENCY => (
            "plugin.circular_dependency",
            "plugin dependency cycle detected",
        ),
        DO_LOG_ERR_TOKEN_EXCEEDED_DEPTH => ("plugin.token_depth", "plugin token depth exceeded"),
        DO_LOG_ERR_CALL_DEADLOCK => ("plugin.call_deadlock", "plugin call deadlock detected"),
        DO_LOG_ERR_STATE_FORMAT_UNSUPPORTED => {
            ("plugin.state_format", "plugin state format unsupported")
        }
        DO_LOG_ERR_STATE_ROLLBACK_REJECTED => {
            ("plugin.state_rollback", "plugin state rollback rejected")
        }
        DO_LOG_ERR_STATE_MIGRATE_FAILED => {
            ("plugin.state_migration", "plugin state migration failed")
        }
        DO_LOG_ERR_RECORD_INVALID => ("record.invalid", "record is invalid"),
        DO_LOG_ERR_FIELD_NOT_FOUND => ("field.not_found", "field not found"),
        DO_LOG_ERR_FIELD_PERMISSION_DENIED => {
            ("field.permission_denied", "field permission denied")
        }
        DO_LOG_ERR_FIELD_TYPE_MISMATCH => ("field.type_mismatch", "field type mismatch"),
        DO_LOG_ERR_FIELD_DEPENDENCY_NOT_MET => {
            ("field.dependency_not_met", "field dependency not met")
        }
        DO_LOG_ERR_RECORD_INVALID_ENCODING => {
            ("record.invalid_encoding", "record input is not valid UTF-8")
        }
        DO_LOG_ERR_BUFFER_FULL => ("buffer.full", "buffer is full"),
        DO_LOG_ERR_PIPELINE_STAGE => ("pipeline.stage", "pipeline stage failed"),
        DO_LOG_ERR_AUDIT_QUEUE_FULL => ("audit.queue_full", "audit queue is full"),
        DO_LOG_ERR_SIGN_FAILED => ("audit.sign_failed", "record signing failed"),
        DO_LOG_ERR_VERIFY_FAILED => ("audit.verify_failed", "record verification failed"),
        DO_LOG_ERR_LSN_CHAIN_BROKEN => ("audit.lsn_chain_broken", "audit LSN chain is broken"),
        DO_LOG_ERR_LSN_GAP_DETECTED => ("audit.lsn_gap", "audit LSN gap detected"),
        DO_LOG_ERR_KEY_NOT_AVAILABLE => ("audit.key_unavailable", "signing key unavailable"),
        DO_LOG_ERR_KEY_PROVIDER_FAILED => ("audit.key_provider_failed", "key provider failed"),
        DO_LOG_ERR_AUDIT_DROP_FORBIDDEN => ("audit.drop_forbidden", "audit record drop forbidden"),
        DO_LOG_ERR_AUDIT_CALLBACK_ONLY => {
            ("audit.callback_only", "audit callback-only mode rejected")
        }
        DO_LOG_ERR_AUDIT_NO_PERSISTENT_SINK => (
            "audit.no_persistent_sink",
            "audit persistence sink unavailable",
        ),
        DO_LOG_ERR_SANDBOX_INIT_FAILED => {
            ("security.sandbox_init", "sandbox initialization failed")
        }
        DO_LOG_ERR_SANDBOX_VIOLATION => ("security.sandbox_violation", "sandbox violation"),
        DO_LOG_ERR_UNTRUSTED_PLUGIN => ("security.untrusted_plugin", "untrusted plugin"),
        DO_LOG_ERR_SINK_WRITE_FAILED => ("sink.write_failed", "sink write failed"),
        DO_LOG_ERR_SINK_CONNECTION_FAILED => ("sink.connection_failed", "sink connection failed"),
        DO_LOG_ERR_SINK_CONNECTION_LOST => ("sink.connection_lost", "sink connection lost"),
        DO_LOG_ERR_SINK_FORMAT_INVALID => ("sink.format_invalid", "sink format invalid"),
        DO_LOG_ERR_SINK_CONFIG_INVALID => ("sink.config_invalid", "sink configuration invalid"),
        DO_LOG_ERR_SINK_NO_FALLBACK => ("sink.no_fallback", "sink fallback unavailable"),
        DO_LOG_ERR_CALLBACK_TIMEOUT => ("sink.callback_timeout", "sink callback timed out"),
        DO_LOG_ERR_WORM_WRITE_FAILED => ("sink.worm_write_failed", "WORM write failed"),
        DO_LOG_ERR_SHM_INIT_FAILED => (
            "sink.shm_init_failed",
            "shared-memory sink initialization failed",
        ),
        DO_LOG_ERR_SHM_RING_FULL => ("sink.shm_ring_full", "shared-memory ring is full"),
        DO_LOG_ERR_AUDIT_SHM_FORBIDDEN => (
            "audit.shm_forbidden",
            "shared-memory sink forbidden for audit",
        ),
        DO_LOG_ERR_CIRCUIT_OPEN => ("network.circuit_open", "sink circuit is open"),
        DO_LOG_ERR_TLS_FAILED => ("network.tls_failed", "TLS operation failed"),
        DO_LOG_ERR_SASL_FAILED => ("network.sasl_failed", "SASL operation failed"),
        DO_LOG_ERR_REMOTE_TIMEOUT => ("network.remote_timeout", "remote operation timed out"),
        DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED => ("resource.memory_quota", "memory quota exceeded"),
        DO_LOG_ERR_QUOTA_CPU_EXCEEDED => ("resource.cpu_quota", "CPU quota exceeded"),
        DO_LOG_ERR_RECURSION_DEPTH_EXCEEDED => {
            ("resource.recursion_depth", "recursion depth exceeded")
        }
        DO_LOG_ERR_COMPLIANCE_VIOLATION => {
            ("compliance.violation", "compliance requirement violated")
        }
        DO_LOG_ERR_AUDIT_DURABILITY_INSUFFICIENT => (
            "compliance.audit_durability",
            "audit durability is insufficient",
        ),
        DO_LOG_ERR_TIME_BACKWARD => ("time.backward", "clock moved backward"),
        DO_LOG_ERR_SIF_INVALID => ("sif.invalid", "SIF frame invalid"),
        DO_LOG_ERR_SIF_VERSION_UNSUPPORTED => {
            ("sif.version_unsupported", "SIF version unsupported")
        }
        DO_LOG_ERR_KV_INVALID => ("kv.invalid", "KV frame invalid"),
        DO_LOG_ERR_KV_VERSION_UNSUPPORTED => ("kv.version_unsupported", "KV version unsupported"),
        DO_LOG_ERR_KV_HASH_MISMATCH => ("kv.hash_mismatch", "KV content hash mismatch"),
        DO_LOG_ERR_FATAL => ("fatal", "fatal engine error"),
        _ if code <= -0x8000_0000 => ("plugin.custom", "plugin-defined error"),
        _ => ("error.unknown", "unknown error"),
    };
    ErrorDescriptor {
        code,
        key,
        default_message,
    }
}

/// Return the locale-independent key for an error code.
pub const fn error_key(code: i32) -> &'static str {
    error_descriptor(code).key
}

/// Return the English fallback message for an error code.
pub const fn error_default_message(code: i32) -> &'static str {
    error_descriptor(code).default_message
}

// ---------------------------------------------------------------------------
// Domain event structure
// ---------------------------------------------------------------------------

/// Structured domain event for diagnostics and audit.
///
/// Every error or significant internal event generates a `DologgerDomainEvent`
/// that can be consumed by the sysmon channel or external monitoring.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct DologgerDomainEvent {
    /// Error code (see constants above) or 0 for informational events
    pub error_code: i32,
    /// Event category string (e.g., "config", "plugin", "audit")
    pub category: [u8; 32],
    /// Human-readable event description
    pub description: [u8; 512],
    /// Timestamp of the event (monotonic milliseconds since engine init)
    pub timestamp_ms: u64,
    /// Severity level (0 = DEBUG, 1 = INFO, 2 = WARN, 3 = ERROR, 4 = CRITICAL, 5 = EMERGENCY)
    pub severity: u8,
    /// Reserved for future use (padding to align struct)
    pub _reserved: [u8; 7],
}

/// Severity levels for domain events.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventSeverity {
    /// Debug information, not actionable
    Debug = 0,
    /// Informational event, normal operation
    Info = 1,
    /// Warning, may need attention
    Warn = 2,
    /// Error condition, requires investigation
    Error = 3,
    /// Critical failure, immediate action needed
    Critical = 4,
    /// Emergency, system may be unstable
    Emergency = 5,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_is_zero() {
        assert_eq!(DO_LOG_OK, 0);
    }

    #[test]
    fn descriptors_keep_machine_key_separate_from_default_text() {
        let descriptor = error_descriptor(DO_LOG_ERR_INVALID_ARG);
        assert_eq!(descriptor.code, DO_LOG_ERR_INVALID_ARG);
        assert_eq!(descriptor.key, "error.invalid_arg");
        assert_ne!(descriptor.key, descriptor.default_message);
    }

    #[test]
    fn all_errors_are_negative() {
        let codes = [
            DO_LOG_ERR_INVALID_ARG,
            DO_LOG_ERR_NOT_SUPPORTED,
            DO_LOG_ERR_NOT_INITIALIZED,
            DO_LOG_ERR_ALREADY_INITIALIZED,
            DO_LOG_ERR_OUT_OF_MEMORY,
            DO_LOG_ERR_BUFFER_TOO_SMALL,
            DO_LOG_ERR_TIMEOUT,
            DO_LOG_ERR_INTERNAL,
            DO_LOG_ERR_INIT_FAILED,
            DO_LOG_ERR_CONFIG_NOT_FOUND,
            DO_LOG_ERR_CONFIG_PERMISSION,
            DO_LOG_ERR_CONFIG_PARSE,
            DO_LOG_ERR_CONFIG_VALIDATION,
            DO_LOG_ERR_CONFIG_MERGE,
            DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED,
            DO_LOG_ERR_CONFIG_HASH_MISMATCH,
            DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID,
            DO_LOG_ERR_CONFIG_RESTART_REQUIRED,
            DO_LOG_ERR_PLUGIN_NOT_FOUND,
            DO_LOG_ERR_PLUGIN_LOAD_FAILED,
            DO_LOG_ERR_PLUGIN_MANIFEST_INVALID,
            DO_LOG_ERR_PLUGIN_VERSION_MISMATCH,
            DO_LOG_ERR_PLUGIN_ABI,
            DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING,
            DO_LOG_ERR_PLUGIN_LOCK_MISMATCH,
            DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID,
            DO_LOG_ERR_MISSING_CAPABILITY,
            DO_LOG_ERR_CIRCULAR_DEPENDENCY,
            DO_LOG_ERR_TOKEN_EXCEEDED_DEPTH,
            DO_LOG_ERR_CALL_DEADLOCK,
            DO_LOG_ERR_STATE_FORMAT_UNSUPPORTED,
            DO_LOG_ERR_STATE_ROLLBACK_REJECTED,
            DO_LOG_ERR_STATE_MIGRATE_FAILED,
            DO_LOG_ERR_RECORD_INVALID,
            DO_LOG_ERR_FIELD_NOT_FOUND,
            DO_LOG_ERR_FIELD_PERMISSION_DENIED,
            DO_LOG_ERR_FIELD_TYPE_MISMATCH,
            DO_LOG_ERR_FIELD_DEPENDENCY_NOT_MET,
            DO_LOG_ERR_RECORD_INVALID_ENCODING,
            DO_LOG_ERR_BUFFER_FULL,
            DO_LOG_ERR_PIPELINE_STAGE,
            DO_LOG_ERR_AUDIT_QUEUE_FULL,
            DO_LOG_ERR_SIGN_FAILED,
            DO_LOG_ERR_VERIFY_FAILED,
            DO_LOG_ERR_LSN_CHAIN_BROKEN,
            DO_LOG_ERR_LSN_GAP_DETECTED,
            DO_LOG_ERR_KEY_NOT_AVAILABLE,
            DO_LOG_ERR_KEY_PROVIDER_FAILED,
            DO_LOG_ERR_AUDIT_DROP_FORBIDDEN,
            DO_LOG_ERR_AUDIT_CALLBACK_ONLY,
            DO_LOG_ERR_AUDIT_NO_PERSISTENT_SINK,
            DO_LOG_ERR_SANDBOX_INIT_FAILED,
            DO_LOG_ERR_SANDBOX_VIOLATION,
            DO_LOG_ERR_UNTRUSTED_PLUGIN,
            DO_LOG_ERR_SINK_WRITE_FAILED,
            DO_LOG_ERR_SINK_CONNECTION_FAILED,
            DO_LOG_ERR_SINK_CONNECTION_LOST,
            DO_LOG_ERR_SINK_FORMAT_INVALID,
            DO_LOG_ERR_SINK_CONFIG_INVALID,
            DO_LOG_ERR_SINK_NO_FALLBACK,
            DO_LOG_ERR_CALLBACK_TIMEOUT,
            DO_LOG_ERR_WORM_WRITE_FAILED,
            DO_LOG_ERR_SHM_INIT_FAILED,
            DO_LOG_ERR_SHM_RING_FULL,
            DO_LOG_ERR_AUDIT_SHM_FORBIDDEN,
            DO_LOG_ERR_CIRCUIT_OPEN,
            DO_LOG_ERR_TLS_FAILED,
            DO_LOG_ERR_SASL_FAILED,
            DO_LOG_ERR_REMOTE_TIMEOUT,
            DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED,
            DO_LOG_ERR_QUOTA_CPU_EXCEEDED,
            DO_LOG_ERR_RECURSION_DEPTH_EXCEEDED,
            DO_LOG_ERR_COMPLIANCE_VIOLATION,
            DO_LOG_ERR_AUDIT_DURABILITY_INSUFFICIENT,
            DO_LOG_ERR_TIME_BACKWARD,
            DO_LOG_ERR_SIF_INVALID,
            DO_LOG_ERR_SIF_VERSION_UNSUPPORTED,
            DO_LOG_ERR_KV_INVALID,
            DO_LOG_ERR_KV_VERSION_UNSUPPORTED,
            DO_LOG_ERR_KV_HASH_MISMATCH,
            DO_LOG_ERR_FATAL,
        ];
        assert!(codes.iter().all(|c| *c < 0), "all error codes are negative");
    }

    #[test]
    fn categories_are_disjoint_and_unique() {
        // Full set must contain no duplicate values.
        let mut seen = std::collections::BTreeSet::new();
        let pairs: &[(i32, &str)] = &[
            (DO_LOG_ERR_INVALID_ARG, "INVALID_ARG"),
            (DO_LOG_ERR_NOT_SUPPORTED, "NOT_SUPPORTED"),
            (DO_LOG_ERR_NOT_INITIALIZED, "NOT_INITIALIZED"),
            (DO_LOG_ERR_ALREADY_INITIALIZED, "ALREADY_INITIALIZED"),
            (DO_LOG_ERR_OUT_OF_MEMORY, "OUT_OF_MEMORY"),
            (DO_LOG_ERR_BUFFER_TOO_SMALL, "BUFFER_TOO_SMALL"),
            (DO_LOG_ERR_TIMEOUT, "TIMEOUT"),
            (DO_LOG_ERR_INTERNAL, "INTERNAL"),
            (DO_LOG_ERR_INIT_FAILED, "INIT_FAILED"),
            (DO_LOG_ERR_CONFIG_NOT_FOUND, "CONFIG_NOT_FOUND"),
            (DO_LOG_ERR_CONFIG_PERMISSION, "CONFIG_PERMISSION"),
            (DO_LOG_ERR_CONFIG_PARSE, "CONFIG_PARSE"),
            (DO_LOG_ERR_CONFIG_VALIDATION, "CONFIG_VALIDATION"),
            (DO_LOG_ERR_CONFIG_MERGE, "CONFIG_MERGE"),
            (
                DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED,
                "CONFIG_HOT_RELOAD_FAILED",
            ),
            (DO_LOG_ERR_CONFIG_HASH_MISMATCH, "CONFIG_HASH_MISMATCH"),
            (
                DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID,
                "CONFIG_HOT_RELOAD_INVALID",
            ),
            (
                DO_LOG_ERR_CONFIG_RESTART_REQUIRED,
                "CONFIG_RESTART_REQUIRED",
            ),
            (DO_LOG_ERR_PLUGIN_NOT_FOUND, "PLUGIN_NOT_FOUND"),
            (DO_LOG_ERR_PLUGIN_LOAD_FAILED, "PLUGIN_LOAD_FAILED"),
            (
                DO_LOG_ERR_PLUGIN_MANIFEST_INVALID,
                "PLUGIN_MANIFEST_INVALID",
            ),
            (
                DO_LOG_ERR_PLUGIN_VERSION_MISMATCH,
                "PLUGIN_VERSION_MISMATCH",
            ),
            (DO_LOG_ERR_PLUGIN_ABI, "PLUGIN_ABI"),
            (
                DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING,
                "PLUGIN_DEPENDENCY_MISSING",
            ),
            (DO_LOG_ERR_PLUGIN_LOCK_MISMATCH, "PLUGIN_LOCK_MISMATCH"),
            (
                DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID,
                "PLUGIN_SIGNATURE_INVALID",
            ),
            (DO_LOG_ERR_MISSING_CAPABILITY, "MISSING_CAPABILITY"),
            (DO_LOG_ERR_CIRCULAR_DEPENDENCY, "CIRCULAR_DEPENDENCY"),
            (DO_LOG_ERR_TOKEN_EXCEEDED_DEPTH, "TOKEN_EXCEEDED_DEPTH"),
            (DO_LOG_ERR_CALL_DEADLOCK, "CALL_DEADLOCK"),
            (
                DO_LOG_ERR_STATE_FORMAT_UNSUPPORTED,
                "STATE_FORMAT_UNSUPPORTED",
            ),
            (
                DO_LOG_ERR_STATE_ROLLBACK_REJECTED,
                "STATE_ROLLBACK_REJECTED",
            ),
            (DO_LOG_ERR_STATE_MIGRATE_FAILED, "STATE_MIGRATE_FAILED"),
            (DO_LOG_ERR_RECORD_INVALID, "RECORD_INVALID"),
            (DO_LOG_ERR_FIELD_NOT_FOUND, "FIELD_NOT_FOUND"),
            (
                DO_LOG_ERR_FIELD_PERMISSION_DENIED,
                "FIELD_PERMISSION_DENIED",
            ),
            (DO_LOG_ERR_FIELD_TYPE_MISMATCH, "FIELD_TYPE_MISMATCH"),
            (
                DO_LOG_ERR_FIELD_DEPENDENCY_NOT_MET,
                "FIELD_DEPENDENCY_NOT_MET",
            ),
            (
                DO_LOG_ERR_RECORD_INVALID_ENCODING,
                "RECORD_INVALID_ENCODING",
            ),
            (DO_LOG_ERR_BUFFER_FULL, "BUFFER_FULL"),
            (DO_LOG_ERR_PIPELINE_STAGE, "PIPELINE_STAGE"),
            (DO_LOG_ERR_AUDIT_QUEUE_FULL, "AUDIT_QUEUE_FULL"),
            (DO_LOG_ERR_SIGN_FAILED, "SIGN_FAILED"),
            (DO_LOG_ERR_VERIFY_FAILED, "VERIFY_FAILED"),
            (DO_LOG_ERR_LSN_CHAIN_BROKEN, "LSN_CHAIN_BROKEN"),
            (DO_LOG_ERR_LSN_GAP_DETECTED, "LSN_GAP_DETECTED"),
            (DO_LOG_ERR_KEY_NOT_AVAILABLE, "KEY_NOT_AVAILABLE"),
            (DO_LOG_ERR_KEY_PROVIDER_FAILED, "KEY_PROVIDER_FAILED"),
            (DO_LOG_ERR_AUDIT_DROP_FORBIDDEN, "AUDIT_DROP_FORBIDDEN"),
            (DO_LOG_ERR_AUDIT_CALLBACK_ONLY, "AUDIT_CALLBACK_ONLY"),
            (
                DO_LOG_ERR_AUDIT_NO_PERSISTENT_SINK,
                "AUDIT_NO_PERSISTENT_SINK",
            ),
            (DO_LOG_ERR_SANDBOX_INIT_FAILED, "SANDBOX_INIT_FAILED"),
            (DO_LOG_ERR_SANDBOX_VIOLATION, "SANDBOX_VIOLATION"),
            (DO_LOG_ERR_UNTRUSTED_PLUGIN, "UNTRUSTED_PLUGIN"),
            (DO_LOG_ERR_SINK_WRITE_FAILED, "SINK_WRITE_FAILED"),
            (DO_LOG_ERR_SINK_CONNECTION_FAILED, "SINK_CONNECTION_FAILED"),
            (DO_LOG_ERR_SINK_CONNECTION_LOST, "SINK_CONNECTION_LOST"),
            (DO_LOG_ERR_SINK_FORMAT_INVALID, "SINK_FORMAT_INVALID"),
            (DO_LOG_ERR_SINK_CONFIG_INVALID, "SINK_CONFIG_INVALID"),
            (DO_LOG_ERR_SINK_NO_FALLBACK, "SINK_NO_FALLBACK"),
            (DO_LOG_ERR_CALLBACK_TIMEOUT, "CALLBACK_TIMEOUT"),
            (DO_LOG_ERR_WORM_WRITE_FAILED, "WORM_WRITE_FAILED"),
            (DO_LOG_ERR_SHM_INIT_FAILED, "SHM_INIT_FAILED"),
            (DO_LOG_ERR_SHM_RING_FULL, "SHM_RING_FULL"),
            (DO_LOG_ERR_AUDIT_SHM_FORBIDDEN, "AUDIT_SHM_FORBIDDEN"),
            (DO_LOG_ERR_CIRCUIT_OPEN, "CIRCUIT_OPEN"),
            (DO_LOG_ERR_TLS_FAILED, "TLS_FAILED"),
            (DO_LOG_ERR_SASL_FAILED, "SASL_FAILED"),
            (DO_LOG_ERR_REMOTE_TIMEOUT, "REMOTE_TIMEOUT"),
            (DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED, "QUOTA_MEMORY_EXCEEDED"),
            (DO_LOG_ERR_QUOTA_CPU_EXCEEDED, "QUOTA_CPU_EXCEEDED"),
            (
                DO_LOG_ERR_RECURSION_DEPTH_EXCEEDED,
                "RECURSION_DEPTH_EXCEEDED",
            ),
            (DO_LOG_ERR_COMPLIANCE_VIOLATION, "COMPLIANCE_VIOLATION"),
            (
                DO_LOG_ERR_AUDIT_DURABILITY_INSUFFICIENT,
                "AUDIT_DURABILITY_INSUFFICIENT",
            ),
            (DO_LOG_ERR_TIME_BACKWARD, "TIME_BACKWARD"),
            (DO_LOG_ERR_SIF_INVALID, "SIF_INVALID"),
            (
                DO_LOG_ERR_SIF_VERSION_UNSUPPORTED,
                "SIF_VERSION_UNSUPPORTED",
            ),
            (DO_LOG_ERR_KV_INVALID, "KV_INVALID"),
            (DO_LOG_ERR_KV_VERSION_UNSUPPORTED, "KV_VERSION_UNSUPPORTED"),
            (DO_LOG_ERR_KV_HASH_MISMATCH, "KV_HASH_MISMATCH"),
            (DO_LOG_ERR_FATAL, "FATAL"),
        ];
        for (code, name) in pairs {
            assert!(
                seen.insert(*code),
                "duplicate error code value {code:#x} for {name} — codes must be unique"
            );
        }
    }
}
