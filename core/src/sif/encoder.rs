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

    let signature = Some(fbb.create_vector(&record.signature));
    let prev_hash = Some(fbb.create_vector(&record.prev_hash));

    let message = opt_string(&mut fbb, record.message.as_str());
    let source_file = opt_string(&mut fbb, record.source_file.as_str());
    let source_function = opt_string(&mut fbb, record.source_function.as_str());
    let thread_name = opt_string(&mut fbb, record.thread_name.as_str());
    let process_name = opt_string(&mut fbb, record.process_name.as_str());
    let host_name = opt_string(&mut fbb, record.host_name.as_str());
    let container_id = opt_string(&mut fbb, record.container_id.as_str());
    let app_name = opt_string(&mut fbb, record.app_name.as_str());
    let app_version = opt_string(&mut fbb, record.app_version.as_str());
    let environment = opt_string(&mut fbb, record.environment.as_str());
    let user_id = opt_string(&mut fbb, record.user_id.as_str());
    let session_id = opt_string(&mut fbb, record.session_id.as_str());
    let request_id = opt_string(&mut fbb, record.request_id.as_str());
    let trace_id = opt_string(&mut fbb, record.trace_id.as_str());
    let span_id = opt_string(&mut fbb, record.span_id.as_str());
    let exception_type = opt_string(&mut fbb, record.exception_type.as_str());
    let exception_message = opt_string(&mut fbb, record.exception_message.as_str());
    let exception_stacktrace = opt_string(&mut fbb, record.exception_stacktrace.as_str());
    let labels = opt_string(&mut fbb, record.labels.as_str());
    let audit_tags = opt_string(&mut fbb, record.audit_tags.as_str());
    let ext_data = opt_string(&mut fbb, record.ext_data.as_str());

    let args = RecordArgs {
        id_hi: record.id.hi,
        id_lo: record.id.lo,
        timestamp_hi: record.timestamp.hi,
        timestamp_lo: record.timestamp.lo,
        signature,
        origin_lsn: record.origin_lsn,
        level: record.level as u8,
        message,
        source_file,
        source_function,
        source_line: record.source_line,
        source_column: record.source_column,
        thread_id: record.thread_id,
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
        coroutine_id: record.coroutine_id,
        exception_type,
        exception_message,
        exception_stacktrace,
        exception_code: record.exception_code,
        labels,
        lsn: record.lsn,
        prev_hash,
        security_gap: record.security_gap,
        audit_tags,
        ext_data,
        ext_crc32c: record.ext_crc32c,
        pool_index: record.pool_index,
        flags: record.flags,
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
