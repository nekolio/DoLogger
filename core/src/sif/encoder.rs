//! SIF encoder — `Record` → SIF frame.
//!
//! The encoder is the serialisation half of the SIF pipeline stage:
//! [`encode_record`] materialises a [`Record`] into a complete framed SIF
//! message, ready for zero-copy consumption by a Sink.

use crate::record::Record;
use crate::sif::generated::{finish_record_buffer, Record as SifRecord, RecordArgs};
use crate::sif::{SifHeader, SIF_INITIAL_BUFFER_SIZE, SIF_MAGIC, SIF_MAX_PREAMBLE};
use flatbuffers::{FlatBufferBuilder, WIPOffset};

/// Encode a [`Record`] into a complete SIF frame.
///
/// # Frame layout
///
/// | Offset | Size | Field                          |
/// |--------|------|--------------------------------|
/// | 0      | 4    | `SIF_MAGIC` (`"SIF1"`)         |
/// | 4      | 12   | `SifHeader` (three LE u32s)     |
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
        SIF_INITIAL_BUFFER_SIZE + SIF_MAX_PREAMBLE + record.message.len(),
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

    let message = opt_string(&mut fbb, record.message.as_str());
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
        ext_data: None, // removed; moved to KV vendor
        ext_crc32c: 0,  // removed
        pool_index: record.pool_index,
        flags: record.flags as u32,
        content_hash,
    };

    let root = SifRecord::create(&mut fbb, &args);
    finish_record_buffer(&mut fbb, root);

    let payload = fbb.finished_data();
    let total_length = SifHeader::FRAME_OVERHEAD + payload.len();
    let header = SifHeader::new(total_length as u32, 1);

    let mut out = Vec::with_capacity(total_length);
    out.extend_from_slice(&SIF_MAGIC);
    out.extend_from_slice(&header.version.to_le_bytes());
    out.extend_from_slice(&header.total_length.to_le_bytes());
    out.extend_from_slice(&header.record_count.to_le_bytes());
    out.extend_from_slice(payload);
    debug_assert_eq!(out.len(), total_length);
    out
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
