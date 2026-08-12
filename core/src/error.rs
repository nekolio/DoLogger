//! Error codes and domain event structures for DoLogger.
//!
//! All error codes are negative `i32` values grouped by category.
//! Domain events provide structured error information for diagnostics.

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
}

impl DologgerError {
    /// Create a new empty error (code = 0 indicates success/no-error).
    pub const fn new() -> Self {
        Self {
            code: 0,
            message: [0u8; 256],
            source_file: [0u8; 128],
            source_line: 0,
        }
    }
}

impl Default for DologgerError {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Error category prefixes (hex nibble scheme)
// ---------------------------------------------------------------------------
// 0x01xx : General / Initialization
// 0x02xx : Configuration
// 0x03xx : Plugin management
// 0x04xx : Record / Field access
// 0x05xx : Ring buffer / Pipeline
// 0x06xx : Signature / Audit chain
// 0x07xx : Sink / IO
// 0x08xx : Sandbox / Security
// 0x09xx : Resource / Quota
// 0x0Axx : Network / RPC
// 0x0Bxx : Compliance
// 0x0Cxx : Internal / Fatal

// --- General / Initialization (0x01xx) ---
/// Success (no error)
pub const DO_LOG_OK: i32 = 0;
/// Generic internal error
pub const DO_LOG_ERR_INTERNAL: i32 = -0x0101;
/// Invalid argument passed to API
pub const DO_LOG_ERR_INVALID_ARG: i32 = -0x0102;
/// Operation not supported on this platform
pub const DO_LOG_ERR_NOT_SUPPORTED: i32 = -0x0103;
/// Core engine not initialized
pub const DO_LOG_ERR_NOT_INITIALIZED: i32 = -0x0104;
/// Core engine already initialized
pub const DO_LOG_ERR_ALREADY_INITIALIZED: i32 = -0x0105;
/// Memory allocation failure
pub const DO_LOG_ERR_OUT_OF_MEMORY: i32 = -0x0106;
/// Buffer too small for operation
pub const DO_LOG_ERR_BUFFER_TOO_SMALL: i32 = -0x0107;
/// Operation timed out
pub const DO_LOG_ERR_TIMEOUT: i32 = -0x0108;

// --- Configuration (0x02xx) ---
/// Configuration file not found
pub const DO_LOG_ERR_CONFIG_NOT_FOUND: i32 = -0x0201;
/// Configuration file permission denied
pub const DO_LOG_ERR_CONFIG_PERMISSION: i32 = -0x0202;
/// Configuration parse/syntax error
pub const DO_LOG_ERR_CONFIG_PARSE: i32 = -0x0203;
/// Configuration validation failed
pub const DO_LOG_ERR_CONFIG_VALIDATION: i32 = -0x0204;
/// Configuration merge conflict
pub const DO_LOG_ERR_CONFIG_MERGE: i32 = -0x0205;
/// Hot reload failed, keeping previous config
pub const DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED: i32 = -0x0206;

// --- Plugin (0x03xx) ---
/// Plugin not found
pub const DO_LOG_ERR_PLUGIN_NOT_FOUND: i32 = -0x0301;
/// Plugin failed to load (link error or missing symbols)
pub const DO_LOG_ERR_PLUGIN_LOAD_FAILED: i32 = -0x0302;
/// Plugin manifest validation failed
pub const DO_LOG_ERR_PLUGIN_MANIFEST_INVALID: i32 = -0x0303;
/// Plugin version incompatible with core ABI
pub const DO_LOG_ERR_PLUGIN_VERSION_MISMATCH: i32 = -0x0304;
/// Plugin dependency not satisfied
pub const DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING: i32 = -0x0305;
/// Plugin lock file mismatch (deterministic loading)
pub const DO_LOG_ERR_PLUGIN_LOCK_MISMATCH: i32 = -0x0306;
/// Plugin signature verification failed
pub const DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID: i32 = -0x0307;

// --- Record / Field (0x04xx) ---
/// Field not found in record
pub const DO_LOG_ERR_FIELD_NOT_FOUND: i32 = -0x0401;
/// Field access denied (Ring permission violation)
pub const DO_LOG_ERR_FIELD_PERMISSION_DENIED: i32 = -0x0402;
/// Field type mismatch
pub const DO_LOG_ERR_FIELD_TYPE_MISMATCH: i32 = -0x0403;
/// Record is in invalid state
pub const DO_LOG_ERR_RECORD_INVALID: i32 = -0x0404;

// --- Ring Buffer / Pipeline (0x05xx) ---
/// Ring buffer is full and blocking disabled
pub const DO_LOG_ERR_BUFFER_FULL: i32 = -0x0501;
/// Pipeline stage error
pub const DO_LOG_ERR_PIPELINE_STAGE: i32 = -0x0502;

// --- Signature / Audit (0x06xx) ---
/// Signature generation failed
pub const DO_LOG_ERR_SIGN_FAILED: i32 = -0x0601;
/// Signature verification failed
pub const DO_LOG_ERR_VERIFY_FAILED: i32 = -0x0602;
/// LSN chain broken (tampering detected)
pub const DO_LOG_ERR_LSN_CHAIN_BROKEN: i32 = -0x0603;
/// Key not available for signing
pub const DO_LOG_ERR_KEY_NOT_AVAILABLE: i32 = -0x0604;

// --- Sink / IO (0x07xx) ---
/// Sink write failed
pub const DO_LOG_ERR_SINK_WRITE_FAILED: i32 = -0x0701;
/// Sink connection lost
pub const DO_LOG_ERR_SINK_CONNECTION_LOST: i32 = -0x0702;
/// WORM write failed (media error)
pub const DO_LOG_ERR_WORM_WRITE_FAILED: i32 = -0x0703;

// --- Sandbox / Security (0x08xx) ---
/// Sandbox initialization failed
pub const DO_LOG_ERR_SANDBOX_INIT_FAILED: i32 = -0x0801;
/// Sandbox policy violation (syscall blocked)
pub const DO_LOG_ERR_SANDBOX_VIOLATION: i32 = -0x0802;

// --- Resource / Quota (0x09xx) ---
/// Memory quota exceeded
pub const DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED: i32 = -0x0901;
/// CPU quota exceeded
pub const DO_LOG_ERR_QUOTA_CPU_EXCEEDED: i32 = -0x0902;

// --- Compliance (0x0Bxx) — note: 0x0A is Network ---
/// Compliance violation detected
pub const DO_LOG_ERR_COMPLIANCE_VIOLATION: i32 = -0x0B01;
/// Circular dependency detected in field requirements
pub const DO_LOG_ERR_CIRCULAR_DEPENDENCY: i32 = -0x0B02;

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
