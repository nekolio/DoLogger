//! SIF decoder — SIF frame → `Record` and structural validation.
//!
//! The decoder is the consuming half of the SIF pipeline stage. It validates a
//! frame (magic, version, length) and materialises the FlatBuffer `Record`
//! back into an in-memory [`Record`]. Sinks that only need to *read* fields can
//! use `root_as_record` on the payload directly for zero-copy access; this
//! module exists for round-trip fidelity and for stages that mutate records.

use std::fmt;

use super::generated::{root_as_record, Record as SifRecord};
use super::{
    FlatbuffersHeader, FLATBUFFERS_FRAME_OVERHEAD, FLATBUFFERS_SCHEMA_VERSION,
    FLATBUFFERS_SIF_MAGIC,
};
use crate::record::{LogLevel, Record};

const MESSAGE_BINARY_FLAG: u32 = 1 << 16;
const MESSAGE_EXPLICIT_TEXT_FLAG: u32 = 1 << 17;
const KV_ENVELOPE_PREFIX: &str = "kv:base64:";
pub(crate) const MAX_FLATBUFFERS_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Compatibility facts observed while decoding a SIF frame.
///
/// The current Record model restores its raw message and KV envelope. A caller
/// that accepts older frames can inspect this report and decide whether the
/// frame is suitable for replay, archival, or audit verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SifCompatibility {
    /// Header schema version carried by the frame.
    pub schema_version: u32,
    /// Whether the frame carried the optional SHA-256 content hash.
    pub content_hash_present: bool,
    /// Whether a legacy in-record signature was present and intentionally ignored.
    pub legacy_signature_present: bool,
    /// Whether a legacy replay origin LSN was present and intentionally ignored.
    pub legacy_origin_lsn_present: bool,
    /// Whether an older extension payload was present and intentionally ignored.
    pub legacy_ext_data_present: bool,
    /// Whether the extension field carried the current KV envelope.
    pub kv_envelope_present: bool,
}

impl SifCompatibility {
    /// True when the frame predates the content-hash field.
    pub fn is_pre_content_hash(&self) -> bool {
        !self.content_hash_present
    }

    /// Return a stable diagnostic summary for logs and migration reports.
    pub fn summary(&self) -> String {
        let mut fields = Vec::new();
        if self.is_pre_content_hash() {
            fields.push("missing-content-hash");
        }
        if self.legacy_signature_present {
            fields.push("ignored-signature");
        }
        if self.legacy_origin_lsn_present {
            fields.push("ignored-origin-lsn");
        }
        if self.legacy_ext_data_present {
            fields.push("ignored-ext-data");
        }
        if fields.is_empty() {
            "current".to_string()
        } else {
            fields.join(",")
        }
    }

    /// Whether replay is safe without a separate audit verification step.
    pub fn replay_requires_audit_review(&self) -> bool {
        self.is_pre_content_hash() || self.dropped_legacy_fields()
    }
    /// True when decoding required dropping older unsupported semantics.
    pub fn dropped_legacy_fields(&self) -> bool {
        self.legacy_signature_present
            || self.legacy_origin_lsn_present
            || self.legacy_ext_data_present
    }
}
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
    /// The frame exceeds the bounded backend budget.
    Oversize {
        /// Number of bytes supplied by the caller.
        found: usize,
        /// Maximum accepted frame size.
        max: usize,
    },
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
            Self::Oversize { found, max } => {
                write!(f, "SIF FlatBuffers frame size {found} exceeds {max}")
            }
        }
    }
}

impl std::error::Error for SifError {}

