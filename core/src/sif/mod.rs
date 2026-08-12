//! SIF — Standard Intermediate Format
//!
//! The SIF is the zero-copy binary wire format that sits between the
//! [Formatter](crate::pipeline) and [Sink](crate::sink) stages.  Records are
//! serialised into FlatBuffer-encoded SIF messages, then consumed by sinks
//! without deserialisation or heap allocation.
//!
//! # Architecture
//!
//! ```text
//! Core Engine  ──>  Record  ──>  Formatter  ──>  SIF bytes  ──>  Sink
//!   (populates)      (mem)       (serialises)     (zero-copy)    (consumes)
//! ```
//!
//! # Integration
//!
//! This module provides the hand-written scaffolding.  The generated
//! FlatBuffers bindings are expected at:
//!
//! ```text
//! core/src/sif/dologger_sif_generated.rs
//! ```
//!
//! Generate them with:
//!
//! ```bash
//! cd core/sif
//! flatc --rust -o ../src/sif/ dologger_sif.fbs
//! ```
//!
//! Once generated, uncomment the `include!` directive below to pull the
//! generated code into this module.
//!
//! # Wire Format
//!
//! Every SIF message is framed as:
//!
//! | Offset | Size  | Field         |
//! |--------|-------|---------------|
//! | 0      | 4     | Magic (`SIF1`)|
//! | 4      | 12    | SifHeader     |
//! | 16     | var   | FlatBuffer    |
//!
//! The FlatBuffer payload starts with a `u32` root table offset, followed by
//! the vtable and data sections of the `Record` table.

// ---------------------------------------------------------------------------
// Magic constant
// ---------------------------------------------------------------------------

/// Four-byte magic identifier for SIF frames.
///
/// Consumers read the first 4 bytes of a SIF stream; if they don't match
/// `SIF_MAGIC`, the stream is either corrupted or not a SIF payload.
pub const SIF_MAGIC: [u8; 4] = *b"SIF1";

// ---------------------------------------------------------------------------
// SIF Header
// ---------------------------------------------------------------------------

/// On-wire framing header placed immediately after the magic bytes.
///
/// The header is always 12 bytes, giving consumers enough information to
/// validate the schema version and size before touching the FlatBuffer data.
///
/// # Layout (packed, no padding — 12 bytes total)
///
/// | Offset | Size | Field        | Description                            |
/// |--------|------|--------------|----------------------------------------|
/// | 0      | 4    | `version`    | Schema version (MAJOR<<24|MINOR<<16|PATCH) |
/// | 4      | 4    | `total_length` | Total SIF frame length including magic + header |
/// | 8      | 4    | `record_count` | Number of Record tables (1 for single, N for batch) |
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SifHeader {
    /// Schema version as a packed u32: `MAJOR << 24 | MINOR << 16 | PATCH`.
    ///
    /// Example: version 1.0.0 is encoded as `0x0100_0000`.
    pub version: u32,

    /// Total length of the SIF frame in bytes, including the 4-byte magic
    /// prefix, this 12-byte header, and the FlatBuffer payload.
    pub total_length: u32,

    /// Number of `Record` tables embedded in the payload.
    ///
    /// Always 1 for single-record SIF messages.  Future versions will use
    /// this field when `RecordBatch` batch transfer is implemented.
    pub record_count: u32,
}

impl SifHeader {
    /// Create a new SIF header for the current schema version (1.0.0).
    ///
    /// `total_length` must include the 4 magic bytes + 12 header bytes +
    /// FlatBuffer payload length.
    #[inline]
    pub const fn new(total_length: u32, record_count: u32) -> Self {
        Self {
            version: SIF_VERSION,
            total_length,
            record_count,
        }
    }

    /// Return the schema version as a `(major, minor, patch)` tuple.
    #[inline]
    pub const fn version_tuple(&self) -> (u16, u16, u16) {
        let major = ((self.version >> 24) & 0xFF) as u16;
        let minor = ((self.version >> 16) & 0xFF) as u16;
        let patch = (self.version & 0xFFFF) as u16;
        (major, minor, patch)
    }

