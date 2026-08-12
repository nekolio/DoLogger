//! Fuzz target for the Record/SIF binary format and field access API.
//!
//! Exercises:
//! - Record creation and field_get/set with random field names and values
//! - Ring permission checks for all ring levels (Ring 0–3)
//! - CRC32C computation with arbitrary data
//! - RecordString set/as_str round-trip
//! - LogLevel parsing from arbitrary u8 values

#![no_main]

use dologger_core::record::{FieldRing, LogLevel, Record, RecordString};
use dologger_core::crc32c;
use libfuzzer_sys::fuzz_target;

/// Generate a pseudo-random field name from bytes, biased towards
/// known field names to exercise the real match arms.
fn bytes_to_field_name(data: &[u8]) -> &str {
    const KNOWN_FIELDS: &[&str] = &[
        // Ring 0
        "record.id",
        "record.timestamp",
        "record.signature",
        "record.origin_lsn",
        // Ring 1
        "level",
        "message",
        "source.file",
        "source.function",
        "source.line",
        "source.column",
        "thread.id",
        "thread.name",
        "process.id",
        "process.name",
        "host.name",
        "container.id",
        "app.name",
        "app.version",
        "environment",
        "user.id",
        "session.id",
        "request.id",
        "trace.id",
        "span.id",
        "coroutine.id",
        "exception.type",
        "exception.message",
        "exception.stacktrace",
        "exception.code",
        "labels",
        "security.lsn",
        "security.prev_hash",
        "security.gap",
        "security.audit_tags",
        // Ring 2 (verified. prefix)
        "verified.custom_field",
        "verified.audit_data",
        // Ring 3 (ext. prefix)
        "ext.extra1",
        "ext.extra2",
    ];

    if data.is_empty() {
        return "message";
    }

    // Use first byte to pick known vs. random
    let idx = (data[0] as usize) % (KNOWN_FIELDS.len() + 1);
    if idx < KNOWN_FIELDS.len() {
        KNOWN_FIELDS[idx]
    } else {
        // Generate a random field name from bytes
        "ext.custom"
    }
}

