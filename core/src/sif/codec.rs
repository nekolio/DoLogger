//! SIF serialization built from the Record KV model.
//!
//! The frame is intentionally owned by the SIF module: KV describes the
//! dynamic data organization inside a Record, while SIF describes the
//! cross-platform byte boundary used by sinks and external consumers.
//!
//! Frame layout (all integers little-endian):
//!
//! ```text
//! header(32) | fixed metadata(90) | UTF-8 message | repeated KV entries
//! ```
//!
//! Each entry contains a numeric tag, closed-set type, UTF-8 field name, and
//! payload bytes. Bounds are checked before every slice so untrusted SHM input
//! cannot cause an out-of-bounds read or an allocation proportional to a forged
//! length.

use std::collections::HashSet;
use std::fmt;

use crate::record::kv::{KvPutError, KvSlot, KvType, KV_TAG_CORE_MAX};
use crate::record::{LogLevel, MessagePayloadKind, Record};

/// Four-byte identifier for a SIF frame.
pub const SIF_MAGIC: [u8; 4] = *b"SIF\0";
/// Fixed header size.
pub const SIF_HEADER_LEN: usize = 32;
/// Fixed metadata size, including the message length word.
pub const SIF_FIXED_LEN: usize = 90;
/// Offset of the content hash relative to the fixed metadata start.
pub const SIF_HASH_OFFSET: usize = 54;
/// Offset of the message length relative to the fixed metadata start.
pub const SIF_MESSAGE_LEN_OFFSET: usize = 86;
/// Maximum accepted frame size.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
/// Maximum number of dynamic fields.
pub const MAX_FIELD_COUNT: usize = 1024;
/// Maximum field-name length.
pub const MAX_FIELD_NAME: usize = 1024;
/// Maximum message length.
pub const MAX_MESSAGE_SIZE: usize = 8 * 1024 * 1024;
/// Maximum individual value length.
pub const MAX_FIELD_VALUE: usize = 8 * 1024 * 1024;
/// Frame contains a non-zero content hash.
pub const SIF_FLAG_CONTENT_HASH: u16 = 0x0001;
/// Frame contains an AUDIT-level record.
pub const SIF_FLAG_AUDIT: u16 = 0x0002;
/// Frame was emitted by the canonical writer.
pub const SIF_FLAG_CANONICAL: u16 = 0x0004;
/// Frame message contains bytes that were not validated as text.
pub const SIF_FLAG_MESSAGE_BINARY: u16 = 0x0008;
/// Frame message contains text produced by explicit decoding.
pub const SIF_FLAG_MESSAGE_EXPLICIT_TEXT: u16 = 0x0010;

/// Validation and codec failures for SIF frames.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum SifError {
    /// Input ended before a complete value was available.
    Truncated { offset: usize, needed: usize },
    /// Input does not begin with the SIF magic bytes.
    InvalidMagic,
    /// A fixed protocol length is inconsistent.
    InvalidHeaderLength { found: usize, expected: usize },
    /// Header length differs from the supplied buffer.
    LengthMismatch { declared: usize, actual: usize },
    /// A resource limit was exceeded.
    LengthExceeded {
        field: &'static str,
        value: usize,
        max: usize,
    },
    /// Too many fields were supplied.
    TooManyFields { found: usize, max: usize },
    /// A tag appears more than once.
    DuplicateTag(u8),
    /// A field name or message is not UTF-8.
    InvalidUtf8,
    /// Message kind flags contain an impossible combination.
    InvalidMessageKind,
    /// A field name violates the portable key contract.
    InvalidFieldName,
    /// A value type is outside the closed set.
    UnknownType(u8),
    /// The level byte is invalid.
    InvalidLevel(u8),
    /// A slot could not be restored.
    FieldApply { name: String },
    /// A verified content hash differs from canonical Record bytes.
    ContentHashMismatch,
    /// A Rust length does not fit in a wire integer.
    EncodeOverflow(&'static str),
}

impl fmt::Display for SifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { offset, needed } => {
                write!(f, "truncated SIF frame at {offset}, need {needed} bytes")
            }
            Self::InvalidMagic => f.write_str("invalid SIF frame magic"),
            Self::InvalidHeaderLength { found, expected } => {
                write!(f, "invalid SIF length {found}, expected {expected}")
            }
            Self::LengthMismatch { declared, actual } => write!(
                f,
                "SIF length mismatch: declared {declared}, actual {actual}"
            ),
            Self::LengthExceeded { field, value, max } => {
                write!(f, "SIF {field} length {value} exceeds {max}")
            }
            Self::TooManyFields { found, max } => {
                write!(f, "SIF field count {found} exceeds {max}")
            }
            Self::DuplicateTag(tag) => write!(f, "duplicate KV tag {tag}"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8 in SIF frame"),
            Self::InvalidMessageKind => f.write_str("invalid KV message kind flags"),
            Self::InvalidFieldName => f.write_str("invalid KV field name"),
            Self::UnknownType(ty) => write!(f, "unknown KV type {ty}"),
            Self::InvalidLevel(level) => write!(f, "invalid log level {level}"),
            Self::FieldApply { name } => write!(f, "cannot apply KV field {name}"),
            Self::ContentHashMismatch => f.write_str("KV content hash mismatch"),
            Self::EncodeOverflow(field) => write!(f, "KV {field} cannot be represented on wire"),
        }
    }
}

