//! Record structure with field permission rings (Ring 0–3) and KV extension.
//!
//! # Ring Model
//!
//! | Ring   | Description                              | Access       |
//! |--------|------------------------------------------|--------------|
//! | Ring 0 | Kernel-core (ID, timestamp)              | Core R/W, Formatter/Sink read-only via dedicated API |
//! | Ring 1 | System trusted (level, message, host)    | Core + HostInfoProvider write, plugins read-only |
//! | Ring 2 | Verified plugin fields                   | Blue/Yellow plugins R/W, audit-tagged |
//! | Ring 3 | Untrusted extension fields               | Any plugin R/W, SHA-256 content-hash covered |
//!
//! # Memory Layout (ADR-002 Appendix A.2)
//!
//! Target: 256 bytes (`repr(C, align(64))`)
//!
//! ```text
//! offset  size  field
//! 0       8     timestamp  (u64 LE, nanos since epoch)
//! 8       1     level      (LogLevel)
//! 12      4     pid        (u32 LE, fixed)
//! 16      4     tid        (u32 LE, fixed)
//! 24      8     lsn        (u64 LE, audit chain)
//! 32      2     flags      (u16 LE, bitfield)
//! 36      4     pool_index (u32, pool slot)
//! 40      96    msg        (RecordString, inline max 94 bytes)
//! 136     32    content_hash (SHA-256, zero when unsigned)
//! 168     32    kv0        (inline KV slot 0)
//! 200     32    kv1        (inline KV slot 1)
//! 232     8     kv_ext     (*mut Vec<KvSlot>, heap overflow; NULL = none)
//! 240     16    _padding   (align(64) → 256 total)
//! ```

use std::borrow::Cow;
use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::Arc;

use crate::error::{
    DO_LOG_ERR_FIELD_NOT_FOUND, DO_LOG_ERR_FIELD_PERMISSION_DENIED, DO_LOG_ERR_FIELD_TYPE_MISMATCH,
};

pub mod kv;
pub mod view;
pub mod wire;

pub use kv::{KvSlot, KvType};
pub use view::{DerivedMessageView, ViewError, ViewTransform};

// ---------------------------------------------------------------------------
// Log levels
// ---------------------------------------------------------------------------

/// Log level enum (matches C ABI `uint8_t` representation).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Trace-level debugging
    Trace = 0,
    /// Debug information
    Debug = 1,
    /// Informational message
    Info = 2,
    /// Warning condition
    Warn = 3,
    /// Error condition
    Error = 4,
    /// Fatal error, system may be unstable
    Fatal = 5,
    /// Non-repudiable audit record (triggers WORM write, Ed25519 signing)
    Audit = 6,
}

impl LogLevel {
    /// Convert to a short display string.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
            Self::Audit => "AUDIT",
        }
    }

    /// Parse from a u8 value.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Trace),
            1 => Some(Self::Debug),
            2 => Some(Self::Info),
            3 => Some(Self::Warn),
            4 => Some(Self::Error),
            5 => Some(Self::Fatal),
            6 => Some(Self::Audit),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// RecordString — 96-byte inline string (ADR-002 Appendix A.5)
// ---------------------------------------------------------------------------

/// Inline capacity for small strings. Messages up to 94 bytes avoid heap.
pub const RECORD_STRING_INLINE_CAPACITY: usize = 96;

/// Maximum inline string length (capacity minus 2 bytes for length sentinel).
pub const RECORD_STRING_INLINE_MAX: usize = RECORD_STRING_INLINE_CAPACITY - 2;

/// The semantic kind of a record message payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePayloadKind {
    /// Bytes validated as canonical UTF-8 at ingestion time.
    Utf8,
    /// Bytes whose text encoding is unknown or intentionally not interpreted.
    Binary,
    /// Text produced by an explicit, lossless decode operation.
    ExplicitDecodedText,
}

const INLINE_UTF8_SENTINEL: u8 = 0x00;
const HEAP_UTF8_SENTINEL: u8 = 0xff;
const INLINE_BINARY_SENTINEL: u8 = 0xfe;
const HEAP_BINARY_SENTINEL: u8 = 0xfd;
const INLINE_DECODED_SENTINEL: u8 = 0xfc;
const HEAP_DECODED_SENTINEL: u8 = 0xfb;

/// A small-string-optimized byte payload with a 96-byte inline buffer.
///
/// Text and binary data share the same storage. Inline binary data uses byte
/// 94 as its length byte and byte 95 as its kind sentinel; heap data stores an
/// `Arc<[u8]>` in the union and uses the sentinel to retain its kind. The
/// payload is mutable only while its owning Record is being assembled; derived
/// formatter/codec views are allocated outside the Record.
///
/// # Memory layout (96 bytes, `repr(C, align(8))`)
///
/// | Offset | Size | Description                           |
/// |--------|------|---------------------------------------|
/// | 0      | 95   | Inline byte buffer (NUL-terminated)   |
/// | 95     | 1    | Length sentinel (0x00 = inline, 0xFF = heap) |
///
/// When stored on the heap (`len == 0xFF`), the first 8 bytes of the inline
/// buffer are reinterpreted as a `*const ()` pointer to an `Arc<str>` (thin
/// pointer on all supported platforms).
#[repr(C, align(8))]
pub union RecordString {
    /// Inline byte buffer (NUL-terminated when in use)
    inline: ManuallyDrop<[u8; RECORD_STRING_INLINE_CAPACITY]>,
    /// Heap fallback: pointer to `Arc<[u8]>` (only for heap sentinels)
    heap: ManuallyDrop<Arc<[u8]>>,
}

// SAFETY: RecordString is only accessed by one thread at a time (the owning
// Record has exclusive access during the pipeline). The pool's free-list
// protocol guarantees no concurrent reads of a freed string.
unsafe impl Send for RecordString {}
// SAFETY: RecordString satisfies Sync because it has exclusive access during
// the pipeline — no concurrent reads of a freed string by the pool protocol.
unsafe impl Sync for RecordString {}

impl RecordString {
    /// Create an empty UTF-8 payload.
    pub const fn empty() -> Self {
        Self {
            inline: ManuallyDrop::new([0u8; RECORD_STRING_INLINE_CAPACITY]),
        }
    }

    /// Returns true if the payload is stored inline (not on the heap).
    #[inline]
    fn is_inline(&self) -> bool {
        // SAFETY: we only access `inline` when `is_inline()` is true,
        // which is determined by the sentinel byte at offset 95.
        unsafe {
            matches!(
                self.inline[RECORD_STRING_INLINE_CAPACITY - 1],
                INLINE_UTF8_SENTINEL | INLINE_BINARY_SENTINEL | INLINE_DECODED_SENTINEL
            )
        }
    }

    /// Return the payload kind.
    pub fn kind(&self) -> MessagePayloadKind {
        // SAFETY: the sentinel byte is initialized by every constructor and
        // mutation path before the payload is exposed to shared readers.
        let sentinel = unsafe { self.inline[RECORD_STRING_INLINE_CAPACITY - 1] };
        match sentinel {
            INLINE_BINARY_SENTINEL | HEAP_BINARY_SENTINEL => MessagePayloadKind::Binary,
            INLINE_DECODED_SENTINEL | HEAP_DECODED_SENTINEL => {
                MessagePayloadKind::ExplicitDecodedText
            }
            _ => MessagePayloadKind::Utf8,
        }
    }

