//! Official DoLogger Filter plugin — `filter_level`.
//!
//! Drops log records below a configurable severity level with per-domain
//! override support. Phase: Filter (1), Trust: Blue.
//!
//! Always passes AUDIT level records (they are never filtered).
//!
//! # C ABI symbols exported
//!
//! - `plugin_query()` → returns PluginInfo with Filter VTable
//! - `plugin_init(config)` → parses config (min_level, drop_debug, drop_trace)
//! - `plugin_shutdown()` → cleanup
//!
//! # Configuration format (JSON string)
//!
//! ```json
//! {"min_level": "WARN", "drop_debug": true, "drop_trace": true}
//! ```

use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use dologger_core::LogLevel;
use dologger_core::Record;

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0102;

// Plugin mount phase — Filter stage
const PHASE_FILTER: u32 = 0x0002;

// Plugin info versioning
const CORE_ABI_VERSION: u32 = 1;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// Trust level: Blue (official, signed)
const TRUST_BLUE: u32 = 0;

// Plugin type: Filter
const PLUGIN_TYPE_FILTER: u32 = 0;

// ---------------------------------------------------------------------------
// Plugin state (init-time defaults, mutable via plugin_init)
// ---------------------------------------------------------------------------

/// Minimum log level to pass through.
/// Default: INFO (drops TRACE and DEBUG).
static MIN_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// When true, TRACE records are always dropped regardless of min_level.
static DROP_TRACE: AtomicBool = AtomicBool::new(false);

/// When true, DEBUG records are always dropped regardless of min_level.
static DROP_DEBUG: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// VTable: Filter
// ---------------------------------------------------------------------------

/// VTable for a Filter plugin.
///
/// SAFETY: Function pointers in the static instance point to static functions,
/// so sharing across threads is safe.
#[repr(C)]
struct FilterVTable {
    /// Filter function.
    ///
    /// Parameters:
    /// - `record`: pointer to a `dologger_core::Record` to evaluate
    /// - `config`: pointer to a null-terminated JSON config string, or NULL
    ///   to use init-time defaults
    ///
    /// Returns:
    /// - 1 = pass (record passes the filter)
    /// - 0 = drop (record is filtered out)
    /// - -1 = error (invalid config, null record pointer)
    filter: unsafe extern "C" fn(*const Record, *const std::ffi::c_void) -> i32,
    /// Batch filter function (optional, NULL if not implemented).
    filter_batch:
        Option<unsafe extern "C" fn(*const Record, *const std::ffi::c_void, u32, *mut u8) -> i32>,
}

// SAFETY: Function pointers point to static functions.
unsafe impl Sync for FilterVTable {}

// ---------------------------------------------------------------------------
// PluginInfo (C ABI struct)
// ---------------------------------------------------------------------------