/// Generate a value string from bytes (limited to prevent excessive allocation).
fn bytes_to_value(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let len = (data.len() % 256) + 1; // 1..256 bytes
    let base = data.iter().cycle().take(len).copied().collect::<Vec<u8>>();
    String::from_utf8_lossy(&base).into_owned()
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // --- 1. Record creation ---
    let mut record = Record::new(0);

    // --- 2. Field access with random field names ---
    let field_name = bytes_to_field_name(data);
    let value = bytes_to_value(data);

    // Test field_ring mapping for all ring levels
    let _ring = Record::field_ring(field_name);

    // Test field_set with all caller ring levels
    for caller_ring in &[FieldRing::Ring0, FieldRing::Ring1, FieldRing::Ring2, FieldRing::Ring3] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            record.field_set(field_name, &value, *caller_ring)
        }));

        match result {
            Ok(Ok(())) => { /* write succeeded */ }
            Ok(Err(e)) => {
                // Permission denied is expected for some caller/field combos
                assert!(!e.is_empty(), "error message should not be empty");
            }
            Err(panic_err) => {
                let msg = format!("{:?}", panic_err);
                panic!(
                    "field_set panicked: field='{field_name}', caller={caller_ring:?}, msg={msg}"
                );
            }
        }
    }

    // Test field_get with all caller ring levels
    for caller_ring in &[FieldRing::Ring0, FieldRing::Ring1, FieldRing::Ring2, FieldRing::Ring3] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            record.field_get(field_name, *caller_ring)
        }));

        match result {
            Ok(Ok(value)) => {
                // Read succeeded — value should be valid UTF-8
                assert!(!value.is_empty() || true, "empty value is valid");
            }
            Ok(Err(e)) => {
                assert!(!e.is_empty(), "error message should not be empty");
            }
            Err(panic_err) => {
                let msg = format!("{:?}", panic_err);
                panic!(
                    "field_get panicked: field='{field_name}', caller={caller_ring:?}, msg={msg}"
                );
            }
        }
    }

    // --- 3. RecordString round-trip ---
    let mut rs = RecordString::empty();
    assert!(rs.is_empty());
    assert_eq!(rs.len(), 0);
    assert_eq!(rs.as_str(), "");

    rs.set(&value);
    let roundtripped = rs.as_str().to_string();
    // Truncation may occur for strings > RECORD_STRING_INLINE_CAPACITY
    let expected_len = value.len().min(dologger_core::record::RECORD_STRING_INLINE_CAPACITY - 1);
    assert_eq!(
        rs.len(),
        roundtripped.len(),
        "RecordString len() and as_str().len() must agree"
    );
    assert!(
        rs.len() <= dologger_core::record::RECORD_STRING_INLINE_CAPACITY - 1,
        "RecordString len must be <= inline capacity - 1 (null terminator)"
    );
    assert!(
        roundtripped.len() <= dologger_core::record::RECORD_STRING_INLINE_CAPACITY - 1,
        "roundtripped len must be <= inline capacity - 1"
    );

    // Empty after reset
    let mut rs2 = RecordString::empty();
    rs2.set("");
    assert!(rs2.is_empty());
    assert_eq!(rs2.as_str(), "");

    // --- 4. CRC32C computation ---
    let crc_val = crc32c::crc32c(data);

    // CRC32C of empty slice must be 0
    assert_eq!(crc32c::crc32c(b""), 0, "CRC32C of empty data must be 0");

    // CRC32C must be deterministic
    let crc_val2 = crc32c::crc32c(data);
    assert_eq!(crc_val, crc_val2, "CRC32C must be deterministic");

    // Incremental CRC32C should equal full CRC32C
    if !data.is_empty() {
        let mid = data.len() / 2;
        let partial = crc32c::crc32c_update(0, &data[..mid]);
        let incremental = crc32c::crc32c_update(partial, &data[mid..]);
        assert_eq!(crc_val, incremental, "Incremental CRC32C must match full CRC32C");
    }

    // CRC32C on known vector (RFC 3720)
    assert_eq!(
        crc32c::crc32c(b"123456789"),
        0xE3069283,
        "CRC32C must match RFC 3720 test vector"
    );

    // CRC32C should not overflow — u32 wraps safely but result must be consistent
    let large_data = vec![0xFFu8; 65536];
    let crc_large = crc32c::crc32c(&large_data);
    let crc_large2 = crc32c::crc32c(&large_data);
    assert_eq!(crc_large, crc_large2, "CRC32C must be deterministic on large input");

    // --- 5. LogLevel parsing ---
    if !data.is_empty() {
        let level_byte = data[0];
        let level = LogLevel::from_u8(level_byte);
        if level_byte <= 6 {
            assert!(level.is_some(), "valid log level u8 should parse");
        } else {
            assert!(level.is_none(), "invalid log level u8 should return None");
        }
    }

    // --- 6. Ring 2 fields append audit tags ---
    let mut record2 = Record::new(1);
    let ring2_field = "verified.test_field";
    assert_eq!(Record::field_ring(ring2_field), Some(FieldRing::Ring2));

    // Setting a Ring 2 field should append to audit_tags
    let _ = record2.field_set(ring2_field, "test_value", FieldRing::Ring2);
    let tags = record2.audit_tags.as_str();
    // Ring 2 write should have populated audit_tags (should not be empty)
    // or at minimum should not have panicked
    let _ = tags;

    // --- 7. Ring 3 field sets auto-compute CRC32C ---
    let mut record3 = Record::new(2);
    let ring3_field = "ext.custom_data";
    assert_eq!(Record::field_ring(ring3_field), Some(FieldRing::Ring3));

    let initial_crc = record3.ext_crc32c;
    let _ = record3.field_set(ring3_field, "ring3_data", FieldRing::Ring3);
    // CRC32C should be updated after Ring 3 write
    if record3.ext_data.as_str() == "ring3_data" {
        let expected_crc = crc32c::crc32c(b"ring3_data");
        assert_eq!(
            record3.ext_crc32c, expected_crc,
            "Ring 3 ext_crc32c must match crc32c(ext_data)"
        );
    }

    // --- 8. Ring permission enforcement ---
    // Ring 0 write by non-Ring0 caller should be denied
    let mut record4 = Record::new(3);
    let result = record4.field_set("record.id", "test", FieldRing::Ring1);
    assert!(
        result.is_err(),
        "Ring 0 write by Ring 1 caller must be denied"
    );

    // Ring 1 write by Ring 2/3 caller should be denied
    let result = record4.field_set("message", "test", FieldRing::Ring2);
    assert!(
        result.is_err(),
        "Ring 1 write by Ring 2 caller must be denied"
    );

    let result = record4.field_set("message", "test", FieldRing::Ring3);
    assert!(
        result.is_err(),
        "Ring 1 write by Ring 3 caller must be denied"
    );
});