impl std::error::Error for SifError {}

/// Decode limits and integrity policy.
#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    /// Maximum accepted complete frame size.
    pub max_frame_size: usize,
    /// Maximum number of dynamic entries.
    pub max_field_count: usize,
    /// Verify a non-zero content hash after decoding.
    pub verify_content_hash: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_frame_size: MAX_FRAME_SIZE,
            max_field_count: MAX_FIELD_COUNT,
            verify_content_hash: false,
        }
    }
}

impl DecodeOptions {
    /// Options for untrusted shared-memory frames.
    pub const fn untrusted() -> Self {
        Self {
            max_frame_size: MAX_FRAME_SIZE,
            max_field_count: MAX_FIELD_COUNT,
            verify_content_hash: true,
        }
    }

    /// Options for audit replay.
    pub const fn audit() -> Self {
        Self::untrusted()
    }
}

/// Metadata validated from a frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SifFrameHeader {
    /// Header flags.
    pub flags: u16,
    /// Complete frame length.
    pub total_length: usize,
    /// Number of dynamic entries.
    pub field_count: usize,
    /// Fixed metadata length.
    pub fixed_length: usize,
}

/// Borrowed dynamic entry for inspection and replay tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvEntry<'a> {
    /// Numeric KV tag.
    pub tag: u8,
    /// Closed-set value type byte.
    pub ty: u8,
    /// Canonical field name.
    pub name: &'a str,
    /// Borrowed payload bytes.
    pub value: &'a [u8],
}

/// Validate a frame with default bounds.
pub fn validate_frame(frame: &[u8]) -> Result<SifFrameHeader, SifError> {
    validate_frame_with(frame, DecodeOptions::default())
}

/// Validate a frame with explicit bounds.
pub fn validate_frame_with(
    frame: &[u8],
    options: DecodeOptions,
) -> Result<SifFrameHeader, SifError> {
    if frame.len() > options.max_frame_size {
        return Err(SifError::LengthExceeded {
            field: "frame",
            value: frame.len(),
            max: options.max_frame_size,
        });
    }
    if frame.len() < SIF_HEADER_LEN {
        return Err(SifError::Truncated {
            offset: frame.len(),
            needed: SIF_HEADER_LEN - frame.len(),
        });
    }
    if frame[..4] != SIF_MAGIC {
        return Err(SifError::InvalidMagic);
    }
    let header_len = read_u16(frame, 4)? as usize;
    if header_len != SIF_HEADER_LEN {
        return Err(SifError::InvalidHeaderLength {
            found: header_len,
            expected: SIF_HEADER_LEN,
        });
    }
    let flags = read_u16(frame, 6)?;
    if flags & SIF_FLAG_MESSAGE_BINARY != 0 && flags & SIF_FLAG_MESSAGE_EXPLICIT_TEXT != 0 {
        return Err(SifError::InvalidMessageKind);
    }
    let total_length = read_u32(frame, 8)? as usize;
    if total_length != frame.len() {
        return Err(SifError::LengthMismatch {
            declared: total_length,
            actual: frame.len(),
        });
    }
    if total_length > options.max_frame_size {
        return Err(SifError::LengthExceeded {
            field: "declared frame",
            value: total_length,
            max: options.max_frame_size,
        });
    }
    let field_count = read_u32(frame, 12)? as usize;
    if field_count > options.max_field_count {
        return Err(SifError::TooManyFields {
            found: field_count,
            max: options.max_field_count,
        });
    }
    let fixed_length = read_u32(frame, 16)? as usize;
    if fixed_length != SIF_FIXED_LEN {
        return Err(SifError::InvalidHeaderLength {
            found: fixed_length,
            expected: SIF_FIXED_LEN,
        });
    }
    let minimum = SIF_HEADER_LEN
        .checked_add(fixed_length)
        .ok_or(SifError::LengthMismatch {
            declared: total_length,
            actual: frame.len(),
        })?;
    if total_length < minimum {
        return Err(SifError::Truncated {
            offset: total_length,
            needed: minimum - total_length,
        });
    }
    Ok(SifFrameHeader {
        flags,
        total_length,
        field_count,
        fixed_length,
    })
}