    /// Get the payload length in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        if self.is_inline() {
            // SAFETY: inline path — the length is the position of the first NUL byte.
            unsafe {
                if self.kind() == MessagePayloadKind::Binary {
                    self.inline[RECORD_STRING_INLINE_CAPACITY - 2] as usize
                } else {
                    self.inline[..RECORD_STRING_INLINE_CAPACITY - 2]
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(RECORD_STRING_INLINE_CAPACITY - 2)
                }
            }
        } else {
            // SAFETY: heap path — the Arc<[u8]> knows its length.
            unsafe { self.heap.len() }
        }
    }

    /// Returns true if the string is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return raw payload bytes without decoding or allocation.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        if self.is_inline() {
            let len = self.len();
            // SAFETY: inline bytes are initialized by `set_with_kind`, and
            // `len` is bounded by the inline storage contract.
            unsafe { &self.inline[..len] }
        } else {
            // SAFETY: heap sentinels are written only after storing a valid
            // Arc<[u8]> in the union.
            unsafe { &self.heap }
        }
    }

    /// Borrow the payload as UTF-8 without guessing or replacement.
    #[inline]
    pub fn as_utf8(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.as_bytes())
    }

    /// Render the payload for a human-facing sink without changing its bytes.
    pub fn display_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(self.as_bytes())
    }

    /// Set a validated UTF-8 message.
    pub fn set(&mut self, value: &str) {
        self.set_with_kind(value.as_bytes(), MessagePayloadKind::Utf8);
    }

    /// Set raw bytes without attempting to decode them.
    pub fn set_bytes(&mut self, value: &[u8]) {
        self.set_with_kind(value, MessagePayloadKind::Binary);
    }

    /// Set bytes that were explicitly decoded and validated as UTF-8.
    pub fn set_explicit_decoded_text(&mut self, value: &str) {
        self.set_with_kind(value.as_bytes(), MessagePayloadKind::ExplicitDecodedText);
    }

    /// Set bytes as UTF-8 only after strict validation.
    pub fn set_utf8_bytes(&mut self, value: &[u8]) -> Result<(), std::str::Utf8Error> {
        std::str::from_utf8(value)?;
        self.set_with_kind(value, MessagePayloadKind::Utf8);
        Ok(())
    }

    fn set_with_kind(&mut self, value: &[u8], kind: MessagePayloadKind) {
        self.clear();
        if value.len() <= RECORD_STRING_INLINE_MAX {
            let sentinel = match kind {
                MessagePayloadKind::Utf8 => INLINE_UTF8_SENTINEL,
                MessagePayloadKind::Binary => INLINE_BINARY_SENTINEL,
                MessagePayloadKind::ExplicitDecodedText => INLINE_DECODED_SENTINEL,
            };
            // SAFETY: we are writing into the inline buffer; the sentinel byte
            // at offset 95 identifies the initialized representation.
            unsafe {
                let buf = &mut self.inline;
                buf[..value.len()].copy_from_slice(value);
                if kind == MessagePayloadKind::Binary {
                    buf[RECORD_STRING_INLINE_CAPACITY - 2] = value.len() as u8;
                } else {
                    buf[value.len()] = 0;
                }
                buf[RECORD_STRING_INLINE_CAPACITY - 1] = sentinel;
            }
        } else {
            let sentinel = match kind {
                MessagePayloadKind::Utf8 => HEAP_UTF8_SENTINEL,
                MessagePayloadKind::Binary => HEAP_BINARY_SENTINEL,
                MessagePayloadKind::ExplicitDecodedText => HEAP_DECODED_SENTINEL,
            };
            let arc: Arc<[u8]> = Arc::from(value);
            // SAFETY: we store the Arc pointer in the first 8 bytes of the
            // inline buffer and set the sentinel to a heap kind marker.
            unsafe {
                self.heap = ManuallyDrop::new(arc);
                self.inline[RECORD_STRING_INLINE_CAPACITY - 1] = sentinel;
            }
        }
    }

    /// Clear the string to empty, freeing heap memory if necessary.
    fn clear(&mut self) {
        if !self.is_inline() {
            // SAFETY: heap path — drop the Arc to free the heap string.
            unsafe {
                ManuallyDrop::drop(&mut self.heap);
            }
        }
        // Zero the entire buffer (including sentinel)
        // SAFETY: we have exclusive access to the union.
        unsafe {
            self.inline.as_mut().iter_mut().for_each(|b| *b = 0);
        }
    }
}

impl Drop for RecordString {
    fn drop(&mut self) {
        self.clear();
    }
}

impl Clone for RecordString {
    fn clone(&self) -> Self {
        let mut new = Self::empty();
        new.set_with_kind(self.as_bytes(), self.kind());
        new
    }
}

impl Default for RecordString {
    fn default() -> Self {
        Self::empty()
    }
}

/// Canonical name for the raw message storage type.
pub type MessagePayload = RecordString;

// ---------------------------------------------------------------------------
// KV tag constants (ADR-002 Appendix A.4)
// ---------------------------------------------------------------------------

/// Empty tag marker.
pub const KV_TAG_EMPTY: u8 = 0;

// ── Core tag IDs (1–25, pre-assigned) ──

/// KV tag: distributed trace ID (W3C traceparent).
pub const KV_TAG_TRACE_ID: u8 = 2;
/// KV tag: distributed span ID.
pub const KV_TAG_SPAN_ID: u8 = 3;
/// KV tag: end-user identifier.
pub const KV_TAG_USER_ID: u8 = 4;
/// KV tag: session identifier.
pub const KV_TAG_SESSION_ID: u8 = 5;
/// KV tag: request identifier.
pub const KV_TAG_REQUEST_ID: u8 = 6;
/// KV tag: host name.
pub const KV_TAG_HOST_NAME: u8 = 7;
/// KV tag: application name.
pub const KV_TAG_APP_NAME: u8 = 8;
/// KV tag: application version.
pub const KV_TAG_APP_VERSION: u8 = 9;
/// KV tag: deployment environment (prod, staging, dev, …).
pub const KV_TAG_ENVIRONMENT: u8 = 10;
/// KV tag: thread name.
pub const KV_TAG_THREAD_NAME: u8 = 11;
/// KV tag: process name.
pub const KV_TAG_PROCESS_NAME: u8 = 13;
/// KV tag: container identifier.
pub const KV_TAG_CONTAINER_ID: u8 = 14;
/// KV tag: source file path.
pub const KV_TAG_SOURCE_FILE: u8 = 15;
/// KV tag: source function name.
pub const KV_TAG_SOURCE_FUNCTION: u8 = 16;
/// KV tag: source line number.
pub const KV_TAG_SOURCE_LINE: u8 = 17;
/// KV tag: source column number.
pub const KV_TAG_SOURCE_COLUMN: u8 = 18;
/// KV tag: exception type name.
pub const KV_TAG_EXCEPTION_TYPE: u8 = 19;
/// KV tag: exception message.
pub const KV_TAG_EXCEPTION_MESSAGE: u8 = 20;
/// KV tag: exception stack trace.
pub const KV_TAG_EXCEPTION_STACKTRACE: u8 = 21;
/// KV tag: exception error code.
pub const KV_TAG_EXCEPTION_CODE: u8 = 22;
/// KV tag: structured labels (JSON map).
pub const KV_TAG_LABELS: u8 = 23;
/// KV tag: audit-specific tags (JSON array).
pub const KV_TAG_AUDIT_TAGS: u8 = 24;
/// KV tag: coroutine / async-task identifier.
pub const KV_TAG_COROUTINE_ID: u8 = 25;

/// Highest pre-assigned core tag (tags 1–25 are core; 26–63 reserved core
/// space; 64+ vendor). Distinct from `kv::KV_TAG_CORE_MAX` (63, the core-space
/// upper bound) — this is the *allocated* high-water mark.
pub const KV_TAG_CORE_ALLOCATED_MAX: u8 = 25;

/// Absolute tag ceiling (ruling #14): 63 core-space tags + 129 vendor tags.
pub const KV_TAG_MAX: u8 = 192;

// ── Vendor tag allocator (ruling #14) ──

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Process-wide vendor tag map: name → tag (64+).
static VENDOR_TAGS: OnceLock<Mutex<HashMap<String, u8>>> = OnceLock::new();
/// Next vendor tag to allocate (starts at 64).
static VENDOR_NEXT: OnceLock<Mutex<u8>> = OnceLock::new();

/// Look up or optionally allocate a vendor tag for `name`.
///
/// Returns `None` if the tag space is exhausted (>192 tags total).
fn vendor_tag_for(name: &str, allocate: bool) -> Option<u8> {
    let map = VENDOR_TAGS.get_or_init(|| Mutex::new(HashMap::new()));
    let next = VENDOR_NEXT.get_or_init(|| Mutex::new(64));

    // Keep a stable lock order (map, then next) for all registration paths.
    // Recovering a poisoned lock is safe because the map and counter remain
    // internally consistent after a panic while holding either mutex.
    let mut tags = match map.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(&tag) = tags.get(name) {
        return Some(tag);
    }
    if !allocate {
        return None;
    }

    let mut n = match next.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *n > 192 {
        return None; // tag space exhausted
    }
    let tag = *n;
    *n += 1;
    tags.insert(name.to_string(), tag);
    Some(tag)
}

