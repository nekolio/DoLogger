//! Official DoLogger Formatter plugin — `fmt_json`.
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

use dologger_core::ffi::DologgerPluginInfo;
use dologger_core::Record;
use serde_json::Value;

// Re-use core error codes
const DO_LOG_OK: i32 = 0;

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
// Helper: format 128-bit timestamp as seconds.nanoseconds
// ---------------------------------------------------------------------------

fn format_timestamp(hi: u64, lo: u64) -> String {
    format!("{}.{:09}", hi, lo)
}

// ---------------------------------------------------------------------------
// VTable function: fmt_json_format
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

    let json_value = match record_to_json(rec) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let json_bytes = match serde_json::to_vec(&json_value) {
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
/// non-zero fields. Returns an error if any field access fails.
fn record_to_json(rec: &Record) -> Result<Value, ()> {
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
            Value::String(format_timestamp(rec.timestamp.hi, rec.timestamp.lo)),
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

    // ── Ring 1: Source location ──
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

    // ── Ring 3: Extension data ──
    let ed = rec.ext_data.as_str();
    if !ed.is_empty() {
        map.insert("ext_data".to_string(), Value::String(ed.to_string()));
    }

    Ok(Value::Object(map))
}

static VTABLE: FormatterVTable = FormatterVTable {
    format: format_impl,
    flush: None,
};

static PLUGIN_NAME: &[u8] = b"fmt-json\0";

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
        return DO_LOG_OK; // Use defaults
    }

    // Config is a JSON string: {"pretty":false,"include_ring3":false,...}
    // SAFETY: config validated non-null above. CStr::from_ptr reads a
    // null-terminated UTF-8 string provided by the host.
    let config_str = unsafe { CStr::from_ptr(config as *const c_char) };
    if let Ok(_s) = config_str.to_str() {
        // TODO: Parse config fields (pretty, include_ring3, timestamp_format)
        // and store in static state for use by format_impl.
        // For now, we accept any valid config and use defaults.
    }
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
        assert_eq!(name, "fmt-json");
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