/// Encode one Record as a SIF frame whose dynamic fields are serialized as KV entries.
pub fn encode_record(record: &Record) -> Result<Vec<u8>, SifError> {
    let fields = collect_entries(record);
    if fields.len() > MAX_FIELD_COUNT {
        return Err(SifError::TooManyFields {
            found: fields.len(),
            max: MAX_FIELD_COUNT,
        });
    }
    let message = record.message.as_bytes();
    if message.len() > MAX_MESSAGE_SIZE {
        return Err(SifError::LengthExceeded {
            field: "message",
            value: message.len(),
            max: MAX_MESSAGE_SIZE,
        });
    }
    let mut flags = SIF_FLAG_CANONICAL;
    if record.level == LogLevel::Audit {
        flags |= SIF_FLAG_AUDIT;
    }
    if record.content_hash != [0; 32] {
        flags |= SIF_FLAG_CONTENT_HASH;
    }
    match record.message.kind() {
        MessagePayloadKind::Utf8 => {}
        MessagePayloadKind::Binary => flags |= SIF_FLAG_MESSAGE_BINARY,
        MessagePayloadKind::ExplicitDecodedText => flags |= SIF_FLAG_MESSAGE_EXPLICIT_TEXT,
    }
    let mut frame = Vec::with_capacity(SIF_HEADER_LEN + SIF_FIXED_LEN + message.len());
    frame.extend_from_slice(&SIF_MAGIC);
    frame.extend_from_slice(&(SIF_HEADER_LEN as u16).to_le_bytes());
    frame.extend_from_slice(&flags.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    frame.extend_from_slice(&(SIF_FIXED_LEN as u32).to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&record.id_hi().to_le_bytes());
    frame.extend_from_slice(&record.id_lo().to_le_bytes());
    frame.extend_from_slice(&record.timestamp.to_le_bytes());
    frame.push(record.level as u8);
    frame.extend_from_slice(&[0; 7]);
    frame.extend_from_slice(&record.thread_id.to_le_bytes());
    frame.extend_from_slice(&record.process_id.to_le_bytes());
    frame.extend_from_slice(&record.lsn.to_le_bytes());
    frame.extend_from_slice(&record.flags.to_le_bytes());
    frame.extend_from_slice(&record.pool_index.to_le_bytes());
    frame.extend_from_slice(&record.content_hash);
    frame.extend_from_slice(&(message.len() as u32).to_le_bytes());
    frame.extend_from_slice(message);
    for field in fields {
        if field.name.len() > MAX_FIELD_NAME {
            return Err(SifError::LengthExceeded {
                field: "field name",
                value: field.name.len(),
                max: MAX_FIELD_NAME,
            });
        }
        if field.value.len() > MAX_FIELD_VALUE {
            return Err(SifError::LengthExceeded {
                field: "field value",
                value: field.value.len(),
                max: MAX_FIELD_VALUE,
            });
        }
        let name_len =
            u16::try_from(field.name.len()).map_err(|_| SifError::EncodeOverflow("field name"))?;
        let value_len = u32::try_from(field.value.len())
            .map_err(|_| SifError::EncodeOverflow("field value"))?;
        frame.push(field.tag);
        frame.push(field.ty);
        frame.extend_from_slice(&name_len.to_le_bytes());
        frame.extend_from_slice(&value_len.to_le_bytes());
        frame.extend_from_slice(field.name.as_bytes());
        frame.extend_from_slice(&field.value);
    }
    let total = u32::try_from(frame.len()).map_err(|_| SifError::EncodeOverflow("frame"))?;
    frame[8..12].copy_from_slice(&total.to_le_bytes());
    if frame.len() > MAX_FRAME_SIZE {
        return Err(SifError::LengthExceeded {
            field: "frame",
            value: frame.len(),
            max: MAX_FRAME_SIZE,
        });
    }
    Ok(frame)
}

/// Decode a frame with the default compatibility policy.
pub fn decode_record(frame: &[u8]) -> Result<Record, SifError> {
    decode_record_with(frame, DecodeOptions::default())
}

/// Decode a frame with explicit resource and integrity policy.
pub fn decode_record_with(frame: &[u8], options: DecodeOptions) -> Result<Record, SifError> {
    let header = validate_frame_with(frame, options)?;
    let fixed_start = SIF_HEADER_LEN;
    let fixed_end = fixed_start + header.fixed_length;
    let id_hi = read_u64(frame, fixed_start)?;
    let id_lo = read_u64(frame, fixed_start + 8)?;
    let timestamp = read_u64(frame, fixed_start + 16)?;
    let level_raw = *frame.get(fixed_start + 24).ok_or(SifError::Truncated {
        offset: fixed_start + 24,
        needed: 1,
    })?;
    let level = LogLevel::from_u8(level_raw).ok_or(SifError::InvalidLevel(level_raw))?;
    let thread_id = read_u32(frame, fixed_start + 32)?;
    let process_id = read_u32(frame, fixed_start + 36)?;
    let lsn = read_u64(frame, fixed_start + 40)?;
    let flags = read_u16(frame, fixed_start + 48)?;
    let pool_index = read_u32(frame, fixed_start + 50)?;
    let content_hash = read_array::<32>(frame, fixed_start + SIF_HASH_OFFSET)?;
    let message_len = read_u32(frame, fixed_start + SIF_MESSAGE_LEN_OFFSET)? as usize;
    if message_len > MAX_MESSAGE_SIZE
        || fixed_end.checked_add(message_len).is_none()
        || fixed_end + message_len > frame.len()
    {
        return Err(SifError::LengthExceeded {
            field: "message",
            value: message_len,
            max: MAX_MESSAGE_SIZE,
        });
    }
    let message_end = fixed_end + message_len;
    let message = &frame[fixed_end..message_end];
    let message_kind =
        match header.flags & (SIF_FLAG_MESSAGE_BINARY | SIF_FLAG_MESSAGE_EXPLICIT_TEXT) {
            0 => MessagePayloadKind::Utf8,
            SIF_FLAG_MESSAGE_BINARY => MessagePayloadKind::Binary,
            SIF_FLAG_MESSAGE_EXPLICIT_TEXT => MessagePayloadKind::ExplicitDecodedText,
            _ => return Err(SifError::InvalidMessageKind),
        };
    if message_kind != MessagePayloadKind::Binary && std::str::from_utf8(message).is_err() {
        return Err(SifError::InvalidUtf8);
    }
    let mut record = Record::new(pool_index);
    record.set_id(id_hi, id_lo);
    record.timestamp = timestamp;
    record.level = level;
    record.thread_id = thread_id;
    record.process_id = process_id;
    record.lsn = lsn;
    record.flags = flags;
    record.content_hash = content_hash;
    match message_kind {
        MessagePayloadKind::Utf8 => record
            .message
            .set_utf8_bytes(message)
            .map_err(|_| SifError::InvalidUtf8)?,
        MessagePayloadKind::Binary => record.message.set_bytes(message),
        MessagePayloadKind::ExplicitDecodedText => {
            let text = std::str::from_utf8(message).map_err(|_| SifError::InvalidUtf8)?;
            record.message.set_explicit_decoded_text(text);
        }
    }
    let mut cursor = message_end;
    let mut tags = HashSet::with_capacity(header.field_count);
    for _ in 0..header.field_count {
        let tag = *frame.get(cursor).ok_or(SifError::Truncated {
            offset: cursor,
            needed: 1,
        })?;
        cursor += 1;
        let ty = *frame.get(cursor).ok_or(SifError::Truncated {
            offset: cursor,
            needed: 1,
        })?;
        cursor += 1;
        if KvType::from_u8(ty).is_none() {
            return Err(SifError::UnknownType(ty));
        }
        let name_len = read_u16(frame, cursor)? as usize;
        cursor += 2;
        let value_len = read_u32(frame, cursor)? as usize;
        cursor += 4;
        if tag == 0 || !tags.insert(tag) {
            return Err(SifError::DuplicateTag(tag));
        }
        if name_len == 0 || name_len > MAX_FIELD_NAME {
            return Err(SifError::InvalidFieldName);
        }
        if value_len > MAX_FIELD_VALUE {
            return Err(SifError::LengthExceeded {
                field: "field value",
                value: value_len,
                max: MAX_FIELD_VALUE,
            });
        }
        let name_end = cursor
            .checked_add(name_len)
            .ok_or(SifError::LengthMismatch {
                declared: frame.len(),
                actual: cursor,
            })?;
        let value_end = name_end
            .checked_add(value_len)
            .ok_or(SifError::LengthMismatch {
                declared: frame.len(),
                actual: cursor,
            })?;
        if value_end > frame.len() {
            return Err(SifError::Truncated {
                offset: cursor,
                needed: value_end - frame.len(),
            });
        }
        let name =
            std::str::from_utf8(&frame[cursor..name_end]).map_err(|_| SifError::InvalidUtf8)?;
        validate_name(name)?;
        if tag > KV_TAG_CORE_MAX {
            register_vendor_tag(name, tag);
        }
        record.put_wire_slot(tag, ty, &frame[name_end..value_end])?;
        cursor = value_end;
    }
    if cursor != frame.len() {
        return Err(SifError::LengthMismatch {
            declared: frame.len(),
            actual: cursor,
        });
    }
    if options.verify_content_hash
        && content_hash != [0; 32]
        && Record::compute_content_hash_from(&record) != content_hash
    {
        return Err(SifError::ContentHashMismatch);
    }
    Ok(record)
}

/// Return zero-copy entry views for inspection and replay tooling.
pub fn entries(frame: &[u8]) -> Result<Vec<KvEntry<'_>>, SifError> {
    let header = validate_frame(frame)?;
    let fixed_start = SIF_HEADER_LEN;
    let fixed_end = fixed_start + header.fixed_length;
    let message_len = read_u32(frame, fixed_start + SIF_MESSAGE_LEN_OFFSET)? as usize;
    let mut cursor = fixed_end
        .checked_add(message_len)
        .ok_or(SifError::LengthMismatch {
            declared: frame.len(),
            actual: fixed_end,
        })?;
    if cursor > frame.len() {
        return Err(SifError::Truncated {
            offset: fixed_end,
            needed: cursor - frame.len(),
        });
    }
    let mut result = Vec::with_capacity(header.field_count);
    let mut tags = HashSet::with_capacity(header.field_count);
    for _ in 0..header.field_count {
        let tag = *frame.get(cursor).ok_or(SifError::Truncated {
            offset: cursor,
            needed: 1,
        })?;
        cursor += 1;
        let ty = *frame.get(cursor).ok_or(SifError::Truncated {
            offset: cursor,
            needed: 1,
        })?;
        cursor += 1;
        let name_len = read_u16(frame, cursor)? as usize;
        cursor += 2;
        let value_len = read_u32(frame, cursor)? as usize;
        cursor += 4;
        if !tags.insert(tag) {
            return Err(SifError::DuplicateTag(tag));
        }
        let name_end = cursor
            .checked_add(name_len)
            .ok_or(SifError::LengthMismatch {
                declared: frame.len(),
                actual: cursor,
            })?;
        let value_end = name_end
            .checked_add(value_len)
            .ok_or(SifError::LengthMismatch {
                declared: frame.len(),
                actual: cursor,
            })?;
        if value_end > frame.len() {
            return Err(SifError::Truncated {
                offset: cursor,
                needed: value_end - frame.len(),
            });
        }
        let name =
            std::str::from_utf8(&frame[cursor..name_end]).map_err(|_| SifError::InvalidUtf8)?;
        validate_name(name)?;
        result.push(KvEntry {
            tag,
            ty,
            name,
            value: &frame[name_end..value_end],
        });
        cursor = value_end;
    }
    if cursor != frame.len() {
        return Err(SifError::LengthMismatch {
            declared: frame.len(),
            actual: cursor,
        });
    }
    Ok(result)
}