/// Resolve a field name to a KV tag (core or vendor).
///
/// Returns `None` for fixed fields (timestamp, level, pid, tid, lsn, flags,
/// pool_index, msg) which are not stored in KV slots.
fn resolve_tag(name: &str, allocate_vendor: bool) -> Option<u8> {
    match name {
        // Core tags (A.4 table)
        "id" | "record.id" => Some(1), // tag 1 = id (binary 16B)
        "trace.id" => Some(KV_TAG_TRACE_ID),
        "span.id" => Some(KV_TAG_SPAN_ID),
        "user.id" => Some(KV_TAG_USER_ID),
        "session.id" => Some(KV_TAG_SESSION_ID),
        "request.id" => Some(KV_TAG_REQUEST_ID),
        "host.name" => Some(KV_TAG_HOST_NAME),
        "app.name" => Some(KV_TAG_APP_NAME),
        "app.version" => Some(KV_TAG_APP_VERSION),
        "environment" => Some(KV_TAG_ENVIRONMENT),
        "thread.name" => Some(KV_TAG_THREAD_NAME),
        "process.name" => Some(KV_TAG_PROCESS_NAME),
        "container.id" => Some(KV_TAG_CONTAINER_ID),
        "source.file" => Some(KV_TAG_SOURCE_FILE),
        "source.function" => Some(KV_TAG_SOURCE_FUNCTION),
        "source.line" => Some(KV_TAG_SOURCE_LINE),
        "source.column" => Some(KV_TAG_SOURCE_COLUMN),
        "exception.type" => Some(KV_TAG_EXCEPTION_TYPE),
        "exception.message" => Some(KV_TAG_EXCEPTION_MESSAGE),
        "exception.stacktrace" => Some(KV_TAG_EXCEPTION_STACKTRACE),
        "exception.code" => Some(KV_TAG_EXCEPTION_CODE),
        "labels" => Some(KV_TAG_LABELS),
        "security.audit_tags" | "audit_tags" => Some(KV_TAG_AUDIT_TAGS),
        "coroutine.id" => Some(KV_TAG_COROUTINE_ID),
        // Fixed fields — not in KV
        "timestamp" | "record.timestamp" => None,
        "level" => None,
        "message" => None,
        "process.id" | "record.process_id" => None,
        "thread.id" | "record.thread_id" => None,
        "security.lsn" => None,
        "security.gap" => None,
        // Legacy aliases
        "source_file" => Some(KV_TAG_SOURCE_FILE),
        "source_function" => Some(KV_TAG_SOURCE_FUNCTION),
        "source_line" => Some(KV_TAG_SOURCE_LINE),
        "source_column" => Some(KV_TAG_SOURCE_COLUMN),
        "thread_id" => None,  // maps to fixed tid
        "process_id" => None, // maps to fixed pid
        "thread_name" => Some(KV_TAG_THREAD_NAME),
        "process_name" => Some(KV_TAG_PROCESS_NAME),
        "host_name" => Some(KV_TAG_HOST_NAME),
        "container_id" => Some(KV_TAG_CONTAINER_ID),
        "app_name" => Some(KV_TAG_APP_NAME),
        "app_version" => Some(KV_TAG_APP_VERSION),
        "exception_type" => Some(KV_TAG_EXCEPTION_TYPE),
        "exception_message" => Some(KV_TAG_EXCEPTION_MESSAGE),
        "exception_stacktrace" => Some(KV_TAG_EXCEPTION_STACKTRACE),
        "exception_code" => Some(KV_TAG_EXCEPTION_CODE),
        "coroutine_id" => Some(KV_TAG_COROUTINE_ID),
        // Vendor prefix — lazy allocate
        other if other.starts_with("ext.") || other.starts_with("verified.") => {
            vendor_tag_for(other, allocate_vendor)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Record flags bitfield
// ---------------------------------------------------------------------------

/// Record is in use (allocated from pool).
pub const RECORD_FLAG_IN_USE: u16 = 0x01;
/// Record has been Ed25519-signed.
pub const RECORD_FLAG_SIGNED: u16 = 0x02;
/// Record is an AUDIT-level record.
pub const RECORD_FLAG_AUDIT: u16 = 0x04;
/// Record is a WORM LSN gap marker.
pub const RECORD_FLAG_GAP: u16 = 0x08;

// ---------------------------------------------------------------------------
// Record struct (256B, ADR-002 Appendix A.2)
// ---------------------------------------------------------------------------

/// A log record with field permission rings and KV extension.
///
/// # Memory Layout
///
/// 256 bytes, `repr(C, align(64))`. Fixed hot-path fields occupy the first
/// 136 bytes; KV slots (kv0, kv1, kv_ext) hold dynamic fields.
#[repr(C, align(64))]
pub struct Record {
    // ── Fixed hot-path fields (A.2) ──
    /// Nanosecond-precision UTC timestamp (u64 LE, nanos since Unix epoch).
    pub timestamp: u64,
    /// Current log level.
    pub level: LogLevel,
    /// Process ID (fixed, Ring1, set by HostInfoProvider).
    pub process_id: u32,
    /// Thread ID (fixed, Ring1, set by HostInfoProvider).
    pub thread_id: u32,
    /// Audit chain Log Sequence Number (monotonically increasing).
    pub lsn: u64,
    /// Flags bitfield (RECORD_FLAG_*).
    pub flags: u16,
    /// Pool slot index (set by the object pool on alloc).
    pub(crate) pool_index: u32,
    /// Log message body (inline up to 94 bytes, then heap).
    pub message: MessagePayload,
    /// SHA-256 hash of the canonical serialization (A.3); zero when unsigned.
    pub content_hash: [u8; 32],
    /// Inline KV slot 0.
    pub kv0: KvSlot,
    /// Inline KV slot 1.
    pub kv1: KvSlot,
    /// Heap KV overflow: pointer to `Vec<KvSlot>` (>2 fields); NULL = none.
    pub kv_ext: *mut Vec<KvSlot>,
    /// Padding to reach exactly 256 bytes with `align(64)`.
    _padding: [u8; 16],
}

const _: () = {
    assert!(std::mem::size_of::<Record>() == 256);
    assert!(std::mem::align_of::<Record>() == 64);
    assert!(std::mem::size_of::<RecordString>() == RECORD_STRING_INLINE_CAPACITY);
};

// SAFETY: Record is designed for single-owner access via the pool's
// free-list protocol. The pool grants exclusive mutable access to each
// record during its lifecycle. The `kv_ext` raw pointer is safe to send
// across threads because the pool protocol guarantees no concurrent access.
unsafe impl Send for Record {}
// SAFETY: Record is shared via `&Record` during the Sink stage, where it
// is read-only. The `kv_ext` pointer references heap data that is not
// mutated while the Record is shared. The pool protocol prevents concurrent
// mutation during shared reads.
unsafe impl Sync for Record {}

impl Record {
    /// Create a new zero-initialized record for the given pool slot.
    pub const fn new(pool_index: u32) -> Self {
        Self {
            timestamp: 0,
            level: LogLevel::Info,
            process_id: 0,
            thread_id: 0,
            lsn: 0,
            flags: 0,
            pool_index,
            message: RecordString::empty(),
            content_hash: [0u8; 32],
            kv0: KvSlot::empty(),
            kv1: KvSlot::empty(),
            kv_ext: ptr::null_mut(),
            _padding: [0u8; 16],
        }
    }

    /// Reset the record to empty state (called by pool before reuse).
    pub(crate) fn reset(&mut self) {
        self.flags = 0;
        self.lsn = 0;
        self.content_hash = [0u8; 32];
        self.message.clear();
        self.kv0.clear();
        self.kv1.clear();
        self.kv_clear_ext();
    }

    // ── KV internal operations ──

    /// Access the kv_ext Vec (if allocated).
    fn kv_ext(&self) -> Option<&Vec<KvSlot>> {
        if self.kv_ext.is_null() {
            None
        } else {
            // SAFETY: kv_ext is non-null and was allocated via Box::into_raw.
            // The Record has exclusive access during non-shared use.
            Some(unsafe { &*self.kv_ext })
        }
    }

    /// Mutable access to kv_ext Vec (allocates if needed).
    fn kv_ext_mut(&mut self) -> &mut Vec<KvSlot> {
        if self.kv_ext.is_null() {
            self.kv_ext = Box::into_raw(Box::new(Vec::new()));
        }
        // SAFETY: kv_ext is non-null (just allocated or already existed).
        unsafe { &mut *self.kv_ext }
    }

    /// Free the kv_ext heap allocation and set to null.
    fn kv_clear_ext(&mut self) {
        if !self.kv_ext.is_null() {
            // SAFETY: kv_ext was allocated via Box::into_raw. We retake
            // ownership and drop it. The Vec::drop will drop each KvSlot,
            // freeing any overflow allocations.
            unsafe {
                drop(Box::from_raw(self.kv_ext));
                self.kv_ext = ptr::null_mut();
            }
        }
    }

    /// Find a KV slot by tag. Returns a raw pointer for zero-copy access.
    fn kv_find(&self, tag: u8) -> Option<*const KvSlot> {
        if tag == KV_TAG_EMPTY {
            return None;
        }
        if self.kv0.tag() == tag {
            return Some(&self.kv0 as *const KvSlot);
        }
        if self.kv1.tag() == tag {
            return Some(&self.kv1 as *const KvSlot);
        }
        if let Some(ext) = self.kv_ext() {
            for slot in ext.iter() {
                if slot.tag() == tag {
                    return Some(slot as *const KvSlot);
                }
            }
        }
        None
    }

    /// Find a mutable KV slot by tag.
    fn kv_find_mut(&mut self, tag: u8) -> Option<*mut KvSlot> {
        if tag == KV_TAG_EMPTY {
            return None;
        }
        if self.kv0.tag() == tag {
            return Some(&mut self.kv0 as *mut KvSlot);
        }
        if self.kv1.tag() == tag {
            return Some(&mut self.kv1 as *mut KvSlot);
        }
        if !self.kv_ext.is_null() {
            // SAFETY: `kv_ext` is only created from `Box::into_raw` in this
            // Record and remains exclusively borrowed through `&mut self`.
            let ext = unsafe { &mut *self.kv_ext };
            for slot in ext.iter_mut() {
                if slot.tag() == tag {
                    return Some(slot as *mut KvSlot);
                }
            }
        }
        None
    }

    /// Find the first empty KV slot.
    fn kv_find_empty(&mut self) -> Option<*mut KvSlot> {
        if self.kv0.is_empty() {
            return Some(&mut self.kv0 as *mut KvSlot);
        }
        if self.kv1.is_empty() {
            return Some(&mut self.kv1 as *mut KvSlot);
        }
        // Search ext for an empty slot (one that was cleared)
        let ext = self.kv_ext_mut();
        for slot in ext.iter_mut() {
            if slot.is_empty() {
                return Some(slot as *mut KvSlot);
            }
        }
        None
    }

    /// Put a string value into a KV slot.
    fn kv_put_string(&mut self, tag: u8, value: &str) {
        if tag == KV_TAG_EMPTY || tag > KV_TAG_MAX {
            return; // invalid tag
        }
        // Check if tag already exists (overwrite)
        if let Some(ptr) = self.kv_find_mut(tag) {
            // SAFETY: ptr points to a valid KvSlot owned by this Record.
            unsafe { (*ptr).put_string(tag, value) };
            return;
        }
        // Find an empty slot
        if let Some(ptr) = self.kv_find_empty() {
            // SAFETY: ptr points to a valid empty KvSlot owned by this Record.
            unsafe { (*ptr).put_string(tag, value) };
            return;
        }
        // All slots full — push to ext
        let ext = self.kv_ext_mut();
        let mut slot = KvSlot::empty();
        slot.put_string(tag, value);
        ext.push(slot);
    }

    /// Put a binary value into a KV slot.
    fn kv_put_binary(&mut self, tag: u8, value: &[u8]) {
        if tag == KV_TAG_EMPTY || tag > KV_TAG_MAX {
            return;
        }
        if let Some(ptr) = self.kv_find_mut(tag) {
            // SAFETY: ptr is a valid mutable reference obtained from kv_find_mut
            // which scans the inline kv0/kv1 union slots. The slot is exclusively
            // owned by this Record (no concurrent access possible).
            unsafe { (*ptr).put_binary(tag, value) };
            return;
        }
        if let Some(ptr) = self.kv_find_empty() {
            // SAFETY: ptr points to a valid empty slot obtained from kv_find_empty.
            // The slot is exclusively owned by this Record.
            unsafe { (*ptr).put_binary(tag, value) };
            return;
        }
        let ext = self.kv_ext_mut();
        let mut slot = KvSlot::empty();
        slot.put_binary(tag, value);
        ext.push(slot);
    }

    /// Put a u64 value into a KV slot.
    fn kv_put_u64(&mut self, tag: u8, value: u64) {
        if tag == KV_TAG_EMPTY || tag > KV_TAG_MAX {
            return;
        }
        if let Some(ptr) = self.kv_find_mut(tag) {
            // SAFETY: ptr is a valid mutable reference obtained from kv_find_mut.
            // The slot is exclusively owned by this Record.
            unsafe { (*ptr).put_u64(tag, value) };
            return;
        }
        if let Some(ptr) = self.kv_find_empty() {
            // SAFETY: ptr points to a valid empty slot obtained from kv_find_empty.
            // The slot is exclusively owned by this Record.
            unsafe { (*ptr).put_u64(tag, value) };
            return;
        }
        let ext = self.kv_ext_mut();
        let mut slot = KvSlot::empty();
        slot.put_u64(tag, value);
        ext.push(slot);
    }

    /// Put an i64 value into a KV slot.
    fn kv_put_i64(&mut self, tag: u8, value: i64) {
        if tag == KV_TAG_EMPTY || tag > KV_TAG_MAX {
            return;
        }
        if let Some(ptr) = self.kv_find_mut(tag) {
            // SAFETY: ptr is a valid mutable reference obtained from kv_find_mut.
            // The slot is exclusively owned by this Record.
            unsafe { (*ptr).put_i64(tag, value) };
            return;
        }
        if let Some(ptr) = self.kv_find_empty() {
            // SAFETY: ptr points to a valid empty slot obtained from kv_find_empty.
            // The slot is exclusively owned by this Record.
            unsafe { (*ptr).put_i64(tag, value) };
            return;
        }
        let ext = self.kv_ext_mut();
        let mut slot = KvSlot::empty();
        slot.put_i64(tag, value);
        ext.push(slot);
    }

    /// Get a KV value as a String (for field_get API compatibility).
    fn kv_get_string(&self, tag: u8) -> Option<String> {
        if let Some(ptr) = self.kv_find(tag) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            unsafe { (*ptr).get_string() }
        } else {
            None
        }
    }

    fn kv_get_display_string(&self, tag: u8) -> Option<String> {
        let ptr = self.kv_find(tag)?;
        // SAFETY: ptr is a valid KvSlot owned by this Record.
        let (_, ty, data) = unsafe { (*ptr).get()? };
        match ty {
            ty if ty == KvType::String.as_u8() => std::str::from_utf8(data).ok().map(String::from),
            ty if ty == KvType::UInt64.as_u8() && data.len() >= 8 => {
                Some(u64::from_le_bytes(data[..8].try_into().ok()?).to_string())
            }
            ty if ty == KvType::Int64.as_u8() && data.len() >= 8 => {
                Some(i64::from_le_bytes(data[..8].try_into().ok()?).to_string())
            }
            _ => None,
        }
    }

    // ── Convenience accessors ──

    /// Get the record ID as a hex string (KV tag 1, binary 16B LE).
    pub fn id_hex(&self) -> String {
        if let Some(ptr) = self.kv_find(1) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            if let Some((_, _ty, data)) = unsafe { (*ptr).get() } {
                if data.len() == 16 {
                    let hi = u64::from_le_bytes(data[0..8].try_into().unwrap_or([0; 8]));
                    let lo = u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8]));
                    return format!("{:016x}{:016x}", hi, lo);
                }
            }
        }
        String::new()
    }

    /// Get the record ID high 64 bits (KV tag 1, binary 16B LE).
    pub fn id_hi(&self) -> u64 {
        if let Some(ptr) = self.kv_find(1) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            if let Some((_, _ty, data)) = unsafe { (*ptr).get() } {
                if data.len() >= 8 {
                    return u64::from_le_bytes(data[0..8].try_into().unwrap_or([0; 8]));
                }
            }
        }
        0
    }

    /// Get the record ID low 64 bits (KV tag 1, binary 16B LE).
    pub fn id_lo(&self) -> u64 {
        if let Some(ptr) = self.kv_find(1) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            if let Some((_, _ty, data)) = unsafe { (*ptr).get() } {
                if data.len() >= 16 {
                    return u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8]));
                }
            }
        }
        0
    }

    /// Get the timestamp split into (seconds, nanos) for SIF compatibility.
    pub fn timestamp_secs(&self) -> u32 {
        (self.timestamp / 1_000_000_000) as u32
    }

    /// Get the timestamp nanoseconds remainder for SIF compatibility.
    pub fn timestamp_subsec_nanos(&self) -> u32 {
        (self.timestamp % 1_000_000_000) as u32
    }

    /// Set the record ID from hi/lo (KV tag 1, binary 16B LE).
    pub fn set_id(&mut self, hi: u64, lo: u64) {
        let mut buf = [0u8; 16];
        buf[0..8].copy_from_slice(&hi.to_le_bytes());
        buf[8..16].copy_from_slice(&lo.to_le_bytes());
        self.kv_put_binary(1, &buf);
    }

    /// Get the source file path (KV tag 15).
    pub fn source_file(&self) -> String {
        self.kv_get_string(KV_TAG_SOURCE_FILE).unwrap_or_default()
    }

    /// Set the source file path.
    pub fn set_source_file(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_SOURCE_FILE, v);
    }

    /// Get the source function name (KV tag 16).
    pub fn source_function(&self) -> String {
        self.kv_get_string(KV_TAG_SOURCE_FUNCTION)
            .unwrap_or_default()
    }

    /// Set the source function name.
    pub fn set_source_function(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_SOURCE_FUNCTION, v);
    }

    /// Get the source line number (KV tag 17, stored as u64).
    pub fn source_line(&self) -> u32 {
        if let Some(ptr) = self.kv_find(KV_TAG_SOURCE_LINE) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            if let Some((_, ty, data)) = unsafe { (*ptr).get() } {
                if ty == KvType::UInt64.as_u8() && data.len() >= 8 {
                    return u64::from_le_bytes(data[..8].try_into().unwrap_or([0; 8])) as u32;
                }
                // Fallback: try parsing as string (for FFI/legacy callers)
                // SAFETY: ptr is a valid KvSlot owned by this Record.
                if let Some(s) = unsafe { (*ptr).get_string() } {
                    return s.parse().unwrap_or(0);
                }
            }
        }
        0
    }

    /// Set the source line number.
    pub fn set_source_line(&mut self, v: u32) {
        self.kv_put_u64(KV_TAG_SOURCE_LINE, v as u64);
    }

    /// Get the source column number (KV tag 18, stored as u64).
    pub fn source_column(&self) -> u32 {
        if let Some(ptr) = self.kv_find(KV_TAG_SOURCE_COLUMN) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            if let Some((_, ty, data)) = unsafe { (*ptr).get() } {
                if ty == KvType::UInt64.as_u8() && data.len() >= 8 {
                    return u64::from_le_bytes(data[..8].try_into().unwrap_or([0; 8])) as u32;
                }
                // SAFETY: ptr is a valid KvSlot owned by this Record.
                if let Some(s) = unsafe { (*ptr).get_string() } {
                    return s.parse().unwrap_or(0);
                }
            }
        }
        0
    }

    /// Set the source column number.
    pub fn set_source_column(&mut self, v: u32) {
        self.kv_put_u64(KV_TAG_SOURCE_COLUMN, v as u64);
    }

    /// Get the thread name (KV tag 11).
    pub fn thread_name(&self) -> String {
        self.kv_get_string(KV_TAG_THREAD_NAME).unwrap_or_default()
    }

    /// Set the thread name.
    pub fn set_thread_name(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_THREAD_NAME, v);
    }

    /// Get the thread ID (fixed field).
    #[inline]
    pub fn thread_id(&self) -> u32 {
        self.thread_id
    }

    /// Get the process name (KV tag 13).
    pub fn process_name(&self) -> String {
        self.kv_get_string(KV_TAG_PROCESS_NAME).unwrap_or_default()
    }

    /// Set the process name.
    pub fn set_process_name(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_PROCESS_NAME, v);
    }

    /// Get the process ID (fixed field).
    #[inline]
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    /// Get the host name (KV tag 7).
    pub fn host_name(&self) -> String {
        self.kv_get_string(KV_TAG_HOST_NAME).unwrap_or_default()
    }

    /// Set the host name.
    pub fn set_host_name(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_HOST_NAME, v);
    }

    /// Get the container ID (KV tag 14).
    pub fn container_id(&self) -> String {
        self.kv_get_string(KV_TAG_CONTAINER_ID).unwrap_or_default()
    }

    /// Set the container ID.
    pub fn set_container_id(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_CONTAINER_ID, v);
    }

    /// Get the application name (KV tag 8).
    pub fn app_name(&self) -> String {
        self.kv_get_string(KV_TAG_APP_NAME).unwrap_or_default()
    }

    /// Set the application name.
    pub fn set_app_name(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_APP_NAME, v);
    }

    /// Get the application version (KV tag 9).
    pub fn app_version(&self) -> String {
        self.kv_get_string(KV_TAG_APP_VERSION).unwrap_or_default()
    }

    /// Set the application version.
    pub fn set_app_version(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_APP_VERSION, v);
    }

    /// Get the environment name (KV tag 10).
    pub fn environment(&self) -> String {
        self.kv_get_string(KV_TAG_ENVIRONMENT).unwrap_or_default()
    }

    /// Set the environment name.
    pub fn set_environment(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_ENVIRONMENT, v);
    }

    /// Get the user ID (KV tag 4).
    pub fn user_id(&self) -> String {
        self.kv_get_string(KV_TAG_USER_ID).unwrap_or_default()
    }

    /// Set the user ID.
    pub fn set_user_id(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_USER_ID, v);
    }

    /// Get the session ID (KV tag 5).
    pub fn session_id(&self) -> String {
        self.kv_get_string(KV_TAG_SESSION_ID).unwrap_or_default()
    }

    /// Set the session ID.
    pub fn set_session_id(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_SESSION_ID, v);
    }

    /// Get the request ID (KV tag 6).
    pub fn request_id(&self) -> String {
        self.kv_get_string(KV_TAG_REQUEST_ID).unwrap_or_default()
    }

    /// Set the request ID.
    pub fn set_request_id(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_REQUEST_ID, v);
    }

    /// Get the trace ID (KV tag 2).
    pub fn trace_id(&self) -> String {
        self.kv_get_string(KV_TAG_TRACE_ID).unwrap_or_default()
    }

    /// Set the trace ID.
    pub fn set_trace_id(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_TRACE_ID, v);
    }

    /// Get the span ID (KV tag 3).
    pub fn span_id(&self) -> String {
        self.kv_get_string(KV_TAG_SPAN_ID).unwrap_or_default()
    }

    /// Set the span ID.
    pub fn set_span_id(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_SPAN_ID, v);
    }

    /// Get the coroutine ID (KV tag 25, stored as u64).
    pub fn coroutine_id(&self) -> u64 {
        if let Some(ptr) = self.kv_find(KV_TAG_COROUTINE_ID) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            if let Some((_, ty, data)) = unsafe { (*ptr).get() } {
                if ty == KvType::UInt64.as_u8() && data.len() >= 8 {
                    return u64::from_le_bytes(data[..8].try_into().unwrap_or([0; 8]));
                }
                // SAFETY: ptr is a valid KvSlot owned by this Record.
                if let Some(s) = unsafe { (*ptr).get_string() } {
                    return s.parse().unwrap_or(0);
                }
            }
        }
        0
    }

    /// Set the coroutine ID.
    pub fn set_coroutine_id(&mut self, v: u64) {
        self.kv_put_u64(KV_TAG_COROUTINE_ID, v);
    }

    /// Get the exception type (KV tag 19).
    pub fn exception_type(&self) -> String {
        self.kv_get_string(KV_TAG_EXCEPTION_TYPE)
            .unwrap_or_default()
    }

    /// Set the exception type.
    pub fn set_exception_type(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_EXCEPTION_TYPE, v);
    }

    /// Get the exception message (KV tag 20).
    pub fn exception_message(&self) -> String {
        self.kv_get_string(KV_TAG_EXCEPTION_MESSAGE)
            .unwrap_or_default()
    }

    /// Set the exception message.
    pub fn set_exception_message(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_EXCEPTION_MESSAGE, v);
    }

    /// Get the exception stacktrace (KV tag 21).
    pub fn exception_stacktrace(&self) -> String {
        self.kv_get_string(KV_TAG_EXCEPTION_STACKTRACE)
            .unwrap_or_default()
    }

    /// Set the exception stacktrace.
    pub fn set_exception_stacktrace(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_EXCEPTION_STACKTRACE, v);
    }

    /// Get the exception code (KV tag 22, stored as i64).
    pub fn exception_code(&self) -> i64 {
        if let Some(ptr) = self.kv_find(KV_TAG_EXCEPTION_CODE) {
            // SAFETY: ptr is a valid KvSlot owned by this Record.
            if let Some((_, ty, data)) = unsafe { (*ptr).get() } {
                if ty == KvType::Int64.as_u8() && data.len() >= 8 {
                    return i64::from_le_bytes(data[..8].try_into().unwrap_or([0; 8]));
                }
                // SAFETY: ptr is a valid KvSlot owned by this Record.
                if let Some(s) = unsafe { (*ptr).get_string() } {
                    return s.parse().unwrap_or(0);
                }
            }
        }
        0
    }

    /// Set the exception code.
    pub fn set_exception_code(&mut self, v: i64) {
        self.kv_put_i64(KV_TAG_EXCEPTION_CODE, v);
    }

    /// Get the labels JSON (KV tag 23).
    pub fn labels(&self) -> String {
        self.kv_get_string(KV_TAG_LABELS).unwrap_or_default()
    }

    /// Set the labels JSON.
    pub fn set_labels(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_LABELS, v);
    }

    /// Get the audit tags (KV tag 24).
    pub fn audit_tags(&self) -> String {
        self.kv_get_string(KV_TAG_AUDIT_TAGS).unwrap_or_default()
    }

    /// Set the audit tags.
    pub fn set_audit_tags(&mut self, v: &str) {
        self.kv_put_string(KV_TAG_AUDIT_TAGS, v);
    }

    /// Get the security gap flag (flags bit 3).
    #[inline]
    pub fn security_gap(&self) -> bool {
        self.flags & RECORD_FLAG_GAP != 0
    }

    /// Set the security gap flag.
    #[inline]
    pub fn set_security_gap(&mut self, v: bool) {
        if v {
            self.flags |= RECORD_FLAG_GAP;
        } else {
            self.flags &= !RECORD_FLAG_GAP;
        }
    }

    /// Get the timestamp as nanos since epoch.
    #[inline]
    pub fn timestamp_nanos(&self) -> u64 {
        self.timestamp
    }

    // ── Canonical serialization (A.3) ──

    /// Compute SHA-256 of the canonical serialization for content_hash.
    ///
    /// The canonical order is: fixed fields in struct order → KV slots in
    /// insertion order (kv0, kv1, then kv_ext). The `content_hash` field
    /// itself is excluded from the hash input.
    pub fn compute_content_hash(&mut self) {
        self.content_hash = Self::compute_content_hash_from(self);
    }

    /// Non-mutating canonical-serialization hash (A.3).
    ///
    /// Used by the signature verifier to detect content tampering without
    /// requiring `&mut Record`: a freshly computed hash that differs from the
    /// stored `content_hash` means some hashed field was altered after signing.
    pub fn compute_content_hash_from(record: &Record) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();

        // Fixed fields in struct order
        hasher.update(record.timestamp.to_le_bytes());
        hasher.update([record.level as u8]);
        hasher.update(record.process_id.to_le_bytes());
        hasher.update(record.thread_id.to_le_bytes());
        hasher.update(record.lsn.to_le_bytes());
        hasher.update(record.flags.to_le_bytes());
        hasher.update(record.pool_index.to_le_bytes());

        // message: kind- and length-prefixed raw bytes. The kind is part of
        // the signed canonical input so text/binary reinterpretation cannot
        // produce the same audit hash.
        let msg_bytes = record.message.as_bytes();
        hasher.update([match record.message.kind() {
            MessagePayloadKind::Utf8 => 0,
            MessagePayloadKind::Binary => 1,
            MessagePayloadKind::ExplicitDecodedText => 2,
        }]);
        hasher.update((msg_bytes.len() as u64).to_le_bytes());
        hasher.update(msg_bytes);

        // content_hash field is EXCLUDED (we're computing it)

        // KV slots in order — stream canonical bytes directly into SHA-256 so
        // hashing does not allocate a temporary buffer per record.
        record.kv0.update_hash(&mut hasher);
        record.kv1.update_hash(&mut hasher);
        if let Some(ext) = record.kv_ext() {
            for slot in ext.iter() {
                slot.update_hash(&mut hasher);
            }
        }

        hasher.finalize().into()
    }

    // ── Field API (C ABI compatible) ──

    /// Set a field by name with ring permission check.
    ///
    /// Returns `Ok(())` on success, or a typed [`FieldError`] that maps to a
    /// `DO_LOG_ERR_FIELD_*` code at the ABI boundary.
    pub fn field_set(
        &mut self,
        name: &str,
        value: &str,
        ring: FieldRing,
    ) -> Result<(), FieldError> {
        // Authorize by field ring first (ADR A.4 + ruling #16): a caller may
        // write a field only when its ring is at least as privileged as the
        // field's ring (smaller number = more privileged). Ring 0 fields are
        // engine-managed and never writable through the string field API.
        // Unknown fields are denied by default.
        let target_ring = Self::field_ring(name).ok_or(FieldError::NotFound)?;
        if target_ring == FieldRing::Ring0 {
            return Err(FieldError::ReadOnly);
        }
        if target_ring == FieldRing::Ring1 && ring > FieldRing::Ring1 {
            crate::sys::diagnostics::error(
                "security",
                &format!(
                    "SECURITY_VIOLATION: Unauthorized Ring 1 write attempt to '{name}' by caller {ring:?}"
                ),
            );
            return Err(FieldError::SecurityViolation);
        }
        // Ring 2/3 fields accept any caller (audit-tagged / content-hash-covered).

        match name {
            // ── Fixed fields ──
            "timestamp" | "record.timestamp" => {
                // Ring 0 — engine-managed, never settable via string API.
                Err(FieldError::ReadOnly)
            }
            "level" => {
                // Parse level string
                match value.to_uppercase().as_str() {
                    "TRACE" => {
                        self.level = LogLevel::Trace;
                        Ok(())
                    }
                    "DEBUG" => {
                        self.level = LogLevel::Debug;
                        Ok(())
                    }
                    "INFO" => {
                        self.level = LogLevel::Info;
                        Ok(())
                    }
                    "WARN" => {
                        self.level = LogLevel::Warn;
                        Ok(())
                    }
                    "ERROR" => {
                        self.level = LogLevel::Error;
                        Ok(())
                    }
                    "FATAL" => {
                        self.level = LogLevel::Fatal;
                        Ok(())
                    }
                    "AUDIT" => {
                        self.level = LogLevel::Audit;
                        Ok(())
                    }
                    _ => {
                        if let Ok(v) = value.parse::<u8>() {
                            if let Some(l) = LogLevel::from_u8(v) {
                                self.level = l;
                                return Ok(());
                            }
                        }
                        Err(FieldError::TypeMismatch)
                    }
                }
            }
            "id" | "record.id" => {
                // Ring0 — always read-only for field API callers.
                Err(FieldError::ReadOnly)
            }
            "message" => {
                self.message.set(value);
                Ok(())
            }
            "process.id" | "record.process_id" => {
                if let Ok(v) = value.parse::<u32>() {
                    self.process_id = v;
                    Ok(())
                } else {
                    Err(FieldError::TypeMismatch)
                }
            }
            "thread.id" | "record.thread_id" => {
                if let Ok(v) = value.parse::<u32>() {
                    self.thread_id = v;
                    Ok(())
                } else {
                    Err(FieldError::TypeMismatch)
                }
            }
            "security.lsn" => {
                // Ring 0 — engine-managed; unreachable via string API.
                Err(FieldError::ReadOnly)
            }
            "security.gap" => {
                self.set_security_gap(value == "1" || value.eq_ignore_ascii_case("true"));
                Ok(())
            }
            // ── KV fields (ring-gated by the guard above) ──
            other => {
                let tag = resolve_tag(other, true).ok_or(FieldError::NotFound)?;
                self.kv_put_string(tag, value);
                Ok(())
            }
        }
    }

    /// Get a field value by name. Reads are permitted for every ring
    /// (ADR A.4 — ring checks apply to writes, not reads).
    ///
    /// Returns `Ok(value_string)` on success, or a typed [`FieldError`].
    pub fn field_get(&self, name: &str, _ring: FieldRing) -> Result<String, FieldError> {
        match name {
            "id" | "record.id" => {
                let value = self.id_hex();
                if value.is_empty() {
                    Err(FieldError::NotFound)
                } else {
                    Ok(value)
                }
            }
            "timestamp" | "record.timestamp" => Ok(self.timestamp.to_string()),
            "level" => Ok(self.level.to_str().to_string()),
            "message" => Ok(self.message.display_lossy().into_owned()),
            "process.id" | "record.process_id" | "process_id" => Ok(self.process_id.to_string()),
            "thread.id" | "record.thread_id" | "thread_id" => Ok(self.thread_id.to_string()),
            "security.lsn" => Ok(self.lsn.to_string()),
            "security.gap" => Ok(if self.security_gap() { "1" } else { "0" }.to_string()),
            // ── KV fields ──
            other => {
                let tag = resolve_tag(other, false).ok_or(FieldError::NotFound)?;
                self.kv_get_display_string(tag).ok_or(FieldError::NotFound)
            }
        }
    }

    /// Returns which ring a field belongs to (for permission checks).
    ///
    /// Mapping follows ADR A.4: `id` is Ring 0 (engine-managed), core tags
    /// 2-25 (`trace.id` … `coroutine.id`) are Ring 1, `verified.*` is Ring 2,
    /// `ext.*` is Ring 3, and unknown fields are denied by default.
    pub fn field_ring(name: &str) -> Option<FieldRing> {
        match name {
            // Ring 0: kernel-core (read-only for external callers)
            "id" | "record.id" | "timestamp" | "record.timestamp" | "record.signature"
            | "record.origin_lsn" | "security.lsn" => Some(FieldRing::Ring0),
            // Ring 1: system trusted (core + HostInfoProvider) — ADR A.4 tags 2-25
            "level"
            | "message"
            | "trace.id"
            | "span.id"
            | "user.id"
            | "session.id"
            | "request.id"
            | "host.name"
            | "app.name"
            | "app.version"
            | "environment"
            | "thread.name"
            | "thread.id"
            | "process.name"
            | "process.id"
            | "container.id"
            | "source.file"
            | "source.function"
            | "source.line"
            | "source.column"
            | "exception.type"
            | "exception.message"
            | "exception.stacktrace"
            | "exception.code"
            | "labels"
            | "security.audit_tags"
            | "coroutine.id"
            | "security.gap" => Some(FieldRing::Ring1),
            // Ring 2: verified plugin (any caller may write, audit-tagged)
            other if other.starts_with("verified.") => Some(FieldRing::Ring2),
            // Ring 3: untrusted extension (any caller may write, content-hash-covered)
            other if other.starts_with("ext.") => Some(FieldRing::Ring3),
            // Legacy aliases
            "source_file"
            | "source_function"
            | "source_line"
            | "source_column"
            | "thread_name"
            | "process_name"
            | "host_name"
            | "container_id"
            | "app_name"
            | "app_version"
            | "exception_type"
            | "exception_message"
            | "exception_stacktrace"
            | "exception_code"
            | "coroutine_id" => Some(FieldRing::Ring1),
            "audit_tags" => Some(FieldRing::Ring1),
            _ => None,
        }
    }
}

