//! Official DoLogger Formatter plugin — `formatter_json`.
//!
//! Serializes log records to structured JSON with configurable field inclusion.
//! Phase: Formatting (5), Trust: Blue.
//!
//! # Bundle member
//!
//! This crate provides the plugin LOGIC (VTable + metadata) as an rlib. It is
//! aggregated by the `dologger-official-plugins` bundle crate, which exposes
//! the C ABI (`plugin_query_multi` / `plugin_init` / `plugin_shutdown`) for
//! all official plugins in ONE dynamic library.

use std::ffi::{c_char, CStr};
use std::sync::Mutex;

use dologger_core::ffi::DologgerPluginInfo;
use dologger_core::Record;
use serde_json::Value;

// Re-use core error codes
const DO_LOG_OK: i32 = 0;
const DO_LOG_ERR_INVALID_ARG: i32 = -0x0102;

// Plugin mount phase — Formatting stage
const PHASE_FORMATTING: u32 = 0x0010;

// Plugin info versioning — abi_version MUST match the core's declared ABI
// (0.1.0); the host validates it when the bundle is loaded.
const CORE_ABI_VERSION: u32 = dologger_core::plugin::CORE_ABI_VERSION;
const PLUGIN_VERSION: u32 = 1; // 0.1.0 (packed major.minor.patch)

// ---------------------------------------------------------------------------
// VTable: Formatter
// ---------------------------------------------------------------------------

/// VTable for a Formatter plugin.
///
/// SAFETY: Function pointers in the static instance point to static functions,
/// so sharing across threads is safe.
#[repr(C)]
struct FormatterVTable {
    /// Format a single record into the caller-provided output buffer.
    ///
    /// Parameters:
    /// - `record`: pointer to a `dologger_core::Record` to serialize
    /// - `output`: pre-allocated output buffer (UTF-8)
    /// - `output_len`: on input, max bytes available; on output, actual bytes written
    ///
    /// Returns 0 on success, -1 on error.
    format: unsafe extern "C" fn(*const Record, *mut u8, *mut u32) -> i32,
    /// Flush any buffered output (optional, NULL if not implemented).
    flush: Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> i32>,
}

// SAFETY: Function pointers point to static functions.
unsafe impl Sync for FormatterVTable {}

// ---------------------------------------------------------------------------
// Plugin info — canonical `dologger_plugin_info_t` (see core/src/ffi.rs).
// Registered by the official bundle via `plugin_query_multi`.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Helper: check if a u128 timestamp is populated
// ---------------------------------------------------------------------------

fn is_zero_u128(hi: u64, lo: u64) -> bool {
    hi == 0 && lo == 0
}

// ---------------------------------------------------------------------------
// Formatter configuration — parsed from the `init` config JSON
// ---------------------------------------------------------------------------

/// Timestamp rendering mode selected via the `timestamp_format` config key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimestampFormat {
    /// Unix seconds.nanoseconds (default — matches historical output).
    Unix,
    /// Unix milliseconds (integer, rounded down).
    UnixMs,
    /// ISO 8601 UTC wall-clock (RFC 3339, `...Z`).
    Iso8601,
}

/// Formatter options parsed from the `init` config JSON.
///
/// Thread-safe static shared with `format_impl`: `init` runs once before any
/// `format` call, so this is set-then-read-only and the `Mutex` guard is held
/// only for the duration of a single `format` call.
#[derive(Debug, Clone, Copy)]
struct JsonConfig {
    /// Pretty-print the JSON output (multi-line, indented).
    pretty: bool,
    /// Include Ring-3 extension data (`ext_data`). Default false: Ring-3 is
    /// arbitrary extension data, so it is opt-in for security-conscious sinks.
    include_ring3: bool,
    /// Include Ring-1 source location fields (file/function/line/column).
    /// Default true: source location is core diagnostic information.
    include_source: bool,
    /// How to render the 128-bit `timestamp` field.
    timestamp_format: TimestampFormat,
}