#[derive(Debug)]
struct Entry {
    tag: u8,
    ty: u8,
    name: String,
    value: Vec<u8>,
}

fn collect_entries(record: &Record) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut append = |slot: &KvSlot| {
        if let Some((tag, ty, value)) = slot.wire_value() {
            entries.push(Entry {
                tag,
                ty,
                name: field_name_for_tag(tag),
                value: value.to_vec(),
            });
        }
    };
    append(&record.kv0);
    append(&record.kv1);
    if let Some(ext) = record.kv_ext() {
        for slot in ext {
            append(slot);
        }
    }
    entries
}

pub(crate) fn field_name_for_tag(tag: u8) -> String {
    const NAMES: &[&str] = &[
        "trace.id",
        "span.id",
        "user.id",
        "session.id",
        "request.id",
        "host.name",
        "app.name",
        "app.version",
        "environment",
        "thread.name",
        "container.id",
        "process.name",
        "source.file",
        "source.function",
        "source.line",
        "source.column",
        "exception.type",
        "exception.message",
        "exception.stacktrace",
        "exception.code",
        "labels",
        "security.audit_tags",
        "coroutine.id",
        "security.gap",
    ];
    for name in NAMES {
        if crate::record::resolve_tag(name, false) == Some(tag) {
            return (*name).to_string();
        }
    }
    vendor_name_for_tag(tag).unwrap_or_else(|| format!("ext.unknown.{tag}"))
}

