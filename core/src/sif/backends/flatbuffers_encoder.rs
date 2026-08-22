//! SIF encoder — `Record` → SIF frame.
//!
//! The encoder is the serialisation half of the SIF pipeline stage:
//! [`encode_record`] materialises a [`Record`] into a complete framed SIF
//! message, ready for zero-copy consumption by a Sink.

use super::generated::{finish_record_buffer, Record as SifRecord, RecordArgs};
use super::{
    FLATBUFFERS_FRAME_OVERHEAD, FLATBUFFERS_INITIAL_BUFFER_SIZE, FLATBUFFERS_MAX_PREAMBLE,
    FLATBUFFERS_SIF_MAGIC,
};
use crate::record::{MessagePayloadKind, Record};
use flatbuffers::{FlatBufferBuilder, WIPOffset};

const MESSAGE_BINARY_FLAG: u32 = 1 << 16;
const MESSAGE_EXPLICIT_TEXT_FLAG: u32 = 1 << 17;
const KV_ENVELOPE_PREFIX: &str = "kv:base64:";

/// Encode a [`Record`] into a complete SIF frame.
///
/// # Frame layout
///
/// | Offset | Size | Field                          |
/// |--------|------|--------------------------------|
/// | 0      | 4    | FlatBuffers backend marker     |
/// | 4      | 12   | backend header (three LE u32s) |
/// | 16     | var  | FlatBuffer `Record` table      |
///
/// # Required fields
///
/// The schema marks `signature` (64B) and `prev_hash` (32B) as `(required)`.
/// They are always serialised. Until the pipeline signing point populates
/// them they carry their zero-initialised placeholders, so the resulting frame
/// always passes structural verification.
pub fn encode_record(record: &Record) -> Vec<u8> {
    // FlatBuffers builds bottom-up: every nested string and vector must be
    // created before the table that references it.
    let mut fbb = FlatBufferBuilder::with_capacity(
        FLATBUFFERS_INITIAL_BUFFER_SIZE + FLATBUFFERS_MAX_PREAMBLE + record.message.len(),
    );

    let signature = Some(fbb.create_vector(&[0u8; 64])); // placeholder; signature is external
    let prev_hash = Some(fbb.create_vector(&[0u8; 32])); // placeholder; chain is content_hash-based
    let content_hash = Some(fbb.create_vector(&record.content_hash));

    // Build owned strings for KV-accessed fields (KV accessors return String)
    let source_file_str = record.source_file();
    let source_function_str = record.source_function();
    let thread_name_str = record.thread_name();
    let process_name_str = record.process_name();
    let host_name_str = record.host_name();
    let container_id_str = record.container_id();
    let app_name_str = record.app_name();
    let app_version_str = record.app_version();
    let environment_str = record.environment();
    let user_id_str = record.user_id();
    let session_id_str = record.session_id();
    let request_id_str = record.request_id();
    let trace_id_str = record.trace_id();
    let span_id_str = record.span_id();
    let exception_type_str = record.exception_type();
    let exception_message_str = record.exception_message();
    let exception_stacktrace_str = record.exception_stacktrace();
    let labels_str = record.labels();
    let audit_tags_str = record.audit_tags();

    let (message_text, message_kind_flags) = match record.message.kind() {
        MessagePayloadKind::Utf8 => (record.message.as_utf8().unwrap_or_default().to_string(), 0),
        MessagePayloadKind::ExplicitDecodedText => (
            record.message.as_utf8().unwrap_or_default().to_string(),
            MESSAGE_EXPLICIT_TEXT_FLAG,
        ),
        MessagePayloadKind::Binary => (
            format!("bin:base64:{}", encode_base64(record.message.as_bytes())),
            MESSAGE_BINARY_FLAG,
        ),
    };
    let kv_envelope = encode_kv_envelope(record);
    let kv_envelope_text = (!kv_envelope.is_empty())
        .then(|| format!("{KV_ENVELOPE_PREFIX}{}", encode_base64(&kv_envelope)));
    let message = opt_string(&mut fbb, &message_text);
    let source_file = opt_string(&mut fbb, &source_file_str);
    let source_function = opt_string(&mut fbb, &source_function_str);
    let thread_name = opt_string(&mut fbb, &thread_name_str);
    let process_name = opt_string(&mut fbb, &process_name_str);
    let host_name = opt_string(&mut fbb, &host_name_str);
    let container_id = opt_string(&mut fbb, &container_id_str);
    let app_name = opt_string(&mut fbb, &app_name_str);
    let app_version = opt_string(&mut fbb, &app_version_str);
    let environment = opt_string(&mut fbb, &environment_str);
    let user_id = opt_string(&mut fbb, &user_id_str);
    let session_id = opt_string(&mut fbb, &session_id_str);
    let request_id = opt_string(&mut fbb, &request_id_str);
    let trace_id = opt_string(&mut fbb, &trace_id_str);
    let span_id = opt_string(&mut fbb, &span_id_str);
    let exception_type = opt_string(&mut fbb, &exception_type_str);
    let exception_message = opt_string(&mut fbb, &exception_message_str);
    let exception_stacktrace = opt_string(&mut fbb, &exception_stacktrace_str);
    let labels = opt_string(&mut fbb, &labels_str);
    let audit_tags = opt_string(&mut fbb, &audit_tags_str);
    let ext_data = kv_envelope_text
        .as_deref()
        .and_then(|value| opt_string(&mut fbb, value));

    let args = RecordArgs {
        id_hi: record.id_hi(),
        id_lo: record.id_lo(),
        timestamp_hi: record.timestamp_secs() as u64,
        timestamp_lo: record.timestamp_subsec_nanos() as u64,
        signature,
        origin_lsn: 0, // removed; no longer stored on Record
        level: record.level as u8,
        message,
        source_file,
        source_function,
        source_line: record.source_line(),
        source_column: record.source_column(),
        thread_id: record.thread_id as u64,
        thread_name,
        process_id: record.process_id,
        process_name,
        host_name,
        container_id,
        app_name,
        app_version,
        environment,
        user_id,
        session_id,
        request_id,
        trace_id,
        span_id,
        coroutine_id: record.coroutine_id(),
        exception_type,
        exception_message,
        exception_stacktrace,
        exception_code: record.exception_code() as i32,
        labels,
        lsn: record.lsn,
        prev_hash,
        security_gap: record.security_gap(),
        audit_tags,
        ext_data,
        ext_crc32c: 0, // removed
        pool_index: record.pool_index,
        flags: record.flags as u32 | message_kind_flags,
        content_hash,
    };

    let root = SifRecord::create(&mut fbb, &args);
    finish_record_buffer(&mut fbb, root);

    let payload = fbb.finished_data();
    let total_length = FLATBUFFERS_FRAME_OVERHEAD + payload.len();
    let version = super::FLATBUFFERS_SCHEMA_VERSION;

    let mut out = Vec::with_capacity(total_length);
    out.extend_from_slice(&FLATBUFFERS_SIF_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(total_length as u32).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(payload);
    debug_assert_eq!(out.len(), total_length);
    out
}

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as usize;
        output.push(ALPHABET[first >> 2] as char);
        let second = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        output.push(ALPHABET[((first & 0x03) << 4) | (second >> 4)] as char);
        if chunk.len() > 1 {
            let third = if chunk.len() > 2 {
                chunk[2] as usize
            } else {
                0
            };
            output.push(ALPHABET[((second & 0x0f) << 2) | (third >> 6)] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[chunk[2] as usize & 0x3f] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn encode_kv_envelope(record: &Record) -> Vec<u8> {
    let mut out = Vec::new();
    let mut append = |slot: &crate::record::KvSlot| {
        let Some((tag, ty, value)) = slot.wire_value() else {
            return;
        };
        let name = crate::sif::codec::field_name_for_tag(tag);
        let Ok(name_len) = u16::try_from(name.len()) else {
            return;
        };
        let Ok(value_len) = u32::try_from(value.len()) else {
            return;
        };
        out.push(tag);
        out.push(ty);
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&value_len.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(value);
    };
    append(&record.kv0);
    append(&record.kv1);
    if let Some(ext) = record.kv_ext() {
        for slot in ext {
            append(slot);
        }
    }
    out
}

/// Encode with the backend resource budget enforced at the public boundary.
pub fn encode_record_checked(record: &Record) -> Result<Vec<u8>, super::decoder::SifError> {
    let frame = encode_record(record);
    if frame.len() > super::decoder::MAX_FLATBUFFERS_FRAME_SIZE {
        return Err(super::decoder::SifError::Oversize {
            found: frame.len(),
            max: super::decoder::MAX_FLATBUFFERS_FRAME_SIZE,
        });
    }
    Ok(frame)
}

/// Create a FlatBuffer string offset, or `None` for empty strings so omitted
/// fields stay absent on the wire (FlatBuffers convention).
fn opt_string<'b>(fbb: &mut FlatBufferBuilder<'b>, s: &str) -> Option<WIPOffset<&'b str>> {
    if s.is_empty() {
        None
    } else {
        Some(fbb.create_string(s))
    }
}