impl JsonConfig {
    /// Default options. `const` so it can initialise the [`CONFIG`] static.
    const fn default_config() -> Self {
        Self {
            pretty: false,
            include_ring3: false,
            include_source: true,
            timestamp_format: TimestampFormat::Unix,
        }
    }
}

impl Default for JsonConfig {
    fn default() -> Self {
        Self::default_config()
    }
}

/// Parsed formatter config, written once by [`init`] and read by `format_impl`.
static CONFIG: Mutex<JsonConfig> = Mutex::new(JsonConfig::default_config());

// ---------------------------------------------------------------------------
// Helper: convert days-since-epoch to a civil (year, month, day) date
// ---------------------------------------------------------------------------

/// Convert days since 1970-01-01 to a (year, month, day) civil date.
///
/// Howard Hinnant's `civil_from_days` algorithm (public domain); correct for
/// the full range of `i64` days. Kept dependency-free so ISO 8601 timestamp
/// rendering needs no date-time crate.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Render a 128-bit `{seconds, nanos}` timestamp per the selected format.
fn render_timestamp(hi: u64, lo: u64, fmt: TimestampFormat) -> String {
    match fmt {
        TimestampFormat::Unix => format!("{}.{:09}", hi, lo),
        TimestampFormat::UnixMs => {
            format!("{}", hi.saturating_mul(1000) + lo / 1_000_000)
        }
        TimestampFormat::Iso8601 => {
            let sod = hi % 86_400;
            let (h, mi, s) = (sod / 3600, (sod % 3600) / 60, sod % 60);
            let (y, mo, d) = civil_from_days((hi / 86_400) as i64);
            format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{:09}Z", lo)
        }
    }
}

// ---------------------------------------------------------------------------
// VTable function: formatter_json_format
// ---------------------------------------------------------------------------

/// Serialize a Record to JSON and write into the output buffer.
///
/// Only non-empty / non-zero fields are included to keep the output compact.
/// Timestamp fields (128-bit) are converted to human-readable strings.
///
/// # Safety
///
/// - `record` must be a valid, non-null pointer to a `Record`
/// - `output` must be a valid, non-null pointer to a buffer of at least
///   `*output_len` writable bytes
/// - `output_len` must be a valid, non-null pointer to a `u32` containing
///   the maximum buffer size on input; on output it will hold the number
///   of bytes actually written
unsafe extern "C" fn format_impl(
    record: *const Record,
    output: *mut u8,
    output_len: *mut u32,
) -> i32 {
    // Null-pointer guards
    if record.is_null() || output.is_null() || output_len.is_null() {
        return -1;
    }

    // SAFETY: All three pointers validated non-null above.
    // The caller guarantees the pointers are valid for the duration of this call.
    let max_len = unsafe { *output_len } as usize;
    let rec = unsafe { &*record };

    // Read the formatter config once (set by `init`); the Mutex guard is
    // released as soon as the config is copied out.
    let cfg = *CONFIG.lock().unwrap();

    let json_value = match record_to_json(rec, &cfg) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let json_bytes = if cfg.pretty {
        serde_json::to_vec_pretty(&json_value)
    } else {
        serde_json::to_vec(&json_value)
    };
    let json_bytes = match json_bytes {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let write_len = json_bytes.len().min(max_len);

    // SAFETY: output is a valid buffer of at least `max_len` bytes (caller
    // guarantee). json_bytes is a local Vec<u8> with at least `write_len`
    // bytes. copy_nonoverlapping copies exactly `write_len` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(json_bytes.as_ptr(), output, write_len);
        *output_len = write_len as u32;
    }

    DO_LOG_OK
}

// ---------------------------------------------------------------------------
// JSON serialization: Record → serde_json::Value
// ---------------------------------------------------------------------------