fn vendor_name_for_tag(tag: u8) -> Option<String> {
    let lock = crate::record::VENDOR_TAGS.get()?.lock().ok()?;
    lock.iter()
        .find_map(|(name, value)| (*value == tag).then(|| name.clone()))
}

pub(crate) fn register_vendor_tag(name: &str, tag: u8) {
    if tag <= KV_TAG_CORE_MAX || !(name.starts_with("ext.") || name.starts_with("verified.")) {
        return;
    }
    if let Some(lock) = crate::record::VENDOR_TAGS.get() {
        if let Ok(mut tags) = lock.lock() {
            tags.entry(name.to_string()).or_insert(tag);
        }
    }
}

fn validate_name(name: &str) -> Result<(), SifError> {
    if name.is_empty() || name.len() > MAX_FIELD_NAME || name.contains('\0') {
        return Err(SifError::InvalidFieldName);
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(SifError::InvalidFieldName);
    }
    if !name.starts_with("ext.") && !name.starts_with("verified.") && !name.contains('.') {
        return Err(SifError::InvalidFieldName);
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, SifError> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or(SifError::Truncated { offset, needed: 2 })?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SifError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or(SifError::Truncated { offset, needed: 4 })?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, SifError> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or(SifError::Truncated { offset, needed: 8 })?;
    Ok(u64::from_le_bytes(
        slice.try_into().expect("checked eight bytes"),
    ))
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], SifError> {
    let slice = bytes
        .get(offset..offset + N)
        .ok_or(SifError::Truncated { offset, needed: N })?;
    Ok(slice.try_into().expect("checked array length"))
}

impl Record {
    pub(crate) fn put_wire_slot(&mut self, tag: u8, ty: u8, value: &[u8]) -> Result<(), SifError> {
        if let Some(slot) = self.kv_find_mut(tag) {
            // SAFETY: kv_find_mut returns a pointer to an initialized slot owned by self.
            unsafe { (&mut *slot).wire_put(tag, ty, value) }.map_err(|_| SifError::FieldApply {
                name: field_name_for_tag(tag),
            })?;
            return Ok(());
        }
        if let Some(slot) = self.kv_find_empty() {
            // SAFETY: kv_find_empty returns a pointer to an initialized slot owned by self.
            unsafe { (&mut *slot).wire_put(tag, ty, value) }.map_err(|_| SifError::FieldApply {
                name: field_name_for_tag(tag),
            })?;
            return Ok(());
        }
        let ext = self.kv_ext_mut();
        let mut slot = KvSlot::empty();
        slot.wire_put(tag, ty, value)
            .map_err(|_| SifError::FieldApply {
                name: field_name_for_tag(tag),
            })?;
        ext.push(slot);
        Ok(())
    }
}
impl From<KvPutError> for SifError {
    fn from(_: KvPutError) -> Self {
        Self::FieldApply {
            name: "<slot>".to_string(),
        }
    }
}

/// Reusable encoder for a single producer thread.
///
/// The returned slice remains valid until the next call. It is intended for
/// SHM and file sinks that already serialize records sequentially and want to
/// avoid retaining one `Vec` per record in their caller.
pub struct ReusableEncoder {
    buffer: Vec<u8>,
    max_frame_size: usize,
}

impl fmt::Debug for ReusableEncoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReusableEncoder")
            .field("capacity", &self.buffer.capacity())
            .field("length", &self.buffer.len())
            .field("max_frame_size", &self.max_frame_size)
            .finish()
    }
}