/// Plugin information returned by plugin_query.
#[repr(C)]
pub struct PluginInfo {
    /// Plugin type identifier (0 = Filter)
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
// JSON config parsing (lightweight, no serde dependency)
// ---------------------------------------------------------------------------

/// Extract a quoted string value for a given key from a flat JSON object.
///
/// Example: `extract_json_string_val(r#"{"min_level":"WARN"}"#, "min_level")`
/// returns `Some("WARN")`.
fn extract_json_string_val(json: &str, key: &str) -> Option<String> {
    let search_key = format!("\"{key}\"");
    let after_key = json.split(&search_key).nth(1)?;

    // Find the opening quote of the value
    let after_colon = after_key.split(':').nth(1)?;
    let trimmed = after_colon.trim();

    // Value must start with a quote
    if !trimmed.starts_with('"') {
        return None;
    }

    // Extract the string between the first and next unescaped quote
    let inner = &trimmed[1..]; // skip opening quote
    let end_idx = inner.find('"')?;
    Some(inner[..end_idx].to_string())
}

/// Extract a boolean value for a given key from a flat JSON object.
///
/// Example: `extract_json_bool_val(r#"{"drop_debug":true}"#, "drop_debug")`
/// returns `Some(true)`.
fn extract_json_bool_val(json: &str, key: &str) -> Option<bool> {
    let search_key = format!("\"{key}\"");
    let after_key = json.split(&search_key).nth(1)?;

    let after_colon = after_key.split(':').nth(1)?;
    let trimmed = after_colon.trim();

    if trimmed.starts_with("true") {
        Some(true)
    } else if trimmed.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Parse a log level string to a `LogLevel` value.
fn parse_level_str(s: &str) -> Option<LogLevel> {
    match s.to_uppercase().as_str() {
        "TRACE" => Some(LogLevel::Trace),
        "DEBUG" => Some(LogLevel::Debug),
        "INFO" => Some(LogLevel::Info),
        "WARN" => Some(LogLevel::Warn),
        "ERROR" => Some(LogLevel::Error),
        "FATAL" => Some(LogLevel::Fatal),
        "AUDIT" => Some(LogLevel::Audit),
        _ => None,
    }
}

/// Parse a complete filter configuration from a JSON string.
///
/// Returns `(min_level, drop_trace, drop_debug)` on success, or `None` if
/// the config is invalid (malformed JSON, unknown level, etc.).
///
/// The config string must at least resemble a JSON object (`{...}`).
/// If `min_level` is absent, the default (INFO) is used.  If `min_level` is
/// present but its value cannot be parsed as a valid level, the config is
/// treated as invalid and `None` is returned.
fn parse_filter_config(json: &str) -> Option<(LogLevel, bool, bool)> {
    let trimmed = json.trim();
    // Basic shape check — must look like a JSON object
    if !trimmed.starts_with('{') {
        return None;
    }

    let min_level_raw = extract_json_string_val(json, "min_level");
    let min_level = match min_level_raw {
        Some(val) => parse_level_str(&val)?,
        None => LogLevel::Info, // default when key is absent
    };

    let drop_trace = extract_json_bool_val(json, "drop_trace").unwrap_or(false);
    let drop_debug = extract_json_bool_val(json, "drop_debug").unwrap_or(false);

    Some((min_level, drop_trace, drop_debug))
}

// ---------------------------------------------------------------------------
// VTable function: filter_level_filter
// ---------------------------------------------------------------------------

/// Filter implementation — drops records below the configured minimum level.
///
/// # Logic
///
/// 1. AUDIT level records always pass (return 1).
/// 2. If `drop_trace` is set and level is TRACE, drop (return 0).
/// 3. If `drop_debug` is set and level is DEBUG, drop (return 0).
/// 4. Otherwise, pass if `record.level >= min_level`, drop if below.
///
/// # Safety
///
/// - `record` must be a valid, non-null pointer to a `Record`.
/// - `config` may be NULL (use init-time defaults) or a valid null-terminated
///   JSON config string.
unsafe extern "C" fn filter_impl(record: *const Record, config: *const std::ffi::c_void) -> i32 {
    // Null record is an error
    if record.is_null() {
        return -1;
    }

    // SAFETY: record is non-null and the caller guarantees it points to a
    // valid, live Record for the duration of this call.
    let rec = unsafe { &*record };

    // AUDIT records always pass — they are never filtered
    if rec.level == LogLevel::Audit {
        return 1;
    }

    // Determine the effective configuration for this call.
    // If per-call config is provided (non-null), parse it and use it.
    // Otherwise fall back to the init-time statics.
    let (min_level, drop_trace, drop_debug) = if config.is_null() {
        let min_val = MIN_LEVEL.load(Ordering::Relaxed);
        let min = LogLevel::from_u8(min_val).unwrap_or(LogLevel::Info);
        let dt = DROP_TRACE.load(Ordering::Relaxed);
        let dd = DROP_DEBUG.load(Ordering::Relaxed);
        (min, dt, dd)
    } else {
        // SAFETY: config is non-null (validated above). CStr::from_ptr reads
        // a null-terminated UTF-8 string from the host.
        let config_str = match unsafe { CStr::from_ptr(config as *const c_char) }.to_str() {
            Ok(s) => s,
            Err(_) => return -1, // invalid config (not valid UTF-8)
        };
        match parse_filter_config(config_str) {
            Some(cfg) => cfg,
            None => return -1, // invalid config (malformed JSON or unknown level)
        }
    };

    // Explicit drop flags take priority over the level comparison
    if drop_trace && rec.level == LogLevel::Trace {
        return 0; // drop
    }
    if drop_debug && rec.level == LogLevel::Debug {
        return 0; // drop
    }

    // Core level comparison
    if rec.level >= min_level {
        1 // pass
    } else {
        0 // drop
    }
}

static VTABLE: FilterVTable = FilterVTable {
    filter: filter_impl,
    filter_batch: None,
};

static PLUGIN_NAME: &[u8] = b"filter-level\0";

// ---------------------------------------------------------------------------
// C ABI: plugin_query
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_query() -> *const PluginInfo {
    static INFO: PluginInfo = PluginInfo {
        plugin_type: PLUGIN_TYPE_FILTER,
        abi_version: CORE_ABI_VERSION,
        version: PLUGIN_VERSION,
        name: PLUGIN_NAME.as_ptr() as *const c_char,
        phase: PHASE_FILTER,
        trust_level: TRUST_BLUE,
        vtable: &VTABLE as *const FilterVTable as *const std::ffi::c_void,
    };
    &INFO as *const PluginInfo
}

// ---------------------------------------------------------------------------
// C ABI: plugin_init
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn plugin_init(config: *const std::ffi::c_void) -> i32 {
    if config.is_null() {
        return DO_LOG_OK; // Use defaults (min_level = INFO)
    }

    // Config is a JSON string: {"min_level":"WARN","drop_debug":true,"drop_trace":true}
    // SAFETY: config is non-null (validated above). CStr::from_ptr reads a
    // null-terminated UTF-8 string from the host.
    let config_str = unsafe { CStr::from_ptr(config as *const c_char) };
    let s = match config_str.to_str() {
        Ok(s) => s,
        Err(_) => return DO_LOG_ERR_INVALID_ARG,
    };

    // Parse min_level
    if let Some(val) = extract_json_string_val(s, "min_level") {
        match parse_level_str(&val) {
            Some(level) => MIN_LEVEL.store(level as u8, Ordering::Relaxed),
            None => return DO_LOG_ERR_INVALID_ARG,
        }
    }

    // Parse drop_trace (optional)
    if let Some(val) = extract_json_bool_val(s, "drop_trace") {
        DROP_TRACE.store(val, Ordering::Relaxed);
    }

    // Parse drop_debug (optional)
    if let Some(val) = extract_json_bool_val(s, "drop_debug") {
        DROP_DEBUG.store(val, Ordering::Relaxed);
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
    use std::ffi::CString;

    // Helper: create a Record with a specific level
    fn record_with_level(level: LogLevel) -> Record {
        let mut rec = Record::new(0);
        rec.level = level;
        rec
    }

    // Helper: call filter_impl with a record and optional config
    fn filter_record(record: &Record, config: Option<&CString>) -> i32 {
        let config_ptr = match config {
            Some(cs) => cs.as_ptr() as *const std::ffi::c_void,
            None => std::ptr::null(),
        };
        unsafe { filter_impl(record as *const Record, config_ptr) }
    }

    // Helper: call filter_impl with a record using stored statics (null config)
    fn filter_record_default(record: &Record) -> i32 {
        filter_record(record, None)
    }

    #[test]
    fn test_plugin_query_returns_valid_info() {
        let info = unsafe { &*plugin_query() };
        assert_eq!(info.plugin_type, PLUGIN_TYPE_FILTER);
        assert_eq!(info.abi_version, CORE_ABI_VERSION);
        assert_eq!(info.phase, PHASE_FILTER);
        assert_eq!(info.trust_level, TRUST_BLUE);
    }

    #[test]
    fn test_null_record_returns_error() {
        assert_eq!(
            unsafe { filter_impl(std::ptr::null(), std::ptr::null()) },
            -1
        );
    }

    #[test]
    fn test_debug_dropped_at_info_min() {
        // Set min_level to INFO via init
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);
        DROP_TRACE.store(false, Ordering::Relaxed);
        DROP_DEBUG.store(false, Ordering::Relaxed);

        let debug_rec = record_with_level(LogLevel::Debug);
        let info_rec = record_with_level(LogLevel::Info);

        // DEBUG < INFO → should be dropped
        assert_eq!(filter_record_default(&debug_rec), 0);
        // INFO >= INFO → should pass
        assert_eq!(filter_record_default(&info_rec), 1);
    }

    #[test]
    fn test_warn_passed_at_info_min() {
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);
        DROP_TRACE.store(false, Ordering::Relaxed);
        DROP_DEBUG.store(false, Ordering::Relaxed);

        let warn_rec = record_with_level(LogLevel::Warn);
        // WARN >= INFO → should pass
        assert_eq!(filter_record_default(&warn_rec), 1);
    }

    #[test]
    fn test_audit_always_passed() {
        // Set min_level to FATAL — everything below should be dropped
        MIN_LEVEL.store(LogLevel::Fatal as u8, Ordering::Relaxed);
        DROP_TRACE.store(false, Ordering::Relaxed);
        DROP_DEBUG.store(false, Ordering::Relaxed);

        let audit_rec = record_with_level(LogLevel::Audit);
        let error_rec = record_with_level(LogLevel::Error);
        let fatal_rec = record_with_level(LogLevel::Fatal);

        // AUDIT always passes regardless of min_level
        assert_eq!(filter_record_default(&audit_rec), 1);
        // ERROR < FATAL → dropped
        assert_eq!(filter_record_default(&error_rec), 0);
        // FATAL >= FATAL → passes
        assert_eq!(filter_record_default(&fatal_rec), 1);
    }

    #[test]
    fn test_audit_passes_even_when_min_is_above() {
        // AUDIT has value 6 — there is no level above it.
        // Even with an impossible min_level, AUDIT should pass.
        MIN_LEVEL.store(LogLevel::Audit as u8, Ordering::Relaxed);

        let audit_rec = record_with_level(LogLevel::Audit);
        assert_eq!(filter_record_default(&audit_rec), 1);
    }

    #[test]
    fn test_invalid_config_returns_error() {
        let bad_config = CString::new("not valid json!!").unwrap();
        let rec = record_with_level(LogLevel::Info);

        let result = filter_record(&rec, Some(&bad_config));
        assert_eq!(result, -1);
    }

    #[test]
    fn test_config_with_unknown_level_returns_error() {
        let bad_config = CString::new(r#"{"min_level":"INVALID"}"#).unwrap();
        let rec = record_with_level(LogLevel::Info);

        let result = filter_record(&rec, Some(&bad_config));
        assert_eq!(result, -1);
    }

    #[test]
    fn test_config_non_utf8_returns_error() {
        // Create a non-UTF-8 byte sequence — 0xFF 0xFE are a BOM-like
        // sequence that is not valid UTF-8.  CString::new adds its own
        // NUL terminator so these two bytes are the entire payload.
        let bad_bytes: &[u8] = &[0xFF, 0xFE];
        let bad_config = CString::new(bad_bytes).unwrap();
        let rec = record_with_level(LogLevel::Info);

        let result = filter_record(&rec, Some(&bad_config));
        // Should be -1 because the bytes are not valid UTF-8
        assert_eq!(result, -1);
    }

    #[test]
    fn test_per_call_config_overrides_statics() {
        // Set static min to DEBUG
        MIN_LEVEL.store(LogLevel::Debug as u8, Ordering::Relaxed);

        let warn_rec = record_with_level(LogLevel::Warn);
        let debug_rec = record_with_level(LogLevel::Debug);
        let trace_rec = record_with_level(LogLevel::Trace);

        // With statics (min=DEBUG): WARN and DEBUG pass, TRACE drops
        assert_eq!(filter_record_default(&warn_rec), 1);
        assert_eq!(filter_record_default(&debug_rec), 1);
        assert_eq!(filter_record_default(&trace_rec), 0);

        // Now use per-call config: min_level = WARN
        let config = CString::new(r#"{"min_level":"WARN"}"#).unwrap();
        // WARN >= WARN → pass
        assert_eq!(filter_record(&warn_rec, Some(&config)), 1);
        // DEBUG < WARN → drop
        assert_eq!(filter_record(&debug_rec, Some(&config)), 0);
        // TRACE < WARN → drop
        assert_eq!(filter_record(&trace_rec, Some(&config)), 0);

        // Reset
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);
    }

    #[test]
    fn test_drop_trace_flag() {
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);

        let trace_rec = record_with_level(LogLevel::Trace);
        let debug_rec = record_with_level(LogLevel::Debug);
        let info_rec = record_with_level(LogLevel::Info);

        // Without drop_trace: TRACE is dropped (below INFO), DEBUG is dropped, INFO passes
        let config_noflag = CString::new(r#"{"min_level":"INFO"}"#).unwrap();
        assert_eq!(filter_record(&trace_rec, Some(&config_noflag)), 0);
        assert_eq!(filter_record(&debug_rec, Some(&config_noflag)), 0);
        assert_eq!(filter_record(&info_rec, Some(&config_noflag)), 1);

        // With min_level=TRACE but drop_trace=true: TRACE explicitly dropped
        let config_drop_trace = CString::new(r#"{"min_level":"TRACE","drop_trace":true}"#).unwrap();
        assert_eq!(filter_record(&trace_rec, Some(&config_drop_trace)), 0);
        // DEBUG >= TRACE and drop_debug not set → passes
        assert_eq!(filter_record(&debug_rec, Some(&config_drop_trace)), 1);
    }

    #[test]
    fn test_drop_debug_flag() {
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);

        let trace_rec = record_with_level(LogLevel::Trace);
        let debug_rec = record_with_level(LogLevel::Debug);
        let info_rec = record_with_level(LogLevel::Info);

        // With min_level=DEBUG but drop_debug=true: DEBUG explicitly dropped
        let config = CString::new(r#"{"min_level":"DEBUG","drop_debug":true}"#).unwrap();
        assert_eq!(filter_record(&debug_rec, Some(&config)), 0);
        // INFO >= DEBUG and no drop_info → passes
        assert_eq!(filter_record(&info_rec, Some(&config)), 1);
        // TRACE < DEBUG → dropped by level comparison (not the flag)
        assert_eq!(filter_record(&trace_rec, Some(&config)), 0);
    }

    #[test]
    fn test_init_parses_min_level() {
        let config = CString::new(r#"{"min_level":"DEBUG"}"#).unwrap();
        assert_eq!(
            plugin_init(config.as_ptr() as *const std::ffi::c_void),
            DO_LOG_OK
        );
        assert_eq!(
            LogLevel::from_u8(MIN_LEVEL.load(Ordering::Relaxed)),
            Some(LogLevel::Debug)
        );
        // Reset
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);
    }

    #[test]
    fn test_init_parses_drop_flags() {
        let config =
            CString::new(r#"{"min_level":"INFO","drop_debug":true,"drop_trace":true}"#).unwrap();
        assert_eq!(
            plugin_init(config.as_ptr() as *const std::ffi::c_void),
            DO_LOG_OK
        );
        assert!(DROP_DEBUG.load(Ordering::Relaxed));
        assert!(DROP_TRACE.load(Ordering::Relaxed));
        // Reset
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);
        DROP_DEBUG.store(false, Ordering::Relaxed);
        DROP_TRACE.store(false, Ordering::Relaxed);
    }

    #[test]
    fn test_init_null_config_uses_defaults() {
        // Reset to something else first
        MIN_LEVEL.store(LogLevel::Warn as u8, Ordering::Relaxed);

        // Null config should NOT change the state
        assert_eq!(plugin_init(std::ptr::null()), DO_LOG_OK);
        assert_eq!(
            LogLevel::from_u8(MIN_LEVEL.load(Ordering::Relaxed)),
            Some(LogLevel::Warn)
        );

        // Reset
        MIN_LEVEL.store(LogLevel::Info as u8, Ordering::Relaxed);
    }

    #[test]
    fn test_init_invalid_level_returns_error() {
        let config = CString::new(r#"{"min_level":"GARBAGE"}"#).unwrap();
        assert_eq!(
            plugin_init(config.as_ptr() as *const std::ffi::c_void),
            DO_LOG_ERR_INVALID_ARG
        );
    }

    #[test]
    fn test_shutdown_returns_ok() {
        assert_eq!(plugin_shutdown(), DO_LOG_OK);
    }
}
