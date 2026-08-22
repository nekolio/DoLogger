//! FlatBuffers implementation of the SIF boundary.
//!
//! This module is deliberately explicit. It does not replace the native
//! KV-backed codec and it does not make FlatBuffers mandatory for in-process
//! sinks. Both implementations serialize the same [`Record`] model.

mod generated {
    #![allow(
        dead_code,
        missing_docs,
        unused,
        unused_extern_crates,
        unused_imports,
        unused_qualifications,
        non_camel_case_types,
        elided_lifetimes_in_paths,
        explicit_outlives_requirements,
        clippy::all,
        clippy::undocumented_unsafe_blocks,
        unsafe_op_in_unsafe_fn
    )]
    include!("dologger_sif_generated.rs");
}

/// Four-byte marker for a FlatBuffers-backed SIF frame.
pub const FLATBUFFERS_SIF_MAGIC: [u8; 4] = *b"SIFB";
/// Complete framing overhead before the FlatBuffers payload.
pub const FLATBUFFERS_FRAME_OVERHEAD: usize = 16;
/// Current FlatBuffers schema contract marker.
pub const FLATBUFFERS_SCHEMA_VERSION: u32 = 1;
/// Initial FlatBuffers builder capacity.
pub const FLATBUFFERS_INITIAL_BUFFER_SIZE: usize = 4096;
/// Reserved builder preamble capacity.
pub const FLATBUFFERS_MAX_PREAMBLE: usize = 256;

/// Framing metadata carried by a FlatBuffers-backed SIF frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlatbuffersHeader {
    /// Backend schema contract marker.
    pub version: u32,
    /// Complete frame length, including marker and header.
    pub total_length: u32,
    /// Number of records in the payload.
    pub record_count: u32,
}

impl FlatbuffersHeader {
    /// Header and marker size before the FlatBuffers payload.
    pub const FRAME_OVERHEAD: usize = FLATBUFFERS_FRAME_OVERHEAD;
}

#[path = "flatbuffers_decoder.rs"]
mod decoder;
#[path = "flatbuffers_encoder.rs"]
mod encoder;

pub use decoder::{decode_record, decode_record_compat, validate_frame};
pub use decoder::{SifCompatibility as FlatbuffersCompatibility, SifError as FlatbuffersError};
pub use encoder::{encode_record, encode_record_checked};

/// Encode a Record with the FlatBuffers SIF backend.
pub fn encode(record: &crate::record::Record) -> Result<Vec<u8>, FlatbuffersError> {
    encode_record_checked(record)
}

/// Decode a FlatBuffers-backed SIF frame.
pub fn decode(frame: &[u8]) -> Result<crate::record::Record, FlatbuffersError> {
    decode_record(frame)
}