impl ReusableEncoder {
    /// Create an encoder with a bounded maximum frame size.
    pub fn new(max_frame_size: usize) -> Result<Self, SifError> {
        if !(SIF_HEADER_LEN + SIF_FIXED_LEN..=MAX_FRAME_SIZE).contains(&max_frame_size) {
            return Err(SifError::LengthExceeded {
                field: "encoder budget",
                value: max_frame_size,
                max: MAX_FRAME_SIZE,
            });
        }
        Ok(Self {
            buffer: Vec::with_capacity(SIF_HEADER_LEN + SIF_FIXED_LEN),
            max_frame_size,
        })
    }

    /// Create the default production encoder.
    pub fn production() -> Self {
        Self::new(MAX_FRAME_SIZE).expect("default KV encoder budget is valid")
    }

    /// Encode and borrow the reusable frame buffer.
    pub fn encode(&mut self, record: &Record) -> Result<&[u8], SifError> {
        let encoded = encode_record(record)?;
        if encoded.len() > self.max_frame_size {
            return Err(SifError::LengthExceeded {
                field: "frame",
                value: encoded.len(),
                max: self.max_frame_size,
            });
        }
        self.buffer.clear();
        self.buffer.extend_from_slice(&encoded);
        Ok(&self.buffer)
    }

    /// Borrow the most recently encoded frame.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Return the current buffer capacity.
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Clear retained bytes without reducing capacity.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

/// Incremental length-prefixed KV stream decoder.
///
/// The stream envelope is `[u32 little-endian frame length][SIF frame]`. The
/// envelope is transport-only; the embedded SIF frame still performs all its
/// own validation. `feed` accepts arbitrary chunks and never allocates beyond
/// the configured stream budget.
pub struct FrameScanner {
    buffer: Vec<u8>,
    cursor: usize,
    max_frame_size: usize,
    frames_seen: u64,
    bytes_seen: u64,
}

impl fmt::Debug for FrameScanner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameScanner")
            .field("buffered", &self.buffer.len().saturating_sub(self.cursor))
            .field("max_frame_size", &self.max_frame_size)
            .field("frames_seen", &self.frames_seen)
            .field("bytes_seen", &self.bytes_seen)
            .finish()
    }
}

/// Stream scanner counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScannerStats {
    /// Complete frames yielded.
    pub frames_seen: u64,
    /// Input bytes accepted.
    pub bytes_seen: u64,
    /// Bytes still buffered.
    pub buffered_bytes: usize,
}

impl FrameScanner {
    /// Construct a scanner with an explicit frame limit.
    pub fn new(max_frame_size: usize) -> Result<Self, SifError> {
        if !(SIF_HEADER_LEN + SIF_FIXED_LEN..=MAX_FRAME_SIZE).contains(&max_frame_size) {
            return Err(SifError::LengthExceeded {
                field: "scanner budget",
                value: max_frame_size,
                max: MAX_FRAME_SIZE,
            });
        }
        Ok(Self {
            buffer: Vec::new(),
            cursor: 0,
            max_frame_size,
            frames_seen: 0,
            bytes_seen: 0,
        })
    }

    /// Construct a scanner using the production bound.
    pub fn production() -> Self {
        Self::new(MAX_FRAME_SIZE).expect("default KV scanner budget is valid")
    }