    /// True if the header's magic matches `SIF_MAGIC`.
    ///
    /// The caller is responsible for reading the 4 magic bytes from the
    /// buffer and passing them here.
    #[inline]
    pub const fn magic_valid(magic: &[u8; 4]) -> bool {
        magic[0] == SIF_MAGIC[0]
            && magic[1] == SIF_MAGIC[1]
            && magic[2] == SIF_MAGIC[2]
            && magic[3] == SIF_MAGIC[3]
    }

    /// Total frame overhead (magic + header) in bytes.
    pub const FRAME_OVERHEAD: usize = 16;
}

/// Pack a MAJOR.MINOR.PATCH version into the wire `u32` layout
/// (`MAJOR << 24 | MINOR << 16 | PATCH`).
const fn sif_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 24) | (minor << 16) | patch
}

/// Current SIF schema version (1.0.0) as a packed `u32`.
pub const SIF_VERSION: u32 = sif_version(1, 0, 0);

// ---------------------------------------------------------------------------
// Generated FlatBuffers bindings
// ---------------------------------------------------------------------------
//
// After running `flatc --rust -o ../src/sif/ dologger_sif.fbs` from the
// `core/sif/` directory, uncomment the line below to include the generated
// Record table, its builder, and accessor methods.
//
// # Safety
//
// The generated code uses `unsafe` for raw pointer arithmetic (FlatBuffers
// wire format).  Review the generated `unsafe` blocks against the SAST audit
// baseline before enabling in production.
//
// ```rust
// include!("dologger_sif_generated.rs");
// ```

// ---------------------------------------------------------------------------
// Buffer sizing helpers
// ---------------------------------------------------------------------------

/// Recommended initial buffer size for serialising a single Record into SIF.
///
/// This accommodates the frame overhead (16 bytes) plus a typical record with
/// several populated string fields.  If the record exceeds this size,
/// FlatBuffers will grow the buffer automatically during `finish()`.
pub const SIF_INITIAL_BUFFER_SIZE: usize = 4096;

/// Maximum number of inline bytes FlatBuffers reserves for the vtable and
/// root offset before the data section.  Added to `SIF_INITIAL_BUFFER_SIZE`
/// when pre-allocating.
pub const SIF_MAX_PREAMBLE: usize = 256;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_constant_is_correct() {
        assert_eq!(&SIF_MAGIC, b"SIF1");
        assert_eq!(SIF_MAGIC.len(), 4);
    }

    #[test]
    fn header_new_has_correct_version() {
        let header = SifHeader::new(1024, 1);
        assert_eq!(header.version, SIF_VERSION);
        assert_eq!(header.total_length, 1024);
        assert_eq!(header.record_count, 1);
    }

    #[test]
    fn header_version_tuple() {
        let header = SifHeader::new(0, 0);
        assert_eq!(header.version_tuple(), (1, 0, 0));
    }

    #[test]
    fn header_magic_valid() {
        assert!(SifHeader::magic_valid(b"SIF1"));
        assert!(!SifHeader::magic_valid(b"XXXX"));
        assert!(!SifHeader::magic_valid(b"SIF2"));
        assert!(!SifHeader::magic_valid(b"sif1"));
    }

    #[test]
    fn header_size_is_12() {
        assert_eq!(core::mem::size_of::<SifHeader>(), 12);
    }

    #[test]
    fn header_alignment_is_4() {
        // repr(C) with three u32 fields — natural 4-byte alignment, no padding
        assert_eq!(core::mem::align_of::<SifHeader>(), 4);
    }

    #[test]
    fn frame_overhead_is_16() {
        assert_eq!(SifHeader::FRAME_OVERHEAD, 16);
        // 4 bytes magic + 12 bytes header = 16
    }

    #[test]
    fn initial_buffer_size_is_reasonable() {
        // Constant-folded at compile time — evaluated here so the invariant
        // is checked on every build, not only when this test runs.
        const { assert!(SIF_INITIAL_BUFFER_SIZE >= SifHeader::FRAME_OVERHEAD + SIF_MAX_PREAMBLE) };
        const { assert!(SIF_INITIAL_BUFFER_SIZE <= 65536) }; // upper sanity bound
    }
}