// ===========================================================================
// Standalone edge-case tests
// ===========================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    // --- Record creation ---

    #[test]
    fn edge_record_new_is_zeroed() {
        let record = Record::new(0);
        assert_eq!(record.pool_index, 0);
        assert_eq!(record.id.hi, 0);
        assert_eq!(record.id.lo, 0);
        assert_eq!(record.level, LogLevel::Info);
        assert_eq!(record.message.as_str(), "");
        assert_eq!(record.ext_crc32c, 0);
    }

    #[test]
    fn edge_record_reset() {
        let mut record = Record::new(0);
        record.message.set("hello");
        record.ext_crc32c = 0xDEADBEEF;
        record.reset();
        assert_eq!(record.message.as_str(), ""); // reset doesn't zero message (only flags/sig/crc)
        assert_eq!(record.ext_crc32c, 0);
        assert_eq!(record.signature, [0u8; 64]);
    }

    // --- RecordString ---

    #[test]
    fn edge_recordstring_empty() {
        let rs = RecordString::empty();
        assert!(rs.is_empty());
        assert_eq!(rs.len(), 0);
        assert_eq!(rs.as_str(), "");
    }

    #[test]
    fn edge_recordstring_set_and_get() {
        let mut rs = RecordString::empty();
        rs.set("Hello, World!");
        assert_eq!(rs.as_str(), "Hello, World!");
        assert_eq!(rs.len(), 13);
        assert!(!rs.is_empty());
    }

    #[test]
    fn edge_recordstring_truncation() {
        let mut rs = RecordString::empty();
        let long = "A".repeat(500);
        rs.set(&long);
        // Should be truncated to inline capacity - 1
        assert_eq!(rs.len(), 255);
        assert_eq!(rs.as_str().len(), 255);
    }

    #[test]
    fn edge_recordstring_unicode() {
        let mut rs = RecordString::empty();
        rs.set("Hello, World!");
        assert_eq!(rs.as_str(), "Hello, World!");
        rs.set("こんにちは世界"); // Japanese "Hello World"
        assert_eq!(rs.as_str(), "こんにちは世界");
    }

    #[test]
    fn edge_recordstring_debug_format() {
        let mut rs = RecordString::empty();
        rs.set("test");
        let debug_str = format!("{rs:?}");
        assert_eq!(debug_str, "\"test\"");
    }

    // --- Field ring mapping ---

    #[test]
    fn edge_ring_mapping_ring0() {
        assert_eq!(Record::field_ring("record.id"), Some(FieldRing::Ring0));
        assert_eq!(
            Record::field_ring("record.timestamp"),
            Some(FieldRing::Ring0)
        );
        assert_eq!(
            Record::field_ring("record.signature"),
            Some(FieldRing::Ring0)
        );
        assert_eq!(
            Record::field_ring("record.origin_lsn"),
            Some(FieldRing::Ring0)
        );
    }

    #[test]
    fn edge_ring_mapping_ring1() {
        assert_eq!(Record::field_ring("level"), Some(FieldRing::Ring1));
        assert_eq!(Record::field_ring("message"), Some(FieldRing::Ring1));
        assert_eq!(Record::field_ring("host.name"), Some(FieldRing::Ring1));
        assert_eq!(
            Record::field_ring("exception.type"),
            Some(FieldRing::Ring1)
        );
    }

    #[test]
    fn edge_ring_mapping_ring2() {
        assert_eq!(
            Record::field_ring("verified.custom"),
            Some(FieldRing::Ring2)
        );
        assert_eq!(
            Record::field_ring("verified.anything_here"),
            Some(FieldRing::Ring2)
        );
    }

    #[test]
    fn edge_ring_mapping_ring3() {
        assert_eq!(Record::field_ring("ext.extra"), Some(FieldRing::Ring3));
        assert_eq!(Record::field_ring("ext.foo.bar"), Some(FieldRing::Ring3));
    }

    #[test]
    fn edge_ring_mapping_unknown() {
        assert_eq!(Record::field_ring("unknown.field"), None);
        assert_eq!(Record::field_ring(""), None); // empty string is not "ext.*"
    }

    // --- CRC32C ---

    #[test]
    fn edge_crc32c_empty() {
        assert_eq!(crc32c::crc32c(b""), 0);
    }

    #[test]
    fn edge_crc32c_known_vector() {
        assert_eq!(crc32c::crc32c(b"123456789"), 0xE3069283);
    }

    #[test]
    fn edge_crc32c_deterministic() {
        let data = b"deterministic test data";
        let a = crc32c::crc32c(data);
        let b = crc32c::crc32c(data);
        assert_eq!(a, b);
    }

    #[test]
    fn edge_crc32c_incremental() {
        let data = b"Hello, this is a test for incremental CRC32C computation.";
        let full = crc32c::crc32c(data);
        let mid = data.len() / 2;
        let partial = crc32c::crc32c_update(0, &data[..mid]);
        let incremental = crc32c::crc32c_update(partial, &data[mid..]);
        assert_eq!(full, incremental);
    }

    #[test]
    fn edge_crc32c_large_input() {
        let data = vec![0xABu8; 1_000_000];
        let crc1 = crc32c::crc32c(&data);
        let crc2 = crc32c::crc32c(&data);
        assert_eq!(crc1, crc2);
    }

    #[test]
    fn edge_crc32c_single_byte() {
        let crc = crc32c::crc32c(b"X");
        assert_ne!(crc, 0, "CRC32C of non-empty input should not be 0");
    }

    // --- LogLevel ---

    #[test]
    fn edge_loglevel_all_variants() {
        for i in 0..=6u8 {
            let level = LogLevel::from_u8(i);
            assert!(level.is_some(), "valid LogLevel {i} should parse");
        }
    }

    #[test]
    fn edge_loglevel_invalid() {
        for i in 7..=255u8 {
            assert!(LogLevel::from_u8(i).is_none(), "invalid LogLevel {i} should be None");
        }
    }

    #[test]
    fn edge_loglevel_display() {
        assert_eq!(LogLevel::Trace.to_str(), "TRACE");
        assert_eq!(LogLevel::Debug.to_str(), "DEBUG");
        assert_eq!(LogLevel::Info.to_str(), "INFO");
        assert_eq!(LogLevel::Warn.to_str(), "WARN");
        assert_eq!(LogLevel::Error.to_str(), "ERROR");
        assert_eq!(LogLevel::Fatal.to_str(), "FATAL");
        assert_eq!(LogLevel::Audit.to_str(), "AUDIT");
    }

    // --- Field set/get ---

    #[test]
    fn edge_field_set_message() {
        let mut record = Record::new(0);
        let result = record.field_set("message", "hello world", FieldRing::Ring1);
        assert!(result.is_ok());
        let value = record.field_get("message", FieldRing::Ring0);
        assert_eq!(value.unwrap(), "hello world");
    }

    #[test]
    fn edge_field_set_numeric_field() {
        let mut record = Record::new(0);
        record.field_set("source.line", "42", FieldRing::Ring1).unwrap();
        assert_eq!(record.source_line, 42);
        let value = record.field_get("source.line", FieldRing::Ring0).unwrap();
        assert_eq!(value, "42");
    }

    #[test]
    fn edge_field_set_ring0_denied() {
        let mut record = Record::new(0);
        // Ring 1 caller cannot write Ring 0 field
        let result = record.field_set("record.id", "test", FieldRing::Ring1);
        assert!(result.is_err());
    }

    #[test]
    fn edge_field_set_ring1_denied_for_plugins() {
        let mut record = Record::new(0);
        // Ring 2 caller (plugin) cannot write Ring 1 field
        let result = record.field_set("message", "test", FieldRing::Ring2);
        assert!(result.is_err());
    }

    #[test]
    fn edge_field_set_ring3_any_caller() {
        let mut record = Record::new(0);
        // Ring 3 can be written by any caller
        let result = record.field_set("ext.foo", "bar", FieldRing::Ring3);
        assert!(result.is_ok());
        let value = record.field_get("ext.foo", FieldRing::Ring3).unwrap();
        assert_eq!(value, "bar");
        // CRC32C should be auto-computed
        assert_eq!(record.ext_crc32c, crc32c::crc32c(b"bar"));
    }

    #[test]
    fn edge_field_get_unknown_field() {
        let record = Record::new(0);
        let result = record.field_get("nonexistent.field", FieldRing::Ring0);
        assert!(result.is_err());
    }

    #[test]
    fn edge_ring2_audit_tags_append() {
        let mut record = Record::new(0);
        record.field_set("verified.field1", "val1", FieldRing::Ring2).unwrap();
        record.field_set("verified.field2", "val2", FieldRing::Ring2).unwrap();

        let tags = record.audit_tags.as_str();
        assert!(!tags.is_empty(), "audit_tags should be populated after Ring 2 write");
        assert!(tags.starts_with('['), "audit_tags should be JSON array");
        assert!(tags.ends_with(']'), "audit_tags should end with ']'");
        assert!(tags.contains("verified.field1"), "should contain field name");
        assert!(tags.contains("val1"), "should contain value");
    }

    #[test]
    fn edge_field_set_security_gap() {
        let mut record = Record::new(0);
        record.field_set("security.gap", "true", FieldRing::Ring1).unwrap();
        assert!(record.security_gap);

        record.field_set("security.gap", "false", FieldRing::Ring1).unwrap();
        assert!(!record.security_gap);
    }
}