impl Drop for Record {
    fn drop(&mut self) {
        // Free kv_ext heap if allocated
        if !self.kv_ext.is_null() {
            // SAFETY: kv_ext was allocated via Box::new(Vec::new()) and is only
            // freed here during Record drop. The pointer is non-null (checked
            // above) and was originally valid. No other reference exists.
            unsafe {
                drop(Box::from_raw(self.kv_ext));
                self.kv_ext = ptr::null_mut();
            }
        }
        // kv0/kv1 Drop handles their own overflow slots
        // msg Drop handles its own heap
    }
}

// ---------------------------------------------------------------------------
// Field permission rings
// ---------------------------------------------------------------------------

/// Permission ring for field access control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FieldRing {
    /// Ring 0 — kernel-core fields (read-only for plugins/formatters)
    Ring0 = 0,
    /// Ring 1 — system trusted fields (core + HostInfoProvider)
    Ring1 = 1,
    /// Ring 2 — verified plugin fields (Blue/Yellow)
    Ring2 = 2,
    /// Ring 3 — untrusted extension fields (any plugin)
    Ring3 = 3,
}

/// Errors returned by the record field API (`field_set` / `field_get`).
///
/// Each variant maps to a `DO_LOG_ERR_FIELD_*` code at the ABI boundary (see
/// `core/src/error.rs`), so FFI callers can distinguish failure causes without
/// parsing message strings. Never returns a bare `&'static str` — see the
/// errors standard §6 (typed internal errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldError {
    /// The field name is unknown (no fixed field, no registered KV tag).
    NotFound,
    /// The caller's ring is not privileged enough for the field's ring.
    PermissionDenied,
    /// The field is engine-managed (Ring 0) and never writable via the API.
    ReadOnly,
    /// The supplied value cannot be parsed into the field's target type.
    TypeMismatch,
    /// A security boundary was crossed (e.g. Ring 1 field by a Ring 2+ caller).
    SecurityViolation,
}

