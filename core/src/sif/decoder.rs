//! SIF decoder — SIF frame → `Record` and structural validation.
//!
//! The decoder is the consuming half of the SIF pipeline stage. It validates a
//! frame (magic, version, length) and materialises the FlatBuffer `Record`
//! back into an in-memory [`Record`]. Sinks that only need to *read* fields can
//! use `root_as_record` on the payload directly for zero-copy access; this
//! module exists for round-trip fidelity and for stages that mutate records.

use std::fmt;

use crate::record::{LogLevel, Record};
use crate::sif::generated::{root_as_record, Record as SifRecord};
use crate::sif::{SifHeader, SIF_MAGIC, SIF_VERSION};

/// Errors produced while validating or decoding a SIF frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SifError {
    /// Frame is shorter than the 4-byte magic + 12-byte header.
    Truncated,
    /// The first four bytes are not `SIF_MAGIC`.
    InvalidMagic,
    /// Header schema `version` does not match this crate's `SIF_VERSION`.
    VersionMismatch {
        /// Version found in the frame header.
        found: u32,
        /// Version this crate understands.
        expected: u32,
    },
    /// Header `total_length` disagrees with the actual buffer length.
    LengthMismatch {
        /// `total_length` as stored in the frame header.
        header: u32,
        /// Actual length of the supplied buffer.
        actual: usize,
    },
    /// The FlatBuffer payload failed structural verification.
    FlatBuffer(String),
}

impl fmt::Display for SifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "SIF frame shorter than magic + header"),
            Self::InvalidMagic => write!(f, "SIF frame has invalid magic bytes"),
            Self::VersionMismatch { found, expected } => write!(
                f,
                "SIF schema version mismatch: found {found:#x}, expected {expected:#x}"
            ),
            Self::LengthMismatch { header, actual } => write!(
                f,
                "SIF header total_length {header} disagrees with buffer length {actual}"
            ),
            Self::FlatBuffer(e) => write!(f, "SIF FlatBuffer verification failed: {e}"),
        }
    }
}

impl std::error::Error for SifError {}

/// Validate a SIF frame's magic, version, and length, returning its header.
///
/// Cheap enough for a sink to run before touching the payload.
pub fn validate_frame(frame: &[u8]) -> Result<SifHeader, SifError> {
    if frame.len() < SifHeader::FRAME_OVERHEAD {
        return Err(SifError::Truncated);
    }
    if frame[..4] != SIF_MAGIC {
        return Err(SifError::InvalidMagic);
    }
    let version = u32::from_le_bytes(frame[4..8].try_into().expect("len >= 16"));
    if version != SIF_VERSION {
        return Err(SifError::VersionMismatch {
            found: version,
            expected: SIF_VERSION,
        });
    }
    let total_length = u32::from_le_bytes(frame[8..12].try_into().expect("len >= 16")) as usize;
    if total_length != frame.len() {
        return Err(SifError::LengthMismatch {
            header: total_length as u32,
            actual: frame.len(),
        });
    }
    let record_count = u32::from_le_bytes(frame[12..16].try_into().expect("len >= 16"));

    Ok(SifHeader {
        version,
        total_length: total_length as u32,
        record_count,
    })
}

/// Decode a SIF frame into a full in-memory [`Record`].
///
/// The returned record is `Record::new(0)` populated field-by-field, so it is
/// not pooled. `pool_index`/`flags` are carried through for round-trip
/// fidelity when snapshotting.
pub fn decode_record(frame: &[u8]) -> Result<Record, SifError> {
    let _header = validate_frame(frame)?;
    let payload = &frame[SifHeader::FRAME_OVERHEAD..];
    let sif = root_as_record(payload).map_err(|e| SifError::FlatBuffer(e.to_string()))?;
    Ok(sif_to_record(&sif))
}