/// Validate a SIF frame's magic, version, and length, returning its header.
///
/// Cheap enough for a sink to run before touching the payload.
pub fn validate_frame(frame: &[u8]) -> Result<FlatbuffersHeader, SifError> {
    if frame.len() > MAX_FLATBUFFERS_FRAME_SIZE {
        return Err(SifError::Oversize {
            found: frame.len(),
            max: MAX_FLATBUFFERS_FRAME_SIZE,
        });
    }
    if frame.len() < FLATBUFFERS_FRAME_OVERHEAD {
        return Err(SifError::Truncated);
    }
    if frame[..4] != FLATBUFFERS_SIF_MAGIC {
        return Err(SifError::InvalidMagic);
    }
    let version = u32::from_le_bytes(frame[4..8].try_into().expect("len >= 16"));
    if version != FLATBUFFERS_SCHEMA_VERSION {
        return Err(SifError::VersionMismatch {
            found: version,
            expected: FLATBUFFERS_SCHEMA_VERSION,
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

    Ok(FlatbuffersHeader {
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
    decode_record_compat(frame).map(|(record, _compatibility)| record)
}

/// Decode a SIF frame and return compatibility facts alongside the Record.
///
/// This is the migration-safe entry point for replay and archival consumers.
/// Missing `content_hash` is accepted because it was optional in the original
/// table. Older unsupported fields are reported, while the current KV envelope
/// is restored into the Record model.
pub fn decode_record_compat(frame: &[u8]) -> Result<(Record, SifCompatibility), SifError> {
    let header = validate_frame(frame)?;
    let payload = &frame[FLATBUFFERS_FRAME_OVERHEAD..];
    let sif = root_as_record(payload).map_err(|e| SifError::FlatBuffer(e.to_string()))?;
    let compatibility = SifCompatibility {
        schema_version: header.version,
        content_hash_present: sif.content_hash().is_some(),
        legacy_signature_present: sif.signature().bytes().iter().any(|byte| *byte != 0),
        legacy_origin_lsn_present: sif.origin_lsn() != 0,
        legacy_ext_data_present: sif
            .ext_data()
            .is_some_and(|value| !value.is_empty() && !value.starts_with(KV_ENVELOPE_PREFIX)),
        kv_envelope_present: sif
            .ext_data()
            .is_some_and(|value| value.starts_with(KV_ENVELOPE_PREFIX)),
    };
    Ok((sif_to_record(&sif)?, compatibility))
}

/// Materialise a parsed FlatBuffer `Record` into an in-memory [`Record`].
fn sif_to_record(sif: &SifRecord<'_>) -> Result<Record, SifError> {
    let mut r = Record::new(0);

    r.set_id(sif.id_hi(), sif.id_lo());
    r.timestamp = sif.timestamp_hi() * 1_000_000_000 + sif.timestamp_lo();
    r.level = LogLevel::from_u8(sif.level()).unwrap_or(LogLevel::Info);

    let message = sif.message().unwrap_or("");
    let wire_flags = sif.flags();
    if wire_flags & MESSAGE_BINARY_FLAG != 0 && wire_flags & MESSAGE_EXPLICIT_TEXT_FLAG != 0 {
        return Err(SifError::FlatBuffer(
            "conflicting message kind flags".to_string(),
        ));
    }
    if wire_flags & MESSAGE_BINARY_FLAG != 0 {
        let Some(encoded) = message.strip_prefix("bin:base64:") else {
            return Err(SifError::FlatBuffer(
                "binary message marker is missing".to_string(),
            ));
        };
        if let Some(bytes) = decode_base64(encoded) {
            r.message.set_bytes(&bytes);
        } else {
            return Err(SifError::FlatBuffer(
                "invalid binary message encoding".to_string(),
            ));
        }
    } else if let Some(encoded) = message.strip_prefix("bin:base64:") {
        if let Some(bytes) = decode_base64(encoded) {
            r.message.set_bytes(&bytes);
        } else {
            r.message.set(message);
        }
    } else if wire_flags & MESSAGE_EXPLICIT_TEXT_FLAG != 0 {
        r.message.set_explicit_decoded_text(message);
    } else {
        r.message.set(message);
    }
    r.set_source_file(sif.source_file().unwrap_or(""));
    r.set_source_function(sif.source_function().unwrap_or(""));
    r.set_source_line(sif.source_line());
    r.set_source_column(sif.source_column());
    r.thread_id = sif.thread_id() as u32;
    r.set_thread_name(sif.thread_name().unwrap_or(""));
    r.process_id = sif.process_id();
    r.set_process_name(sif.process_name().unwrap_or(""));
    r.set_host_name(sif.host_name().unwrap_or(""));
    r.set_container_id(sif.container_id().unwrap_or(""));
    r.set_app_name(sif.app_name().unwrap_or(""));
    r.set_app_version(sif.app_version().unwrap_or(""));
    r.set_environment(sif.environment().unwrap_or(""));
    r.set_user_id(sif.user_id().unwrap_or(""));
    r.set_session_id(sif.session_id().unwrap_or(""));
    r.set_request_id(sif.request_id().unwrap_or(""));
    r.set_trace_id(sif.trace_id().unwrap_or(""));
    r.set_span_id(sif.span_id().unwrap_or(""));
    r.set_coroutine_id(sif.coroutine_id());

    r.set_exception_type(sif.exception_type().unwrap_or(""));
    r.set_exception_message(sif.exception_message().unwrap_or(""));
    r.set_exception_stacktrace(sif.exception_stacktrace().unwrap_or(""));
    r.set_exception_code(sif.exception_code() as i64);

    r.set_labels(sif.labels().unwrap_or(""));
    r.lsn = sif.lsn();
    r.set_security_gap(sif.security_gap());
    r.set_audit_tags(sif.audit_tags().unwrap_or(""));

    r.pool_index = sif.pool_index();
    r.flags = (wire_flags & u16::MAX as u32) as u16;
    // A.3 canonical-serialization hash; absent on pre-schema-evolution frames
    // (decodes to the zero initialiser).
    r.content_hash = sif
        .content_hash()
        .map(|v| <[u8; 32]>::try_from(v.bytes()).unwrap_or([0u8; 32]))
        .unwrap_or([0u8; 32]);

    if let Some(encoded) = sif
        .ext_data()
        .and_then(|value| value.strip_prefix(KV_ENVELOPE_PREFIX))
    {
        let bytes = decode_base64(encoded)
            .ok_or_else(|| SifError::FlatBuffer("invalid KV envelope encoding".to_string()))?;
        restore_kv_envelope(&mut r, &bytes)?;
    }

    Ok(r)
}

fn restore_kv_envelope(record: &mut Record, bytes: &[u8]) -> Result<(), SifError> {
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let header_end = cursor
            .checked_add(8)
            .ok_or_else(|| SifError::FlatBuffer("KV envelope offset overflow".to_string()))?;
        let header = bytes
            .get(cursor..header_end)
            .ok_or_else(|| SifError::FlatBuffer("truncated KV envelope header".to_string()))?;
        let tag = header[0];
        let ty = header[1];
        let name_len = u16::from_le_bytes([header[2], header[3]]) as usize;
        let value_len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let name_start = header_end;
        let value_start = name_start
            .checked_add(name_len)
            .ok_or_else(|| SifError::FlatBuffer("KV envelope name overflow".to_string()))?;
        let value_end = value_start
            .checked_add(value_len)
            .ok_or_else(|| SifError::FlatBuffer("KV envelope value overflow".to_string()))?;
        let name = std::str::from_utf8(
            bytes
                .get(name_start..value_start)
                .ok_or_else(|| SifError::FlatBuffer("truncated KV envelope name".to_string()))?,
        )
        .map_err(|_| SifError::FlatBuffer("KV envelope name is not UTF-8".to_string()))?;
        let value = bytes
            .get(value_start..value_end)
            .ok_or_else(|| SifError::FlatBuffer("truncated KV envelope value".to_string()))?;
        if tag == 0 || name.is_empty() {
            return Err(SifError::FlatBuffer(
                "invalid KV envelope field".to_string(),
            ));
        }
        crate::sif::codec::register_vendor_tag(name, tag);
        record
            .put_wire_slot(tag, ty, value)
            .map_err(|error| SifError::FlatBuffer(format!("invalid KV envelope field: {error}")))?;
        cursor = value_end;
    }
    Ok(())
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(4) {
        return None;
    }
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for chunk in value.as_bytes().chunks_exact(4) {
        let values = [decode_base64_byte(chunk[0])?, decode_base64_byte(chunk[1])?];
        output.push((values[0] << 2) | (values[1] >> 4));
        if chunk[2] != b'=' {
            let third = decode_base64_byte(chunk[2])?;
            output.push((values[1] << 4) | (third >> 2));
            if chunk[3] != b'=' {
                let fourth = decode_base64_byte(chunk[3])?;
                output.push((third << 6) | fourth);
            }
        }
    }
    Some(output)
}

fn decode_base64_byte(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::encoder::encode_record;
    use super::super::{
        FlatbuffersHeader as SifHeader, FLATBUFFERS_SCHEMA_VERSION as SIF_VERSION,
        FLATBUFFERS_SIF_MAGIC as SIF_MAGIC,
    };
    use super::*;

    /// A record with every schema field populated, including heap-backed
    /// `RecordString` fields.
    fn full_record() -> Record {
        let mut r = Record::new(0);
        r.set_id(0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF01);
        r.timestamp = 1_780_000_000u64 * 1_000_000_000 + 123_456_789;
        r.level = LogLevel::Error;
        r.message.set(&"M".repeat(300)); // heap path
        r.set_source_file("src/main.rs");
        r.set_source_function("main");
        r.set_source_line(10);
        r.set_source_column(5);
        r.thread_id = 12345;
        r.set_thread_name("worker-1");
        r.process_id = 999;
        r.set_process_name("dologctl");
        r.set_host_name("host.example.com");
        r.set_container_id("abc123");
        r.set_app_name("my-service");
        r.set_app_version("1.2.3");
        r.set_environment("prod");
        r.set_user_id("u-1");
        r.set_session_id("s-1");
        r.set_request_id("r-1");
        r.set_trace_id("t-1");
        r.set_span_id("sp-1");
        r.set_coroutine_id(7);
        r.set_exception_type("std::runtime_error");
        r.set_exception_message("boom");
        r.set_exception_stacktrace("at main.rs:10");
        r.set_exception_code(-1);
        r.set_labels(r#"{"k":"v"}"#);
        r.lsn = 1000;
        r.set_security_gap(true);
        r.set_audit_tags("[{\"plugin\":\"x\"}]");
        r.pool_index = 5;
        r.flags = 0x07;
        r.content_hash = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C,
            0x1D, 0x1E, 0x1F, 0x20,
        ];
        r
    }

    /// Assert every schema field of `a` equals `b`.
    fn assert_records_equal(a: &Record, b: &Record) {
        assert_eq!(a.id_hi(), b.id_hi(), "id_hi");
        assert_eq!(a.id_lo(), b.id_lo(), "id_lo");
        assert_eq!(a.timestamp, b.timestamp, "timestamp");
        assert_eq!(a.level, b.level, "level");
        assert_eq!(a.message.as_bytes(), b.message.as_bytes(), "msg");
        assert_eq!(a.source_file(), b.source_file(), "source_file");
        assert_eq!(a.source_function(), b.source_function(), "source_function");
        assert_eq!(a.source_line(), b.source_line(), "source_line");
        assert_eq!(a.source_column(), b.source_column(), "source_column");
        assert_eq!(a.thread_id, b.thread_id, "tid");
        assert_eq!(a.thread_name(), b.thread_name(), "thread_name");
        assert_eq!(a.process_id, b.process_id, "pid");
        assert_eq!(a.process_name(), b.process_name(), "process_name");
        assert_eq!(a.host_name(), b.host_name(), "host_name");
        assert_eq!(a.container_id(), b.container_id(), "container_id");
        assert_eq!(a.app_name(), b.app_name(), "app_name");
        assert_eq!(a.app_version(), b.app_version(), "app_version");
        assert_eq!(a.environment(), b.environment(), "environment");
        assert_eq!(a.user_id(), b.user_id(), "user_id");
        assert_eq!(a.session_id(), b.session_id(), "session_id");
        assert_eq!(a.request_id(), b.request_id(), "request_id");
        assert_eq!(a.trace_id(), b.trace_id(), "trace_id");
        assert_eq!(a.span_id(), b.span_id(), "span_id");
        assert_eq!(a.coroutine_id(), b.coroutine_id(), "coroutine_id");
        assert_eq!(a.exception_type(), b.exception_type(), "exception_type");
        assert_eq!(
            a.exception_message(),
            b.exception_message(),
            "exception_message"
        );
        assert_eq!(
            a.exception_stacktrace(),
            b.exception_stacktrace(),
            "exception_stacktrace"
        );
        assert_eq!(a.exception_code(), b.exception_code(), "exception_code");
        assert_eq!(a.labels(), b.labels(), "labels");
        assert_eq!(a.lsn, b.lsn, "lsn");
        assert_eq!(a.security_gap(), b.security_gap(), "security_gap");
        assert_eq!(a.audit_tags(), b.audit_tags(), "audit_tags");
        assert_eq!(a.pool_index, b.pool_index, "pool_index");
        assert_eq!(a.flags, b.flags, "flags");
        assert_eq!(a.content_hash, b.content_hash, "content_hash");
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
        assert_eq!(decoded.message.as_utf8().unwrap(), "");
    }

    #[test]
    fn roundtrip_unicode_message() {
        let mut original = Record::new(0);
        original.message.set(&"こんにちは世界 😀 ".repeat(40));
        let frame = encode_record(&original);
        let decoded = decode_record(&frame).expect("valid frame decodes");
        assert_eq!(decoded.message.as_bytes(), original.message.as_bytes());
    }

    #[test]
    fn roundtrip_binary_message_uses_backend_base64_marker() {
        let mut original = Record::new(0);
        original.message.set_bytes(&[0, 0xff, 0x80, 0x01]);
        let frame = encode_record(&original);
        let decoded = decode_record(&frame).expect("valid binary compatibility frame decodes");
        assert_eq!(
            decoded.message.kind(),
            crate::record::MessagePayloadKind::Binary
        );
        assert_eq!(decoded.message.as_bytes(), original.message.as_bytes());
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
    #[test]
    fn compatibility_report_is_clean_for_current_frames() {
        let original = full_record();
        let frame = encode_record(&original);
        let (decoded, compatibility) = decode_record_compat(&frame).expect("compat decode");

        assert_eq!(decoded.message.as_bytes(), original.message.as_bytes());
        assert!(compatibility.content_hash_present);
        assert!(!compatibility.dropped_legacy_fields());
    }

    #[test]
    fn current_compatibility_summary_is_stable() {
        let frame = encode_record(&full_record());
        let (_, compatibility) = decode_record_compat(&frame).expect("compat decode");
        assert_eq!(compatibility.summary(), "current");
        assert!(!compatibility.replay_requires_audit_review());
    }

    #[test]
    fn ordinary_decode_discards_compatibility_metadata() {
        let frame = encode_record(&full_record());
        let decoded = decode_record(&frame).expect("ordinary decode");
        assert_eq!(decoded.lsn, 1000);
    }
}