/// Build a `serde_json::Value` from a `Record`, including only non-empty /
/// non-zero fields, honouring the formatter [`JsonConfig`]. Returns an error
/// if any field access fails.
fn record_to_json(rec: &Record, cfg: &JsonConfig) -> Result<Value, ()> {
    let mut map = serde_json::Map::new();

    // ── Ring 0: Kernel-core fields ──
    if !is_zero_u128(rec.id.hi, rec.id.lo) {
        map.insert(
            "id".to_string(),
            Value::String(format!("{:016x}{:016x}", rec.id.hi, rec.id.lo)),
        );
    }
    if !is_zero_u128(rec.timestamp.hi, rec.timestamp.lo) {
        map.insert(
            "timestamp".to_string(),
            Value::String(render_timestamp(
                rec.timestamp.hi,
                rec.timestamp.lo,
                cfg.timestamp_format,
            )),
        );
    }
    if rec.origin_lsn != 0 {
        map.insert("origin_lsn".to_string(), Value::from(rec.origin_lsn));
    }

    // ── Ring 1: Level + Message ──
    map.insert(
        "level".to_string(),
        Value::String(rec.level.to_str().to_string()),
    );

    let msg = rec.message.as_str();
    if !msg.is_empty() {
        map.insert("message".to_string(), Value::String(msg.to_string()));
    }

    // ── Ring 1: Source location (configurable via `source`) ──
    if cfg.include_source {
        let sf = rec.source_file.as_str();
        if !sf.is_empty() {
            map.insert("source_file".to_string(), Value::String(sf.to_string()));
        }
        let sfn = rec.source_function.as_str();
        if !sfn.is_empty() {
            map.insert(
                "source_function".to_string(),
                Value::String(sfn.to_string()),
            );
        }
        if rec.source_line != 0 {
            map.insert("source_line".to_string(), Value::from(rec.source_line));
        }
        if rec.source_column != 0 {
            map.insert("source_column".to_string(), Value::from(rec.source_column));
        }
    }

    // ── Ring 1: Thread / Process ──
    if rec.thread_id != 0 {
        map.insert("thread_id".to_string(), Value::from(rec.thread_id));
    }
    let tn = rec.thread_name.as_str();
    if !tn.is_empty() {
        map.insert("thread_name".to_string(), Value::String(tn.to_string()));
    }
    if rec.process_id != 0 {
        map.insert("process_id".to_string(), Value::from(rec.process_id));
    }
    let pn = rec.process_name.as_str();
    if !pn.is_empty() {
        map.insert("process_name".to_string(), Value::String(pn.to_string()));
    }

    // ── Ring 1: Host / Container ──
    let hn = rec.host_name.as_str();
    if !hn.is_empty() {
        map.insert("host_name".to_string(), Value::String(hn.to_string()));
    }
    let ci = rec.container_id.as_str();
    if !ci.is_empty() {
        map.insert("container_id".to_string(), Value::String(ci.to_string()));
    }

    // ── Ring 1: Application ──
    let an = rec.app_name.as_str();
    if !an.is_empty() {
        map.insert("app_name".to_string(), Value::String(an.to_string()));
    }
    let av = rec.app_version.as_str();
    if !av.is_empty() {
        map.insert("app_version".to_string(), Value::String(av.to_string()));
    }
    let env = rec.environment.as_str();
    if !env.is_empty() {
        map.insert("environment".to_string(), Value::String(env.to_string()));
    }

    // ── Ring 1: User / Session ──
    let uid = rec.user_id.as_str();
    if !uid.is_empty() {
        map.insert("user_id".to_string(), Value::String(uid.to_string()));
    }
    let sid = rec.session_id.as_str();
    if !sid.is_empty() {
        map.insert("session_id".to_string(), Value::String(sid.to_string()));
    }

    // ── Ring 1: Distributed Tracing (W3C / OpenTelemetry) ──
    let rid = rec.request_id.as_str();
    if !rid.is_empty() {
        map.insert("request_id".to_string(), Value::String(rid.to_string()));
    }
    let tid = rec.trace_id.as_str();
    if !tid.is_empty() {
        map.insert("trace_id".to_string(), Value::String(tid.to_string()));
    }
    let spid = rec.span_id.as_str();
    if !spid.is_empty() {
        map.insert("span_id".to_string(), Value::String(spid.to_string()));
    }
    if rec.coroutine_id != 0 {
        map.insert("coroutine_id".to_string(), Value::from(rec.coroutine_id));
    }

    // ── Ring 1: Exception ──
    let et = rec.exception_type.as_str();
    if !et.is_empty() {
        map.insert("exception_type".to_string(), Value::String(et.to_string()));
    }
    let em = rec.exception_message.as_str();
    if !em.is_empty() {
        map.insert(
            "exception_message".to_string(),
            Value::String(em.to_string()),
        );
    }
    let est = rec.exception_stacktrace.as_str();
    if !est.is_empty() {
        map.insert(
            "exception_stacktrace".to_string(),
            Value::String(est.to_string()),
        );
    }
    if rec.exception_code != 0 {
        map.insert(
            "exception_code".to_string(),
            Value::from(rec.exception_code),
        );
    }

    // ── Ring 1: Labels ──
    let lbl = rec.labels.as_str();
    if !lbl.is_empty() {
        map.insert("labels".to_string(), Value::String(lbl.to_string()));
    }

    // ── Ring 1: Security ──
    if rec.lsn != 0 {
        map.insert("lsn".to_string(), Value::from(rec.lsn));
    }
    if rec.security_gap {
        map.insert("security_gap".to_string(), Value::Bool(true));
    }
    let at = rec.audit_tags.as_str();
    if !at.is_empty() {
        map.insert("audit_tags".to_string(), Value::String(at.to_string()));
    }

    // ── Ring 3: Extension data (configurable via `include_ring3`) ──
    if cfg.include_ring3 {
        let ed = rec.ext_data.as_str();
        if !ed.is_empty() {
            map.insert("ext_data".to_string(), Value::String(ed.to_string()));
        }
    }

    Ok(Value::Object(map))
}