impl FieldError {
    /// Map to the C ABI error code for the FFI layer.
    pub fn abi_code(self) -> i32 {
        match self {
            Self::NotFound => DO_LOG_ERR_FIELD_NOT_FOUND,
            Self::PermissionDenied | Self::ReadOnly | Self::SecurityViolation => {
                DO_LOG_ERR_FIELD_PERMISSION_DENIED
            }
            Self::TypeMismatch => DO_LOG_ERR_FIELD_TYPE_MISMATCH,
        }
    }

    /// Human-readable message for the FFI `set_last_error` slot and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "field not found",
            Self::PermissionDenied => "Permission denied: caller ring is not privileged enough",
            Self::ReadOnly => "field is read-only (engine-managed)",
            Self::TypeMismatch => "invalid field value (type mismatch)",
            Self::SecurityViolation => {
                "SECURITY_VIOLATION: Ring 1 fields require HostInfoProvider or core"
            }
        }
    }
}

impl std::fmt::Display for FieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Extract a u64 from the current thread's ThreadId via its Debug representation.
///
/// Used by both the FFI layer and HostInfoProvider to populate `record.thread_id`.
pub fn thread_id_u64() -> u64 {
    let s = format!("{:?}", std::thread::current().id());
    s.trim_start_matches("ThreadId(")
        .trim_end_matches(')')
        .parse()
        .unwrap_or(0)
}