    /// Append an arbitrary transport chunk.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), SifError> {
        if chunk.len() as u64 > u64::MAX.saturating_sub(self.bytes_seen) {
            return Err(SifError::EncodeOverflow("stream bytes"));
        }
        let buffered = self.buffer.len().saturating_sub(self.cursor);
        if buffered.saturating_add(chunk.len()) > self.max_frame_size.saturating_add(4) {
            return Err(SifError::LengthExceeded {
                field: "stream buffer",
                value: buffered + chunk.len(),
                max: self.max_frame_size + 4,
            });
        }
        self.compact();
        self.buffer.extend_from_slice(chunk);
        self.bytes_seen = self.bytes_seen.saturating_add(chunk.len() as u64);
        Ok(())
    }

    /// Yield the next complete SIF frame, if available.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, SifError> {
        if self.buffer.len().saturating_sub(self.cursor) < 4 {
            return Ok(None);
        }
        let length = u32::from_le_bytes(
            self.buffer[self.cursor..self.cursor + 4]
                .try_into()
                .expect("four bytes checked"),
        ) as usize;
        if length < SIF_HEADER_LEN + SIF_FIXED_LEN || length > self.max_frame_size {
            return Err(SifError::LengthExceeded {
                field: "stream frame",
                value: length,
                max: self.max_frame_size,
            });
        }
        let end = self
            .cursor
            .checked_add(4)
            .and_then(|start| start.checked_add(length))
            .ok_or(SifError::LengthMismatch {
                declared: length,
                actual: self.buffer.len(),
            })?;
        if end > self.buffer.len() {
            return Ok(None);
        }
        let frame = self.buffer[self.cursor + 4..end].to_vec();
        validate_frame_with(&frame, DecodeOptions::default())?;
        self.cursor = end;
        self.frames_seen = self.frames_seen.saturating_add(1);
        self.compact();
        Ok(Some(frame))
    }

    /// Decode the next complete frame directly.
    pub fn next_record(&mut self, options: DecodeOptions) -> Result<Option<Record>, SifError> {
        let Some(frame) = self.next_frame()? else {
            return Ok(None);
        };
        decode_record_with(&frame, options).map(Some)
    }

    /// Return scanner counters.
    pub fn stats(&self) -> ScannerStats {
        ScannerStats {
            frames_seen: self.frames_seen,
            bytes_seen: self.bytes_seen,
            buffered_bytes: self.buffer.len().saturating_sub(self.cursor),
        }
    }

    /// Discard an incomplete buffered frame and reset the scanner position.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.frames_seen = 0;
        self.bytes_seen = 0;
    }

    fn compact(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if self.cursor == self.buffer.len() {
            self.buffer.clear();
            self.cursor = 0;
        } else if self.cursor > self.buffer.capacity() / 2 {
            self.buffer.drain(..self.cursor);
            self.cursor = 0;
        }
    }
}