static VTABLE: FormatterVTable = FormatterVTable {
    format: format_impl,
    flush: None,
};

static PLUGIN_NAME: &[u8] = b"formatter-json\0";

// ---------------------------------------------------------------------------
// Plugin registry entry — aggregated by the official bundle.
// ---------------------------------------------------------------------------

/// Canonical plugin info for this crate, as it appears in the bundle registry.
pub static INFO: DologgerPluginInfo = DologgerPluginInfo {
    name: PLUGIN_NAME.as_ptr() as *const c_char,
    version: PLUGIN_VERSION,
    abi_version: CORE_ABI_VERSION,
    phase: PHASE_FORMATTING,
    vtable: &VTABLE as *const FormatterVTable as *const std::ffi::c_void,
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
        // Null config ⇒ "use defaults": reset to the initial values.
        *CONFIG.lock().unwrap() = JsonConfig::default();
        return DO_LOG_OK;
    }

    // Config is a JSON string: {"pretty":false,"include_ring3":false,...}
    // SAFETY: config validated non-null above. CStr::from_ptr reads a
    // null-terminated UTF-8 string provided by the host.
    let config_str = unsafe { CStr::from_ptr(config as *const c_char) };
    let Ok(s) = config_str.to_str() else {
        return DO_LOG_ERR_INVALID_ARG; // Not valid UTF-8
    };
    let Ok(value) = serde_json::from_str::<Value>(s) else {
        return DO_LOG_ERR_INVALID_ARG; // Not valid JSON
    };
    let Some(obj) = value.as_object() else {
        return DO_LOG_ERR_INVALID_ARG; // Config must be a JSON object
    };

    let mut cfg = JsonConfig::default();
    if let Some(p) = obj.get("pretty").and_then(Value::as_bool) {
        cfg.pretty = p;
    }
    if let Some(r) = obj.get("include_ring3").and_then(Value::as_bool) {
        cfg.include_ring3 = r;
    }
    if let Some(s) = obj.get("source").and_then(Value::as_bool) {
        cfg.include_source = s;
    }
    if let Some(f) = obj.get("timestamp_format").and_then(Value::as_str) {
        cfg.timestamp_format = match f {
            "unix_ms" => TimestampFormat::UnixMs,
            "iso8601" => TimestampFormat::Iso8601,
            _ => TimestampFormat::Unix, // "unix" or unknown → default
        };
    }

    *CONFIG.lock().unwrap() = cfg;
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
    use dologger_core::Record;
    use std::ffi::CString;

    #[test]
    fn test_plugin_info_returns_valid_entry() {
        let info = plugin_info();
        assert_eq!(info.abi_version, CORE_ABI_VERSION);
        assert_eq!(info.phase, PHASE_FORMATTING);
        assert_eq!(
            info.vtable,
            &VTABLE as *const FormatterVTable as *const std::ffi::c_void
        );
        let name = unsafe { CStr::from_ptr(info.name) }.to_str().unwrap();
        assert_eq!(name, "formatter-json");
    }

    #[test]
    fn test_init_with_null_config() {
        assert_eq!(init(std::ptr::null()), DO_LOG_OK);
    }

    #[test]
    fn test_shutdown_returns_ok() {
        assert_eq!(shutdown(), DO_LOG_OK);
    }

    #[test]
    fn test_init_parses_config_and_affects_output() {
        // Reset to defaults first, then apply a full non-default config.
        init(std::ptr::null());
        let cfg = CString::new(
            r#"{"pretty":true,"include_ring3":true,"source":false,"timestamp_format":"unix_ms"}"#,
        )
        .unwrap();
        assert_eq!(init(cfg.as_ptr() as *const std::ffi::c_void), DO_LOG_OK);

        let mut record = Record::new(0);
        record.timestamp = dologger_core::ffi::dologger_uint128_t {
            hi: 1_700_000_000,
            lo: 500_000_000,
        };
        record.level = dologger_core::LogLevel::Warn;
        record.message.set("cfg");
        record.source_file.set("main.rs");
        record.source_line = 7;
        record.ext_data.set(r#"{"tenant":"acme"}"#);

        let mut output_buf = vec![0u8; 8192];
        let mut output_len: u32 = output_buf.len() as u32;
        let result = unsafe { format_impl(&record, output_buf.as_mut_ptr(), &mut output_len) };
        assert_eq!(result, DO_LOG_OK);
        let json_str =
            std::str::from_utf8(&output_buf[..output_len as usize]).expect("valid UTF-8");

        // pretty ⇒ multi-line output with newlines.
        assert!(
            json_str.contains('\n'),
            "expected pretty output, got: {json_str}"
        );

        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");
        let obj = parsed.as_object().unwrap();
        // include_ring3=true ⇒ ext_data present.
        assert_eq!(obj["ext_data"], r#"{"tenant":"acme"}"#);
        // timestamp_format=unix_ms ⇒ integer millis (1700000000.5s → 1700000000500ms).
        assert_eq!(obj["timestamp"], "1700000000500");
        // source=false ⇒ source fields absent.
        assert!(!obj.contains_key("source_file"));
        assert!(!obj.contains_key("source_line"));

        // Reset so later tests see defaults.
        init(std::ptr::null());
    }

    #[test]
    fn test_init_rejects_invalid_config() {
        let invalid = CString::new("not json").unwrap();
        assert_eq!(
            init(invalid.as_ptr() as *const std::ffi::c_void),
            DO_LOG_ERR_INVALID_ARG
        );

        // A non-object JSON value is also invalid.
        let arr = CString::new("[1,2,3]").unwrap();
        assert_eq!(
            init(arr.as_ptr() as *const std::ffi::c_void),
            DO_LOG_ERR_INVALID_ARG
        );

        init(std::ptr::null()); // reset
    }

    #[test]
    fn test_render_timestamp_modes() {
        let (hi, lo) = (1_700_000_000u64, 123_456_789u64);
        assert_eq!(
            render_timestamp(hi, lo, TimestampFormat::Unix),
            "1700000000.123456789"
        );
        assert_eq!(
            render_timestamp(hi, lo, TimestampFormat::UnixMs),
            "1700000000123"
        );
        // 1970-01-01 00:00:00 UTC → ISO 8601.
        assert_eq!(
            render_timestamp(0, 0, TimestampFormat::Iso8601),
            "1970-01-01T00:00:00.000000000Z"
        );
        // 2000-01-01T17:54:56Z = 946749296s (verified via `date`); nanos 7.
        assert_eq!(
            render_timestamp(946_749_296, 7, TimestampFormat::Iso8601),
            "2000-01-01T17:54:56.000000007Z"
        );
    }

    #[test]
    fn test_civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1)); // verified: 10957 days
        assert_eq!(civil_from_days(19_534), (2023, 6, 26)); // verified: 19534 days
    }

    #[test]
    fn test_format_null_pointers() {
        // All null → error
        assert_eq!(
            unsafe { format_impl(std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut()) },
            -1
        );

        let mut output_buf = [0u8; 256];
        let mut output_len: u32 = 256;
        // Null record → error
        assert_eq!(
            unsafe { format_impl(std::ptr::null(), output_buf.as_mut_ptr(), &mut output_len) },
            -1
        );
    }

    #[test]
    fn test_format_empty_record() {
        init(std::ptr::null()); // reset config to defaults
        let record = Record::new(0);
        let mut output_buf = vec![0u8; 4096];
        let mut output_len: u32 = output_buf.len() as u32;

        let result = unsafe { format_impl(&record, output_buf.as_mut_ptr(), &mut output_len) };
        assert_eq!(result, DO_LOG_OK);
        assert!(output_len > 0);

        let json_str =
            std::str::from_utf8(&output_buf[..output_len as usize]).expect("valid UTF-8");
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");

        // An empty record should have at minimum the "level" field
        assert!(parsed.is_object());
        let obj = parsed.as_object().unwrap();
        assert!(obj.contains_key("level"));
        assert_eq!(obj["level"], "INFO");
    }

    #[test]
    fn test_format_populated_record() {
        init(std::ptr::null()); // reset config to defaults
        let mut record = Record::new(0);

        // Set some fields
        record.id = dologger_core::ffi::dologger_uint128_t { hi: 1, lo: 2 };
        record.timestamp = dologger_core::ffi::dologger_uint128_t {
            hi: 1700000000,
            lo: 123456789,
        };
        record.level = dologger_core::LogLevel::Warn;
        record.message.set("Test message");
        record.source_file.set("main.rs");
        record.source_line = 42;
        record.thread_id = 12345;
        record.process_id = 999;
        record.host_name.set("test-host");
        record.app_name.set("dologger-test");
        record.environment.set("test");
        record.user_id.set("user-001");
        record.lsn = 100;
        record.labels.set(r#"{"key":"value"}"#);

        let mut output_buf = vec![0u8; 4096];
        let mut output_len: u32 = output_buf.len() as u32;

        let result = unsafe { format_impl(&record, output_buf.as_mut_ptr(), &mut output_len) };
        assert_eq!(result, DO_LOG_OK);
        assert!(output_len > 0);

        let json_str =
            std::str::from_utf8(&output_buf[..output_len as usize]).expect("valid UTF-8");
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");

        let obj = parsed.as_object().unwrap();

        // Verify key fields are present
        assert_eq!(obj["level"], "WARN");
        assert_eq!(obj["message"], "Test message");
        assert_eq!(obj["source_file"], "main.rs");
        assert_eq!(obj["source_line"], 42);
        assert_eq!(obj["thread_id"], 12345);
        assert_eq!(obj["process_id"], 999);
        assert_eq!(obj["host_name"], "test-host");
        assert_eq!(obj["app_name"], "dologger-test");
        assert_eq!(obj["environment"], "test");
        assert_eq!(obj["user_id"], "user-001");
        assert_eq!(obj["lsn"], 100);
        assert_eq!(obj["labels"], r#"{"key":"value"}"#);
        assert_eq!(obj["id"], "00000000000000010000000000000002");
        assert_eq!(obj["timestamp"], "1700000000.123456789");
    }

    #[test]
    fn test_format_respects_output_buffer_limit() {
        init(std::ptr::null()); // reset config to defaults
        let mut record = Record::new(0);
        record
            .message
            .set("A very long message that exceeds the tiny buffer we provide");
        record.level = dologger_core::LogLevel::Error;

        // Provide a very small buffer — only 32 bytes
        let mut output_buf = vec![0u8; 32];
        let mut output_len: u32 = output_buf.len() as u32;

        let result = unsafe { format_impl(&record, output_buf.as_mut_ptr(), &mut output_len) };
        assert_eq!(result, DO_LOG_OK);
        // Should not overflow — actual bytes written must be ≤ 32
        assert!(output_len <= 32);
        assert!(output_len > 0);

        // The output should be valid truncated UTF-8 at least
        let json_str =
            std::str::from_utf8(&output_buf[..output_len as usize]).expect("valid truncated UTF-8");
        // Even truncated, should start with '{'
        assert!(json_str.starts_with('{'));
    }

    #[test]
    fn test_format_skips_empty_fields() {
        init(std::ptr::null()); // reset config to defaults
        let mut record = Record::new(0);
        // Set only two fields — everything else is default/empty
        record.level = dologger_core::LogLevel::Info;
        record.message.set("hello");

        let mut output_buf = vec![0u8; 4096];
        let mut output_len: u32 = output_buf.len() as u32;

        let result = unsafe { format_impl(&record, output_buf.as_mut_ptr(), &mut output_len) };
        assert_eq!(result, DO_LOG_OK);

        let json_str =
            std::str::from_utf8(&output_buf[..output_len as usize]).expect("valid UTF-8");
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");

        let obj = parsed.as_object().unwrap();
        // Only level, message should be present
        assert_eq!(obj.len(), 2);
        assert_eq!(obj["level"], "INFO");
        assert_eq!(obj["message"], "hello");
        // These defaults should NOT be in the output
        assert!(!obj.contains_key("source_file"));
        assert!(!obj.contains_key("source_line"));
        assert!(!obj.contains_key("thread_id"));
        assert!(!obj.contains_key("process_id"));
        assert!(!obj.contains_key("host_name"));
    }

    #[test]
    fn test_format_json_is_valid_utf8() {
        init(std::ptr::null()); // reset config to defaults
        let mut record = Record::new(0);
        record.level = dologger_core::LogLevel::Debug;
        record.message.set("UTF-8 test: \u{00e9}\u{00f1}\u{00fc}");

        let mut output_buf = vec![0u8; 4096];
        let mut output_len: u32 = output_buf.len() as u32;

        let result = unsafe { format_impl(&record, output_buf.as_mut_ptr(), &mut output_len) };
        assert_eq!(result, DO_LOG_OK);

        // Should be valid UTF-8
        let json_str =
            std::str::from_utf8(&output_buf[..output_len as usize]).expect("valid UTF-8");
        // Should parse as JSON
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");
        assert_eq!(parsed["message"], "UTF-8 test: \u{00e9}\u{00f1}\u{00fc}");
    }
}