/// Materialise a parsed FlatBuffer `Record` into an in-memory [`Record`].
fn sif_to_record(sif: &SifRecord<'_>) -> Record {
    let mut r = Record::new(0);

    r.id = crate::ffi::dologger_uint128_t {
        hi: sif.id_hi(),
        lo: sif.id_lo(),
    };
    r.timestamp = crate::ffi::dologger_uint128_t {
        hi: sif.timestamp_hi(),
        lo: sif.timestamp_lo(),
    };
    r.signature = vector_to_array(sif.signature().bytes());
    r.origin_lsn = sif.origin_lsn();
    r.level = LogLevel::from_u8(sif.level()).unwrap_or(LogLevel::Info);

    r.message.set(sif.message().unwrap_or(""));
    r.source_file.set(sif.source_file().unwrap_or(""));
    r.source_function.set(sif.source_function().unwrap_or(""));
    r.source_line = sif.source_line();
    r.source_column = sif.source_column();
    r.thread_id = sif.thread_id();
    r.thread_name.set(sif.thread_name().unwrap_or(""));
    r.process_id = sif.process_id();
    r.process_name.set(sif.process_name().unwrap_or(""));
    r.host_name.set(sif.host_name().unwrap_or(""));
    r.container_id.set(sif.container_id().unwrap_or(""));
    r.app_name.set(sif.app_name().unwrap_or(""));
    r.app_version.set(sif.app_version().unwrap_or(""));
    r.environment.set(sif.environment().unwrap_or(""));
    r.user_id.set(sif.user_id().unwrap_or(""));
    r.session_id.set(sif.session_id().unwrap_or(""));
    r.request_id.set(sif.request_id().unwrap_or(""));
    r.trace_id.set(sif.trace_id().unwrap_or(""));
    r.span_id.set(sif.span_id().unwrap_or(""));
    r.coroutine_id = sif.coroutine_id();

    r.exception_type.set(sif.exception_type().unwrap_or(""));
    r.exception_message
        .set(sif.exception_message().unwrap_or(""));
    r.exception_stacktrace
        .set(sif.exception_stacktrace().unwrap_or(""));
    r.exception_code = sif.exception_code();

    r.labels.set(sif.labels().unwrap_or(""));
    r.lsn = sif.lsn();
    r.prev_hash = vector_to_array(sif.prev_hash().bytes());
    r.security_gap = sif.security_gap();
    r.audit_tags.set(sif.audit_tags().unwrap_or(""));

    r.ext_data.set(sif.ext_data().unwrap_or(""));
    r.ext_crc32c = sif.ext_crc32c();

    r.pool_index = sif.pool_index();
    r.flags = sif.flags();

    r
}