/// Get the current process ID as a u64.
pub fn process_id_u64() -> u64 {
    std::process::id() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── RecordString tests ──

    #[test]
    fn test_record_string_sizes() {
        assert_eq!(
            std::mem::size_of::<RecordString>(),
            RECORD_STRING_INLINE_CAPACITY
        );
    }

    #[test]
    fn test_record_string_inline() {
        let mut s = RecordString::empty();
        assert!(s.is_empty());
        assert!(s.is_inline());

        s.set("hello world");
        assert_eq!(s.len(), 11);
        assert_eq!(s.as_utf8().unwrap(), "hello world");
        assert!(s.is_inline());

        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn test_record_string_heap_fallback() {
        let mut s = RecordString::empty();
        let long = "x".repeat(RECORD_STRING_INLINE_MAX + 1);
        s.set(&long);
        assert!(!s.is_inline());
        assert_eq!(s.len(), RECORD_STRING_INLINE_MAX + 1);
        assert_eq!(s.as_utf8().unwrap(), long.as_str());

        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn test_record_string_zero_copy() {
        let mut s = RecordString::empty();
        s.set("test");
        let ptr1 = s.as_utf8().unwrap().as_ptr();
        // Reading again should return the same pointer (no re-allocation)
        let ptr2 = s.as_utf8().unwrap().as_ptr();
        assert_eq!(ptr1, ptr2);
    }

    #[test]
    fn test_record_string_binary_payload_preserves_nuls() {
        let mut payload = RecordString::empty();
        payload.set_bytes(&[0, 1, 0xff, 0]);
        assert_eq!(payload.kind(), MessagePayloadKind::Binary);
        assert_eq!(payload.as_bytes(), &[0, 1, 0xff, 0]);
        assert!(payload.as_utf8().is_err());
    }

    // ── Record size tests ──

    #[test]
    fn test_record_size() {
        assert_eq!(std::mem::size_of::<Record>(), 256);
    }

    #[test]
    fn test_record_align() {
        assert_eq!(std::mem::align_of::<Record>(), 64);
    }

    // ── Record basic tests ──

    #[test]
    fn test_record_new() {
        let r = Record::new(42);
        assert_eq!(r.pool_index, 42);
        assert_eq!(r.timestamp, 0);
        assert_eq!(r.level, LogLevel::Info);
        assert_eq!(r.process_id, 0);
        assert_eq!(r.thread_id, 0);
        assert_eq!(r.lsn, 0);
        assert_eq!(r.flags, 0);
        assert!(r.message.is_empty());
        assert_eq!(r.content_hash, [0u8; 32]);
        assert!(r.kv0.is_empty());
        assert!(r.kv1.is_empty());
        assert!(r.kv_ext.is_null());
    }

    #[test]
    fn test_record_defaults() {
        let r = Record::new(0);
        assert!(!r.security_gap());
        assert_eq!(r.source_file(), "");
        assert_eq!(r.source_function(), "");
        assert_eq!(r.source_line(), 0);
        assert_eq!(r.source_column(), 0);
        assert_eq!(r.thread_name(), "");
        assert_eq!(r.thread_id(), 0);
        assert_eq!(r.process_name(), "");
        assert_eq!(r.process_id(), 0);
        assert_eq!(r.host_name(), "");
        assert_eq!(r.container_id(), "");
        assert_eq!(r.app_name(), "");
        assert_eq!(r.app_version(), "");
        assert_eq!(r.environment(), "");
        assert_eq!(r.user_id(), "");
        assert_eq!(r.session_id(), "");
        assert_eq!(r.request_id(), "");
        assert_eq!(r.trace_id(), "");
        assert_eq!(r.span_id(), "");
        assert_eq!(r.coroutine_id(), 0);
        assert_eq!(r.exception_type(), "");
        assert_eq!(r.exception_message(), "");
        assert_eq!(r.exception_stacktrace(), "");
        assert_eq!(r.exception_code(), 0);
        assert_eq!(r.labels(), "");
        assert_eq!(r.audit_tags(), "");
    }

    // ── KV field roundtrip tests ──

    #[test]
    fn test_kv_string_roundtrip() {
        let mut r = Record::new(0);
        r.set_source_file("src/main.rs");
        assert_eq!(r.source_file(), "src/main.rs");
        assert!(r.kv_ext.is_null(), "inline KV writes must not allocate ext");

        r.set_host_name("prod-server-1");
        assert_eq!(r.host_name(), "prod-server-1");

        r.set_trace_id("abc123");
        assert_eq!(r.trace_id(), "abc123");
    }

    #[test]
    fn vendor_read_does_not_reserve_tag() {
        let mut record = Record::new(0);
        let field = "ext.dacs.lookup_only";
        assert_eq!(
            record.field_get(field, FieldRing::Ring3),
            Err(FieldError::NotFound)
        );
        record
            .field_set(field, "value", FieldRing::Ring3)
            .expect("first write allocates the vendor tag");
        assert_eq!(record.field_get(field, FieldRing::Ring3).unwrap(), "value");
    }

    #[test]
    fn test_kv_u64_roundtrip() {
        let mut r = Record::new(0);
        r.set_source_line(42);
        assert_eq!(r.source_line(), 42);
        assert_eq!(r.field_get("source.line", FieldRing::Ring1).unwrap(), "42");

        r.set_coroutine_id(0xDEAD_BEEF);
        assert_eq!(r.coroutine_id(), 0xDEAD_BEEF);
        assert_eq!(
            r.field_get("coroutine.id", FieldRing::Ring1).unwrap(),
            "3735928559"
        );

        r.set_exception_code(-42);
        assert_eq!(
            r.field_get("exception.code", FieldRing::Ring1).unwrap(),
            "-42"
        );
    }

    #[test]
    fn test_kv_binary_id_roundtrip() {
        let mut r = Record::new(0);
        r.set_id(0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF01);
        let hex = r.id_hex();
        assert!(hex.contains("1122334455667788"));
        assert!(hex.contains("99aabbccddeeff01"));
        assert_eq!(r.field_get("id", FieldRing::Ring0).unwrap(), hex);
    }

    #[test]
    fn test_kv_overwrite() {
        let mut r = Record::new(0);
        r.set_source_file("first.rs");
        assert_eq!(r.source_file(), "first.rs");
        r.set_source_file("second.rs");
        assert_eq!(r.source_file(), "second.rs");
    }

    #[test]
    fn test_kv_two_slots() {
        let mut r = Record::new(0);
        r.set_trace_id("t1");
        r.set_span_id("s1");
        assert_eq!(r.trace_id(), "t1");
        assert_eq!(r.span_id(), "s1");
    }

    #[test]
    fn test_kv_overflow_to_ext() {
        let mut r = Record::new(0);
        // Fill both inline slots
        r.set_trace_id("t1");
        r.set_span_id("s1");
        // Third field should go to kv_ext
        r.set_user_id("u1");
        assert_eq!(r.trace_id(), "t1");
        assert_eq!(r.span_id(), "s1");
        assert_eq!(r.user_id(), "u1");
        assert!(!r.kv_ext.is_null());
    }

    // ── Flags tests ──

    #[test]
    fn test_security_gap_flag() {
        let mut r = Record::new(0);
        assert!(!r.security_gap());
        r.set_security_gap(true);
        assert!(r.security_gap());
        assert!(r.flags & RECORD_FLAG_GAP != 0);
        r.set_security_gap(false);
        assert!(!r.security_gap());
    }

    // ── Canonical serialization tests ──

    #[test]
    fn test_canonical_deterministic() {
        let mut r1 = Record::new(0);
        r1.timestamp = 12345;
        r1.level = LogLevel::Info;
        r1.message.set("test message");
        r1.set_source_file("main.rs");
        r1.compute_content_hash();

        let mut r2 = Record::new(0);
        r2.timestamp = 12345;
        r2.level = LogLevel::Info;
        r2.message.set("test message");
        r2.set_source_file("main.rs");
        r2.compute_content_hash();

        assert_eq!(r1.content_hash, r2.content_hash);
    }

    #[test]
    fn test_canonical_different_records_differ() {
        let mut r1 = Record::new(0);
        r1.timestamp = 100;
        r1.compute_content_hash();

        let mut r2 = Record::new(0);
        r2.timestamp = 200;
        r2.compute_content_hash();

        assert_ne!(r1.content_hash, r2.content_hash);
    }

    // ── Level conversions ──

    #[test]
    fn test_level_conversions() {
        assert_eq!(LogLevel::Trace.to_str(), "TRACE");
        assert_eq!(LogLevel::Debug.to_str(), "DEBUG");
        assert_eq!(LogLevel::Info.to_str(), "INFO");
        assert_eq!(LogLevel::Warn.to_str(), "WARN");
        assert_eq!(LogLevel::Error.to_str(), "ERROR");
        assert_eq!(LogLevel::Fatal.to_str(), "FATAL");
        assert_eq!(LogLevel::Audit.to_str(), "AUDIT");

        assert_eq!(LogLevel::from_u8(0), Some(LogLevel::Trace));
        assert_eq!(LogLevel::from_u8(6), Some(LogLevel::Audit));
        assert_eq!(LogLevel::from_u8(7), None);
    }

    // ── uint128 conversions (kept for compatibility) ──

    #[test]
    fn test_uint128_conversions() {
        use crate::ffi::dologger_uint128_t;
        let v = dologger_uint128_t {
            hi: 0x0102_0304_0506_0708,
            lo: 0x090A_0B0C_0D0E_0F10,
        };
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&v.hi.to_le_bytes());
        bytes[8..].copy_from_slice(&v.lo.to_le_bytes());
        let v2 = dologger_uint128_t {
            hi: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            lo: u64::from_le_bytes(bytes[8..].try_into().unwrap()),
        };
        assert_eq!(v, v2);
    }

    // ── Drop and cleanup tests ──

    #[test]
    fn test_record_drop_frees_kv_ext() {
        let mut r = Record::new(0);
        // Fill both inline slots and force ext allocation
        r.set_trace_id("t1");
        r.set_span_id("s1");
        r.set_user_id("u1");
        assert!(!r.kv_ext.is_null());
        // Drop should free kv_ext without memory leak
        drop(r);
    }

    // ── Clone tests ──

    #[test]
    fn test_record_string_clone() {
        let mut s = RecordString::empty();
        s.set("hello");
        let s2 = s.clone();
        assert_eq!(s2.as_utf8().unwrap(), "hello");
    }
}
