//! Standard Intermediate Format (SIF) serialization.
//!
//! SIF is DoLogger's neutral byte boundary for records that leave the Rust
//! process. The in-memory [`Record`](crate::record::Record) uses fixed hot
//! fields plus dynamic KV fields; this module serializes that model into one
//! bounded, cross-platform SIF frame.
//!
//! SIF is not a logging sink and it is not a display encoding. A sink may use
//! it for process, shared-memory, file, or plugin transport, while in-process
//! sinks may keep using `Record` or an immutable derived view directly.

mod codec;

pub use codec::{
    decode_record, decode_record_with, encode_length_prefixed, encode_record, entries,
    validate_frame, validate_frame_with, DecodeOptions, FrameScanner, KvEntry, ReusableEncoder,
    SifError, SifFrameHeader, MAX_FIELD_COUNT, MAX_FIELD_NAME, MAX_FIELD_VALUE, MAX_FRAME_SIZE,
    MAX_MESSAGE_SIZE, SIF_FIXED_LEN, SIF_HASH_OFFSET, SIF_HEADER_LEN, SIF_MAGIC,
    SIF_MESSAGE_LEN_OFFSET,
};