/// Copy a vector slice into a fixed-size array, zero-padding short input and
/// truncating over-long input.
fn vector_to_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let n = bytes.len().min(N);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::dologger_uint128_t;
    use crate::sif::{encode_record, SifHeader, SIF_MAGIC, SIF_VERSION};

    /// A record with every schema field populated, including heap-backed
    /// `RecordString` fields.
    fn full_record() -> Record {
        let mut r = Record::new(0);
        r.id = dologger_uint128_t {
            hi: 0x1122_3344_5566_7788,
            lo: 0x99AA_BBCC_DDEE_FF01,
        };
        r.timestamp = dologger_uint128_t {
            hi: 1_780_000_000,
            lo: 123_456_789,
        };
        r.signature = [0xAA; 64];
        r.origin_lsn = 42;
        r.level = LogLevel::Error;
        r.message.set(&"M".repeat(300)); // heap path
        r.source_file.set("src/main.rs");
        r.source_function.set("main");
        r.source_line = 10;
        r.source_column = 5;
        r.thread_id = 12345;
        r.thread_name.set("worker-1");
        r.process_id = 999;
        r.process_name.set("dologctl");
        r.host_name.set("host.example.com");
        r.container_id.set("abc123");
        r.app_name.set("my-service");
        r.app_version.set("1.2.3");
        r.environment.set("prod");
        r.user_id.set("u-1");
        r.session_id.set("s-1");
        r.request_id.set("r-1");
        r.trace_id.set("t-1");
        r.span_id.set("sp-1");
        r.coroutine_id = 7;
        r.exception_type.set("std::runtime_error");
        r.exception_message.set("boom");
        r.exception_stacktrace.set("at main.rs:10");
        r.exception_code = -1;
        r.labels.set(r#"{"k":"v"}"#);
        r.lsn = 1000;
        r.prev_hash = [0xBB; 32];
        r.security_gap = true;
        r.audit_tags.set("[{\"plugin\":\"x\"}]");
        r.ext_data.set("ext-blob");
        r.ext_crc32c = 0xDEAD_BEEF;
        r.pool_index = 5;
        r.flags = 0x07;
        r
    }

    /// Assert every schema field of `a` equals `b`.
    fn assert_records_equal(a: &Record, b: &Record) {
        assert_eq!(a.id.hi, b.id.hi, "id.hi");
        assert_eq!(a.id.lo, b.id.lo, "id.lo");
        assert_eq!(a.timestamp.hi, b.timestamp.hi, "timestamp.hi");
        assert_eq!(a.timestamp.lo, b.timestamp.lo, "timestamp.lo");
        assert_eq!(a.signature, b.signature, "signature");
        assert_eq!(a.origin_lsn, b.origin_lsn, "origin_lsn");
        assert_eq!(a.level, b.level, "level");
        assert_eq!(a.message.as_str(), b.message.as_str(), "message");
        assert_eq!(
            a.source_file.as_str(),
            b.source_file.as_str(),
            "source_file"
        );
        assert_eq!(
            a.source_function.as_str(),
            b.source_function.as_str(),
            "source_function"
        );
        assert_eq!(a.source_line, b.source_line, "source_line");
        assert_eq!(a.source_column, b.source_column, "source_column");
        assert_eq!(a.thread_id, b.thread_id, "thread_id");
        assert_eq!(
            a.thread_name.as_str(),
            b.thread_name.as_str(),
            "thread_name"
        );
        assert_eq!(a.process_id, b.process_id, "process_id");
        assert_eq!(
            a.process_name.as_str(),
            b.process_name.as_str(),
            "process_name"
        );
        assert_eq!(a.host_name.as_str(), b.host_name.as_str(), "host_name");
        assert_eq!(
            a.container_id.as_str(),
            b.container_id.as_str(),
            "container_id"
        );
        assert_eq!(a.app_name.as_str(), b.app_name.as_str(), "app_name");
        assert_eq!(
            a.app_version.as_str(),
            b.app_version.as_str(),
            "app_version"
        );
        assert_eq!(
            a.environment.as_str(),
            b.environment.as_str(),
            "environment"
        );
        assert_eq!(a.user_id.as_str(), b.user_id.as_str(), "user_id");
        assert_eq!(a.session_id.as_str(), b.session_id.as_str(), "session_id");
        assert_eq!(a.request_id.as_str(), b.request_id.as_str(), "request_id");
        assert_eq!(a.trace_id.as_str(), b.trace_id.as_str(), "trace_id");
        assert_eq!(a.span_id.as_str(), b.span_id.as_str(), "span_id");
        assert_eq!(a.coroutine_id, b.coroutine_id, "coroutine_id");
        assert_eq!(
            a.exception_type.as_str(),
            b.exception_type.as_str(),
            "exception_type"
        );
        assert_eq!(
            a.exception_message.as_str(),
            b.exception_message.as_str(),
            "exception_message"
        );
        assert_eq!(
            a.exception_stacktrace.as_str(),
            b.exception_stacktrace.as_str(),
            "exception_stacktrace"
        );
        assert_eq!(a.exception_code, b.exception_code, "exception_code");
        assert_eq!(a.labels.as_str(), b.labels.as_str(), "labels");
        assert_eq!(a.lsn, b.lsn, "lsn");
        assert_eq!(a.prev_hash, b.prev_hash, "prev_hash");
        assert_eq!(a.security_gap, b.security_gap, "security_gap");
        assert_eq!(a.audit_tags.as_str(), b.audit_tags.as_str(), "audit_tags");
        assert_eq!(a.ext_data.as_str(), b.ext_data.as_str(), "ext_data");
        assert_eq!(a.ext_crc32c, b.ext_crc32c, "ext_crc32c");
        assert_eq!(a.pool_index, b.pool_index, "pool_index");
        assert_eq!(a.flags, b.flags, "flags");
    }

    #[test]
    fn roundtrip_full_record() {
        let original = full_record();
        let frame = encode_record(&original);
        let decoded = decode_record(&frame).expect("valid frame decodes");
        assert_records_equal(&original, &decoded);
    }

    #[test]
    fn roundtrip_empty_record() {
        let original = Record::new(0);
        let frame = encode_record(&original);
        let decoded = decode_record(&frame).expect("valid frame decodes");
        assert_records_equal(&original, &decoded);
        // Empty strings must round-trip to empty, not panic or misread.
        assert_eq!(decoded.message.as_str(), "");
    }

    #[test]
    fn roundtrip_unicode_message() {
        let mut original = Record::new(0);
        original.message.set(&"こんにちは世界 😀 ".repeat(40));
        let frame = encode_record(&original);
        let decoded = decode_record(&frame).expect("valid frame decodes");
        assert_eq!(decoded.message.as_str(), original.message.as_str());
    }

    #[test]
    fn frame_layout_is_magic_header_payload() {
        let original = full_record();
        let frame = encode_record(&original);
        // Magic at [0..4), header at [4..16), payload after.
        assert_eq!(&frame[..4], &SIF_MAGIC);
        let version = u32::from_le_bytes(frame[4..8].try_into().unwrap());
        assert_eq!(version, SIF_VERSION);
        let total = u32::from_le_bytes(frame[8..12].try_into().unwrap()) as usize;
        assert_eq!(total, frame.len());
        let count = u32::from_le_bytes(frame[12..16].try_into().unwrap());
        assert_eq!(count, 1);
        // Payload must start with the FlatBuffer root-table offset u32.
        let root_off = u32::from_le_bytes(frame[16..20].try_into().unwrap());
        assert!(root_off >= 4 && (root_off as usize) < frame.len());
    }

    #[test]
    fn validate_frame_ok() {
        let frame = encode_record(&full_record());
        let header = validate_frame(&frame).expect("valid frame validates");
        assert_eq!(header.version, SIF_VERSION);
        assert_eq!(header.total_length as usize, frame.len());
        assert_eq!(header.record_count, 1);
    }

    #[test]
    fn validate_frame_truncated() {
        assert_eq!(validate_frame(&[]), Err(SifError::Truncated));
        assert_eq!(
            validate_frame(&SIF_MAGIC),
            Err(SifError::Truncated),
            "magic only is still too short for the 12-byte header"
        );
    }

    #[test]
    fn validate_frame_invalid_magic() {
        let mut frame = encode_record(&full_record());
        frame[0] = b'X';
        assert_eq!(validate_frame(&frame), Err(SifError::InvalidMagic));
    }

    #[test]
    fn validate_frame_version_mismatch() {
        let mut frame = encode_record(&full_record());
        // Corrupt the version field (offset 4..8).
        frame[4..8].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        match validate_frame(&frame) {
            Err(SifError::VersionMismatch { found, expected }) => {
                assert_eq!(found, 0xDEAD_BEEF);
                assert_eq!(expected, SIF_VERSION);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn validate_frame_length_mismatch() {
        let mut frame = encode_record(&full_record());
        // Header says 4 bytes more than the buffer holds.
        let bad_len = (frame.len() + 4) as u32;
        frame[8..12].copy_from_slice(&bad_len.to_le_bytes());
        match validate_frame(&frame) {
            Err(SifError::LengthMismatch { header, actual }) => {
                assert_eq!(header, bad_len);
                assert_eq!(actual, frame.len());
            }
            other => panic!("expected LengthMismatch, got {other:?}"),
        }
    }

    #[test]
    fn frame_overhead_constant_matches_schema() {
        // The frame layout depends on magic (4) + SifHeader (12) = 16.
        assert_eq!(SifHeader::FRAME_OVERHEAD, 16);
        assert_eq!(core::mem::size_of::<SifHeader>(), 12);
    }
}