/// Encode a frame with a four-byte stream length prefix.
pub fn encode_length_prefixed(record: &Record) -> Result<Vec<u8>, SifError> {
    let frame = encode_record(record)?;
    let length =
        u32::try_from(frame.len()).map_err(|_| SifError::EncodeOverflow("stream frame"))?;
    let mut output = Vec::with_capacity(4 + frame.len());
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(&frame);
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::FieldRing;

    fn populated() -> Record {
        let mut record = Record::new(7);
        record.set_id(0x1122, 0x3344);
        record.timestamp = 99;
        record.level = LogLevel::Warn;
        record.thread_id = 3;
        record.process_id = 4;
        record.lsn = 5;
        record.flags = 6;
        record.message.set("hello KV 😀");
        record.set_trace_id("trace");
        record.set_span_id("span");
        record
            .field_set("ext.example", "vendor-value", FieldRing::Ring3)
            .unwrap();
        record.compute_content_hash();
        record
    }

    #[test]
    fn round_trip_preserves_fixed_and_dynamic_data() {
        let original = populated();
        let frame = encode_record(&original).unwrap();
        let decoded = decode_record_with(&frame, DecodeOptions::untrusted()).unwrap();
        assert_eq!(decoded.id_hi(), original.id_hi());
        assert_eq!(decoded.id_lo(), original.id_lo());
        assert_eq!(decoded.timestamp, original.timestamp);
        assert_eq!(decoded.message.as_bytes(), original.message.as_bytes());
        assert_eq!(decoded.trace_id(), original.trace_id());
        assert_eq!(
            decoded.field_get("ext.example", FieldRing::Ring3).unwrap(),
            "vendor-value"
        );
        assert_eq!(decoded.content_hash, original.content_hash);
    }

    #[test]
    fn binary_message_round_trip_preserves_bytes_and_kind() {
        let mut original = Record::new(1);
        original.message.set_bytes(&[0, 0xff, 0x80, 0]);
        original.compute_content_hash();
        let frame = encode_record(&original).unwrap();
        let header = validate_frame(&frame).unwrap();
        assert_ne!(header.flags & SIF_FLAG_MESSAGE_BINARY, 0);
        let decoded = decode_record(&frame).unwrap();
        assert_eq!(decoded.message.kind(), MessagePayloadKind::Binary);
        assert_eq!(decoded.message.as_bytes(), &[0, 0xff, 0x80, 0]);
        assert_eq!(decoded.content_hash, original.content_hash);
    }

    #[test]
    fn invalid_message_kind_flags_are_rejected() {
        let mut frame = encode_record(&populated()).unwrap();
        let flags = SIF_FLAG_MESSAGE_BINARY | SIF_FLAG_MESSAGE_EXPLICIT_TEXT;
        frame[6..8].copy_from_slice(&flags.to_le_bytes());
        assert_eq!(validate_frame(&frame), Err(SifError::InvalidMessageKind));
    }

    #[test]
    fn malformed_headers_are_rejected_without_panics() {
        for frame in [vec![], vec![b'K'; 31], b"BAD1".to_vec()] {
            assert!(validate_frame(&frame).is_err());
        }
        let mut frame = encode_record(&populated()).unwrap();
        frame[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            validate_frame(&frame),
            Err(SifError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn entries_are_borrowed_and_validated() {
        let frame = encode_record(&populated()).unwrap();
        let entries = entries(&frame).unwrap();
        assert!(entries.iter().any(|entry| entry.name == "trace.id"));
        assert!(entries.iter().all(|entry| !entry.value.is_empty()));
    }

    #[test]
    fn audit_options_verify_hash() {
        let mut record = populated();
        record.level = LogLevel::Audit;
        record.compute_content_hash();
        let mut frame = encode_record(&record).unwrap();
        frame[SIF_HEADER_LEN + SIF_HASH_OFFSET] ^= 1;
        assert!(matches!(
            decode_record_with(&frame, DecodeOptions::audit()),
            Err(SifError::ContentHashMismatch)
        ));
    }

    #[test]
    fn sif_layout_constants_are_stable() {
        assert_eq!(&SIF_MAGIC, b"SIF\0");
        assert_eq!(SIF_HEADER_LEN, 32);
        assert_eq!(SIF_FIXED_LEN, 90);
        assert_eq!(SIF_MESSAGE_LEN_OFFSET, 86);
    }
    #[test]
    fn reusable_encoder_reuses_capacity() {
        let record = populated();
        let mut encoder = ReusableEncoder::new(MAX_FRAME_SIZE).unwrap();
        let first = encoder.encode(&record).unwrap().to_vec();
        let capacity = encoder.capacity();
        let second = encoder.encode(&record).unwrap().to_vec();
        assert_eq!(first, second);
        assert_eq!(encoder.capacity(), capacity);
        assert_eq!(encoder.as_bytes(), second.as_slice());
    }

    #[test]
    fn reusable_encoder_rejects_unreasonable_budget() {
        assert!(ReusableEncoder::new(SIF_HEADER_LEN + SIF_FIXED_LEN - 1).is_err());
        assert!(ReusableEncoder::new(MAX_FRAME_SIZE + 1).is_err());
    }

    #[test]
    fn length_prefixed_stream_round_trip_handles_small_chunks() {
        let first = encode_length_prefixed(&populated()).unwrap();
        let mut second_record = populated();
        second_record.lsn = 11;
        let second = encode_length_prefixed(&second_record).unwrap();
        let mut stream = FrameScanner::new(MAX_FRAME_SIZE).unwrap();
        let mut combined = first.clone();
        combined.extend_from_slice(&second);
        for chunk in combined.chunks(3) {
            stream.feed(chunk).unwrap();
        }
        let decoded_first = stream
            .next_record(DecodeOptions::default())
            .unwrap()
            .unwrap();
        let decoded_second = stream
            .next_record(DecodeOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(decoded_first.lsn, 5);
        assert_eq!(decoded_second.lsn, 11);
        assert!(stream
            .next_record(DecodeOptions::default())
            .unwrap()
            .is_none());
        assert_eq!(stream.stats().frames_seen, 2);
        assert_eq!(stream.stats().buffered_bytes, 0);
    }

    #[test]
    fn scanner_waits_for_complete_frame() {
        let stream = encode_length_prefixed(&populated()).unwrap();
        let mut scanner = FrameScanner::new(MAX_FRAME_SIZE).unwrap();
        scanner.feed(&stream[..2]).unwrap();
        assert!(scanner.next_frame().unwrap().is_none());
        scanner.feed(&stream[2..stream.len() - 1]).unwrap();
        assert!(scanner.next_frame().unwrap().is_none());
        scanner.feed(&stream[stream.len() - 1..]).unwrap();
        assert!(scanner.next_frame().unwrap().is_some());
    }

    #[test]
    fn scanner_rejects_forged_length_before_allocating() {
        let mut scanner = FrameScanner::new(1024).unwrap();
        scanner.feed(&u32::MAX.to_le_bytes()).unwrap();
        assert!(matches!(
            scanner.next_frame(),
            Err(SifError::LengthExceeded {
                field: "stream frame",
                ..
            })
        ));
    }

    #[test]
    fn scanner_rejects_buffer_bombs() {
        let mut scanner = FrameScanner::new(1024).unwrap();
        let oversized = vec![0u8; 1029];
        assert!(matches!(
            scanner.feed(&oversized),
            Err(SifError::LengthExceeded {
                field: "stream buffer",
                ..
            })
        ));
    }

    #[test]
    fn scanner_reset_discards_partial_input() {
        let stream = encode_length_prefixed(&populated()).unwrap();
        let mut scanner = FrameScanner::new(MAX_FRAME_SIZE).unwrap();
        scanner.feed(&stream[..8]).unwrap();
        scanner.reset();
        assert_eq!(scanner.stats(), ScannerStats::default());
        assert!(scanner.next_frame().unwrap().is_none());
    }
}
