//! Record structure with field permission rings (Ring 0–3).
//!
//! # Ring Model
//!
//! | Ring   | Description                              | Access       |
//! |--------|------------------------------------------|--------------|
//! | Ring 0 | Kernel-core (ID, timestamp, signature)   | Core R/W, Formatter/Sink read-only via dedicated API |
//! | Ring 1 | System trusted (level, message, host)    | Core + HostInfoProvider write, plugins read-only |
//! | Ring 2 | Verified plugin fields                   | Blue/Yellow plugins R/W, audit-tagged |
//! | Ring 3 | Untrusted extension fields               | Any plugin R/W, CRC32C only |

use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::Arc;

use crate::ffi::dologger_uint128_t;

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
// Record struct
// ---------------------------------------------------------------------------

/// A log record with field permission rings.
///
/// # Memory Layout
///
/// The struct is organized with cache-line awareness:
/// - Ring 0 hot fields are in the first cache line (64 bytes aligned)
/// - Ring 3 cold/tail fields are at the end to avoid false sharing.
#[repr(C, align(64))]
pub struct Record {
    // ── Ring 0: Kernel-core fields (always present, immutable by plugins) ──
    /// Globally unique record ID (128-bit, modified snowflake algorithm)
    pub id: dologger_uint128_t,
    /// Nanosecond-precision UTC timestamp (128-bit)
    pub timestamp: dologger_uint128_t,
    /// Ed25519 signature covering Ring 0 + Ring 1 fields (64 bytes)
    pub signature: [u8; 64],
    /// Origin LSN if received from another DoLogger node, 0 otherwise
    pub origin_lsn: u64,

    // ── Ring 1: System trusted fields (core + HostInfoProvider writable) ──
    /// Current log level
    pub level: LogLevel,
    /// Log message body (UTF-8, fixed buffer for small messages)
    pub message: RecordString,
    /// Source file name (optional, Ring 1)
    pub source_file: RecordString,
    /// Source function name (optional, Ring 1)
    pub source_function: RecordString,
    /// Source line number (optional, Ring 1)
    pub source_line: u32,
    /// Source column number (optional, Ring 1)
    pub source_column: u32,
    /// Thread ID
    pub thread_id: u64,
    /// Thread name
    pub thread_name: RecordString,
    /// Process ID
    pub process_id: u32,
    /// Process name
    pub process_name: RecordString,
    /// Host name
    pub host_name: RecordString,
    /// Container ID
    pub container_id: RecordString,
    /// Application name
    pub app_name: RecordString,
    /// Application version
    pub app_version: RecordString,
    /// Environment (dev/test/staging/prod)
    pub environment: RecordString,
    /// User identifier
    pub user_id: RecordString,
    /// Session identifier
    pub session_id: RecordString,
    /// Request/trace identifiers (W3C Trace Context / OpenTelemetry)
    pub request_id: RecordString,
    /// Distributed trace ID (W3C Trace Context / OpenTelemetry compatible)
    pub trace_id: RecordString,
    /// Span ID within a distributed trace
    pub span_id: RecordString,

    /// Coroutine ID (for async runtimes: tokio task id, go goroutine id)
    pub coroutine_id: u64,

    // ── Ring 1 Exception fields ──
    /// Exception type name (e.g., "std::runtime_error", "panic")
    pub exception_type: RecordString,
    /// Exception message
    pub exception_message: RecordString,
    /// Exception stack trace
    pub exception_stacktrace: RecordString,
    /// Exception error code
    pub exception_code: i32,

    /// Key-value labels (JSON object string, e.g. {"key":"val"})
    pub labels: RecordString,

    // ── Ring 1 Security fields ──
    /// Log Sequence Number (monotonically increasing, uint64_t)
    pub lsn: u64,
    /// SHA-256 hash of previous audit record's (LSN || Signature)
    pub prev_hash: [u8; 32],
    /// WORM LSN gap marker flag
    pub security_gap: bool,

    // ── Ring 1 Audit tags (JSON string array) ──
    /// JSON-encoded array of audit tags tracking field modifications
    pub audit_tags: RecordString,

    // ── Ring 3: Untrusted extension fields (CRC32C protected) ──
    /// Extension data blob (CRC32C checksummed, not in signature coverage)
    pub ext_data: RecordString,
    /// CRC32C checksum of extension data
    pub ext_crc32c: u32,

    // ── Internal bookkeeping ──
    /// Slot index in the object pool (used for return)
    pub(crate) pool_index: u32,
    /// Record flags (bitfield: 0x01 = in_use, 0x02 = signed, 0x04 = audit)
    pub(crate) flags: u32,
    /// Padding to maintain cache line alignment
    pub(crate) _padding: [u8; 44],
}

// ---------------------------------------------------------------------------
// RecordString: Fixed-buffer string with overflow fallback
// ---------------------------------------------------------------------------

/// Size of the `RecordString` union (one cache line, keeps `Record` layout fixed).
pub const RECORD_STRING_INLINE_CAPACITY: usize = 256;

/// Maximum bytes stored inline. Byte `255` is the variant sentinel, so inline
/// content is capped at 254 bytes with the NUL terminator at index `len`.
/// Messages ≥ 255 bytes use the heap (`Arc<str>`) variant instead.
pub const RECORD_STRING_INLINE_MAX: usize = RECORD_STRING_INLINE_CAPACITY - 2;

/// A string stored either inline (≤ 254 bytes) or on the heap.
///
/// This avoids heap allocation for the vast majority of log messages,
/// while still supporting arbitrarily long messages when needed.
///
/// The two variants share one 256-byte `#[repr(C)]` union so the outer
/// [`Record`] layout stays fixed. The active variant is tracked by a sentinel
/// in the union's last byte:
///
/// | `inline[255]` | Variant                                        |
/// |:-------------:|:-----------------------------------------------|
/// | `0xFF`        | inline — NUL-terminated bytes at `[0..254]`     |
/// | `0`           | heap  — `Arc<str>` (fat pointer) at `[0..16)`   |
///
/// `empty()` and every teardown write the sentinel explicitly, so a torn or
/// zeroed union can never be misread as the heap variant.
#[repr(C)]
pub union RecordString {
    /// Inline fixed-size buffer (fast path for short strings).
    /// Byte `inline[255]` is the variant sentinel, not message content.
    inline: [u8; RECORD_STRING_INLINE_CAPACITY],
    /// Heap path: reference-counted string for messages ≥ 255 bytes.
    /// Only bytes `[0..16)` are used; byte 255 holds the sentinel `0`.
    heap: ManuallyDrop<Arc<str>>,
}

impl RecordString {
    /// Create an empty RecordString in the inline variant.
    ///
    /// `inline[255]` is set to the inline sentinel so the all-zeros layout
    /// (used by `Record::new`) never reads as a live heap pointer.
    pub const fn empty() -> Self {
        let mut inline = [0u8; RECORD_STRING_INLINE_CAPACITY];
        inline[RECORD_STRING_INLINE_CAPACITY - 1] = 0xFF;
        Self { inline }
    }

    /// True when the heap variant is active (sentinel byte != `0xFF`).
    #[inline]
    fn is_heap(&self) -> bool {
        // SAFETY: reading a single `u8` through the inline field is valid
        // regardless of the active variant (`u8` is always well-formed).
        unsafe { self.inline[RECORD_STRING_INLINE_CAPACITY - 1] != 0xFF }
    }

    /// Set the string value, using heap fallback for strings ≥ 255 bytes.
    pub fn set(&mut self, s: &str) {
        self.drop_heap();
        let bytes = s.as_bytes();
        if bytes.len() > RECORD_STRING_INLINE_MAX {
            // Heap path — preserve the full length.
            // SAFETY: drop_heap() freed any prior heap string; the Arc is
            // stored in bytes [0..16) and the sentinel write at byte 255 does
            // not overlap it.
            unsafe {
                self.heap = ManuallyDrop::new(Arc::from(s));
                self.inline[RECORD_STRING_INLINE_CAPACITY - 1] = 0;
            }
        } else {
            // Inline path — NUL-terminated at bytes.len() (≤ 254, so the
            // sentinel byte 255 stays free).
            // SAFETY: bytes.len() ≤ 254 and both source and destination
            // pointers are valid for that many bytes.
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), self.inline.as_mut_ptr(), bytes.len());
                self.inline[bytes.len()] = 0;
                self.inline[RECORD_STRING_INLINE_CAPACITY - 1] = 0xFF;
            }
        }
    }

    /// Get the string as a `&str`.
    pub fn as_str(&self) -> &str {
        if self.is_heap() {
            // SAFETY: is_heap() confirms the heap variant is active; the Arc
            // is never moved out of the union, so the borrowed `&str` stays
            // tied to `self` and valid for as long as `self` is unmutated.
            unsafe { &self.heap }
        } else {
            // SAFETY: the inline variant is always NUL-terminated
            // (guaranteed by empty(), set(), and clear()).
            unsafe {
                let len = self.inline.iter().position(|&b| b == 0).unwrap_or(0);
                let slice = std::slice::from_raw_parts(self.inline.as_ptr(), len);
                std::str::from_utf8(slice).unwrap_or("")
            }
        }
    }

    /// Get the length of the string (excluding NUL terminator).
    pub fn len(&self) -> usize {
        if self.is_heap() {
            // SAFETY: heap variant is active — `str::len` via the Arc deref.
            unsafe { self.heap.len() }
        } else {
            // SAFETY: inline variant is always NUL-terminated.
            unsafe { self.inline.iter().position(|&b| b == 0).unwrap_or(0) }
        }
    }

    /// Check if the string is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Free the heap string, if any. A no-op for the inline variant — inline
    /// content is left untouched because `set()` overwrites it immediately
    /// (this keeps the hot path free of a 256-byte memset).
    fn drop_heap(&mut self) {
        if self.is_heap() {
            // SAFETY: is_heap() confirms the heap variant is active.
            unsafe {
                let arc: Arc<str> = ManuallyDrop::take(&mut self.heap);
                drop(arc);
            }
        }
    }

    /// Fully clear the string — frees any heap Arc and reinitializes the union
    /// to the empty inline state. Used by pool reuse ([`Record::reset`]) and
    /// [`Drop`], so a recycled slot neither leaks an `Arc<str>` nor serves
    /// stale inline content.
    fn clear(&mut self) {
        self.drop_heap();
        // SAFETY: writing the full `[u8; 256]` reinitializes the inline
        // variant; the sentinel makes the empty state unambiguous.
        unsafe {
            self.inline = [0u8; RECORD_STRING_INLINE_CAPACITY];
            self.inline[RECORD_STRING_INLINE_CAPACITY - 1] = 0xFF;
        }
    }
}

impl std::fmt::Debug for RecordString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{}\"", self.as_str())
    }
}

// SAFETY: both variants are Send — inline is plain data and the heap variant
// is an `Arc<str>` (Send + Sync). The union is only ever mutated through
// `&mut self`, so no data race is possible.
unsafe impl Send for RecordString {}
// SAFETY: see Send impl — `Arc<str>` is Sync, inline is plain data.
unsafe impl Sync for RecordString {}

// ---------------------------------------------------------------------------
// Field access control
// ---------------------------------------------------------------------------

/// Identifies which permission ring a field belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRing {
    /// Core kernel fields — only core engine and output-stage plugins (read-only)
    Ring0,
    /// System trusted fields — core + HostInfoProvider write
    Ring1,
    /// Verified plugin fields — blue/yellow plugins
    Ring2,
    /// Untrusted extension fields — any plugin, CRC32C only
    Ring3,
}

/// Free every [`RecordString`] field. Shared by [`Record::reset`] (pool reuse)
/// and [`Drop`] so the two teardown paths stay in lockstep.
fn drop_record_strings(r: &mut Record) {
    r.message.clear();
    r.source_file.clear();
    r.source_function.clear();
    r.thread_name.clear();
    r.process_name.clear();
    r.host_name.clear();
    r.container_id.clear();
    r.app_name.clear();
    r.app_version.clear();
    r.environment.clear();
    r.user_id.clear();
    r.session_id.clear();
    r.request_id.clear();
    r.trace_id.clear();
    r.span_id.clear();
    r.exception_type.clear();
    r.exception_message.clear();
    r.exception_stacktrace.clear();
    r.labels.clear();
    r.audit_tags.clear();
    r.ext_data.clear();
}

impl Drop for Record {
    /// Safety net for records that leave scope without passing through
    /// [`Record::reset`] (e.g. test-constructed records). Production records
    /// are pool-owned and `reset()` runs first, so this is a no-op for them.
    fn drop(&mut self) {
        drop_record_strings(self);
    }
}

impl Record {
    /// Create a new empty record (all fields zeroed).
    /// For production use, allocate via RecordPool. This constructor is
    /// public for testing and benchmarks.
    pub const fn new(pool_index: u32) -> Self {
        Self {
            id: dologger_uint128_t { hi: 0, lo: 0 },
            timestamp: dologger_uint128_t { hi: 0, lo: 0 },
            signature: [0u8; 64],
            origin_lsn: 0,
            level: LogLevel::Info,
            message: RecordString::empty(),
            source_file: RecordString::empty(),
            source_function: RecordString::empty(),
            source_line: 0,
            source_column: 0,
            thread_id: 0,
            thread_name: RecordString::empty(),
            process_id: 0,
            process_name: RecordString::empty(),
            host_name: RecordString::empty(),
            container_id: RecordString::empty(),
            app_name: RecordString::empty(),
            app_version: RecordString::empty(),
            environment: RecordString::empty(),
            user_id: RecordString::empty(),
            session_id: RecordString::empty(),
            request_id: RecordString::empty(),
            trace_id: RecordString::empty(),
            span_id: RecordString::empty(),
            coroutine_id: 0,
            exception_type: RecordString::empty(),
            exception_message: RecordString::empty(),
            exception_stacktrace: RecordString::empty(),
            exception_code: 0,
            labels: RecordString::empty(),
            lsn: 0,
            prev_hash: [0u8; 32],
            security_gap: false,
            audit_tags: RecordString::empty(),
            ext_data: RecordString::empty(),
            ext_crc32c: 0,
            pool_index,
            flags: 0,
            _padding: [0u8; 44],
        }
    }

    /// Reset the record for reuse (called when returning to pool).
    ///
    /// Clears flags/signature and frees every [`RecordString`] field so the
    /// slot returns to a pristine state — a reused slot must neither leak an
    /// `Arc<str>` nor serve stale field content.
    pub(crate) fn reset(&mut self) {
        self.flags = 0;
        self.signature = [0u8; 64];
        self.ext_crc32c = 0;
        drop_record_strings(self);
    }

    /// Get the permission ring for a field by name.
    ///
    /// Returns `None` if the field name is not recognized.
    pub fn field_ring(field_name: &str) -> Option<FieldRing> {
        match field_name {
            // Ring 0 — read-only for all plugins
            "record.id" | "record.timestamp" | "record.signature" | "record.origin_lsn" => {
                Some(FieldRing::Ring0)
            }
            // Ring 1 — system trusted
            "level"
            | "message"
            | "source.file"
            | "source.function"
            | "source.line"
            | "source.column"
            | "thread.id"
            | "thread.name"
            | "process.id"
            | "process.name"
            | "host.name"
            | "container.id"
            | "app.name"
            | "app.version"
            | "environment"
            | "user.id"
            | "session.id"
            | "request.id"
            | "trace.id"
            | "span.id"
            | "coroutine.id"
            | "exception.type"
            | "exception.message"
            | "exception.stacktrace"
            | "exception.code"
            | "labels"
            | "security.lsn"
            | "security.prev_hash"
            | "security.gap"
            | "security.audit_tags" => Some(FieldRing::Ring1),
            // Ring 2 — verified plugin fields
            // Uses "verified." prefix namespace; modifications auto-append audit_tags
            _ if field_name.starts_with("verified.") => Some(FieldRing::Ring2),
            // Ring 3 — untrusted extension fields
            _ if field_name.starts_with("ext.") => Some(FieldRing::Ring3),
            // Unknown fields default to None (deny by default for safety)
            _ => None,
        }
    }

    /// Set a field by name with ring permission checking.
    ///
    /// `caller_ring` is the ring level of the caller (core = Ring0, HostInfo = Ring1, etc.)
    /// Returns `Ok(())` on success, or an error string on permission denial.
    pub fn field_set(
        &mut self,
        field_name: &str,
        value: &str,
        caller_ring: FieldRing,
    ) -> Result<(), &'static str> {
        let target_ring = Self::field_ring(field_name).unwrap_or(FieldRing::Ring3);

        match target_ring {
            FieldRing::Ring0 => {
                // Ring 0 fields (id/timestamp/signature/origin_lsn) are owned
                // by the engine and populated through dedicated methods
                // (RecordPool, TimeSource, SignatureEngine). They are not
                // settable through the string field API: non-Ring0 callers get
                // a permission denial, and even a Ring0 caller gets a typed
                // error rather than a silent success.
                if !matches!(caller_ring, FieldRing::Ring0) {
                    return Err("Permission denied: Ring 0 fields are read-only for plugins");
                }
                return Err("Ring 0 fields are engine-managed and not settable via field_set");
            }
            FieldRing::Ring1 => {
                // Only core or HostInfoProvider (Ring1 caller) can write Ring 1
                // Plugins attempting to write Ring 1 MUST trigger security alarm.
                if matches!(caller_ring, FieldRing::Ring2 | FieldRing::Ring3) {
                    crate::sys::diagnostics::error(
                        "security",
                        &format!(
                            "SECURITY_VIOLATION: Unauthorized Ring 1 write attempt to '{}' by caller {:?}",
                            field_name, caller_ring
                        ),
                    );
                    return Err(
                        "SECURITY_VIOLATION: Ring 1 fields require HostInfoProvider or core — plugin should be unloaded",
                    );
                }
                self.set_ring1_field(field_name, value);
            }
            FieldRing::Ring2 => {
                // Ring 2 — verified plugin fields. Append audit tag.
                self.set_ring2_field(field_name, value);
            }
            FieldRing::Ring3 => {
                // Ring 3 — untrusted, any caller can write
                self.set_ring3_ext(field_name, value);
            }
        }

        Ok(())
    }

    /// Read a field by name with ring permission checking.
    ///
    /// Returns `Some(&str)` on success, `None` if field not found, or error on permission denial.
    pub fn field_get(
        &self,
        field_name: &str,
        _caller_ring: FieldRing,
    ) -> Result<String, &'static str> {
        // Ring 0 fields are only readable by output-stage plugins (Formatter/Sink)
        // For now, we allow all callers to read for simplicity
        match field_name {
            "record.id" => Ok(format!("{:016x}{:016x}", self.id.hi, self.id.lo)),
            "record.timestamp" => Ok(format!("{}.{:09}", self.timestamp.hi, self.timestamp.lo)),
            "record.signature" => Ok(hex::encode(&self.signature[..16])), // truncated
            "record.origin_lsn" => Ok(self.origin_lsn.to_string()),
            "level" => Ok(self.level.to_str().to_string()),
            "message" => Ok(self.message.as_str().to_string()),
            "source.file" => Ok(self.source_file.as_str().to_string()),
            "source.function" => Ok(self.source_function.as_str().to_string()),
            "source.line" => Ok(self.source_line.to_string()),
            "source.column" => Ok(self.source_column.to_string()),
            "thread.id" => Ok(self.thread_id.to_string()),
            "thread.name" => Ok(self.thread_name.as_str().to_string()),
            "process.id" => Ok(self.process_id.to_string()),
            "process.name" => Ok(self.process_name.as_str().to_string()),
            "host.name" => Ok(self.host_name.as_str().to_string()),
            "container.id" => Ok(self.container_id.as_str().to_string()),
            "app.name" => Ok(self.app_name.as_str().to_string()),
            "app.version" => Ok(self.app_version.as_str().to_string()),
            "environment" => Ok(self.environment.as_str().to_string()),
            "user.id" => Ok(self.user_id.as_str().to_string()),
            "session.id" => Ok(self.session_id.as_str().to_string()),
            "request.id" => Ok(self.request_id.as_str().to_string()),
            "trace.id" => Ok(self.trace_id.as_str().to_string()),
            "span.id" => Ok(self.span_id.as_str().to_string()),
            "coroutine.id" => Ok(self.coroutine_id.to_string()),
            "exception.type" => Ok(self.exception_type.as_str().to_string()),
            "exception.message" => Ok(self.exception_message.as_str().to_string()),
            "exception.stacktrace" => Ok(self.exception_stacktrace.as_str().to_string()),
            "exception.code" => Ok(self.exception_code.to_string()),
            "labels" => Ok(self.labels.as_str().to_string()),
            "security.lsn" => Ok(self.lsn.to_string()),
            "security.prev_hash" => Ok(hex::encode(&self.prev_hash[..8])),
            "security.gap" => Ok(self.security_gap.to_string()),
            "security.audit_tags" => Ok(self.audit_tags.as_str().to_string()),
            _ if field_name.starts_with("ext.") => Ok(self.ext_data.as_str().to_string()),
            _ => Err("Field not found"),
        }
    }

    // -- Internal helpers --

    fn set_ring1_field(&mut self, field_name: &str, value: &str) {
        match field_name {
            "message" => self.message.set(value),
            "source.file" => self.source_file.set(value),
            "source.function" => self.source_function.set(value),
            "source.line" => {
                if let Ok(n) = value.parse() {
                    self.source_line = n;
                }
            }
            "source.column" => {
                if let Ok(n) = value.parse() {
                    self.source_column = n;
                }
            }
            "thread.id" => {
                if let Ok(n) = value.parse() {
                    self.thread_id = n;
                }
            }
            "thread.name" => self.thread_name.set(value),
            "process.id" => {
                if let Ok(n) = value.parse() {
                    self.process_id = n;
                }
            }
            "process.name" => self.process_name.set(value),
            "host.name" => self.host_name.set(value),
            "container.id" => self.container_id.set(value),
            "app.name" => self.app_name.set(value),
            "app.version" => self.app_version.set(value),
            "environment" => self.environment.set(value),
            "user.id" => self.user_id.set(value),
            "session.id" => self.session_id.set(value),
            "request.id" => self.request_id.set(value),
            "trace.id" => self.trace_id.set(value),
            "span.id" => self.span_id.set(value),
            "coroutine.id" => {
                if let Ok(n) = value.parse() {
                    self.coroutine_id = n;
                }
            }
            "exception.type" => self.exception_type.set(value),
            "exception.message" => self.exception_message.set(value),
            "exception.stacktrace" => self.exception_stacktrace.set(value),
            "exception.code" => {
                if let Ok(n) = value.parse() {
                    self.exception_code = n;
                }
            }
            "labels" => self.labels.set(value),
            "security.gap" => {
                if let Ok(b) = value.parse() {
                    self.security_gap = b;
                }
            }
            "security.audit_tags" => self.audit_tags.set(value),
            _ => { /* unknown Ring 1 field — silently ignore */ }
        }
    }

    fn set_ring2_field(&mut self, field_name: &str, value: &str) {
        // Ring 2: Append to audit_tags as JSON array entry with
        // plugin_id, version, field, value, and timestamp metadata.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        // Build a proper JSON entry (escaped strings for field/value)
        let escaped_field = field_name.replace('\\', "\\\\").replace('"', "\\\"");
        let escaped_value = value.replace('\\', "\\\\").replace('"', "\\\"");
        let entry = format!(
            r#"{{"plugin":"core","ver":"0.1.0","field":"{escaped_field}","val":"{escaped_value}","ts":{ts}}}"#
        );

        let mut current = self.audit_tags.as_str().to_string();
        if !current.is_empty() && !current.starts_with('[') {
            // Migrate legacy semicolon format to JSON array
            let legacy = current;
            current = format!(r#"["{legacy}"]"#);
        }
        if current == "[]" || current.is_empty() {
            current = format!("[{entry}]");
        } else {
            // Insert before the closing ']'
            current.pop(); // remove trailing ]
            if !current.ends_with('[') {
                current.push(',');
            }
            current.push_str(&entry);
            current.push(']');
        }
        self.audit_tags.set(&current);
    }

    fn set_ring3_ext(&mut self, _field_name: &str, value: &str) {
        self.ext_data.set(value);
        // Auto-compute CRC32C when Ring 3 extension data is written
        self.ext_crc32c = crate::security::crc32c(value.as_bytes());
    }
}

// Simple hex encoding helper (avoids an extra dependency)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ---------------------------------------------------------------------------
// Shared utilities
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

// Assert Record size and alignment at compile time
const _: () = {
    // Record must be cache-line aligned (64 bytes)
    assert!(core::mem::align_of::<Record>() == 64);
    // Record size must be a multiple of cache-line size to avoid false sharing
    assert!(core::mem::size_of::<Record>().is_multiple_of(64));
    // RecordString must stay a single 256-byte union (heap variant is a fat
    // pointer living in bytes [0..16); Record layout must not drift).
    assert!(core::mem::size_of::<RecordString>() == RECORD_STRING_INLINE_CAPACITY);
    assert!(core::mem::align_of::<RecordString>() == 8);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordstring_inline_roundtrip() {
        let mut rs = RecordString::empty();
        assert!(rs.is_empty());
        assert_eq!(rs.len(), 0);
        assert_eq!(rs.as_str(), "");
        rs.set("hello");
        assert!(!rs.is_empty());
        assert_eq!(rs.len(), 5);
        assert_eq!(rs.as_str(), "hello");
        rs.set("");
        assert!(rs.is_empty());
        assert_eq!(rs.as_str(), "");
    }

    #[test]
    fn recordstring_inline_heap_boundary() {
        let mut rs = RecordString::empty();
        // 254 bytes (INLINE_MAX) stays inline.
        let max_inline = "x".repeat(RECORD_STRING_INLINE_MAX);
        rs.set(&max_inline);
        assert_eq!(rs.len(), RECORD_STRING_INLINE_MAX);
        assert_eq!(rs.as_str(), max_inline);
        assert!(!rs.is_heap());
        // Re-setting with another short string must not corrupt the sentinel.
        rs.set("y");
        assert_eq!(rs.as_str(), "y");

        // 255 bytes (INLINE_MAX + 1) crosses to the heap path.
        let first_heap = "y".repeat(RECORD_STRING_INLINE_MAX + 1);
        rs.set(&first_heap);
        assert_eq!(rs.len(), RECORD_STRING_INLINE_MAX + 1);
        assert_eq!(rs.as_str(), first_heap);
        assert!(rs.is_heap());
    }

    #[test]
    fn recordstring_heap_roundtrip() {
        let mut rs = RecordString::empty();
        let s = "A".repeat(256);
        rs.set(&s);
        assert!(!rs.is_empty());
        assert_eq!(rs.len(), s.len());
        assert_eq!(rs.as_str(), s);
        // Debug impl must reflect the full heap content.
        let dbg = format!("{rs:?}");
        assert_eq!(dbg, format!("\"{s}\""));
    }

    #[test]
    fn recordstring_heap_switches_back_to_inline() {
        let mut rs = RecordString::empty();
        rs.set(&"B".repeat(500));
        assert_eq!(rs.len(), 500);
        assert_eq!(rs.as_str(), "B".repeat(500));
        // Replacing with a short string frees the heap Arc and returns inline.
        rs.set("short");
        assert_eq!(rs.as_str(), "short");
        assert!(!rs.is_heap());
    }

    #[test]
    fn recordstring_heap_len_matches_str_len() {
        let mut rs = RecordString::empty();
        rs.set(&"Z".repeat(4096));
        assert_eq!(rs.len(), rs.as_str().len());
        assert_eq!(rs.len(), 4096);
        assert_eq!(rs.as_str(), "Z".repeat(4096));
    }

    #[test]
    fn recordstring_unicode_roundtrip() {
        let mut rs = RecordString::empty();
        rs.set("こんにちは世界");
        assert_eq!(rs.as_str(), "こんにちは世界");
        rs.set(&"😀 ".repeat(300)); // multi-byte, crosses into heap
        assert_eq!(rs.as_str(), &"😀 ".repeat(300));
    }

    #[test]
    fn record_reset_frees_heap_strings() {
        let mut record = Record::new(0);
        record.message.set(&"M".repeat(300));
        record.source_file.set(&"F".repeat(300));
        assert_eq!(record.message.len(), 300);
        record.reset();
        assert_eq!(record.message.as_str(), "");
        assert_eq!(record.source_file.as_str(), "");
        assert!(record.message.is_empty());
    }

    #[test]
    fn record_reset_clears_inline_strings() {
        let mut record = Record::new(0);
        record.message.set("hello");
        record.ext_crc32c = 0xDEADBEEF;
        record.reset();
        assert_eq!(record.message.as_str(), "");
        assert_eq!(record.ext_crc32c, 0);
        assert_eq!(record.signature, [0u8; 64]);
    }

    #[test]
    fn record_drop_heap_string_smoke() {
        // A record with a heap string that leaves scope without reset() must
        // not panic or double-free (Drop is the safety net).
        let mut record = Record::new(7);
        record.message.set(&"D".repeat(400));
        record.host_name.set(&"H".repeat(400));
        assert_eq!(record.message.len(), 400);
        drop(record);
    }

    #[test]
    fn record_strings_are_shared_not_copied_after_drop() {
        // Dropping one record must not affect another record's independent
        // Arc (each set() allocates its own Arc — no aliasing across records).
        let mut a = Record::new(1);
        let mut b = Record::new(2);
        a.message.set(&"S".repeat(300));
        b.message.set(&"T".repeat(300));
        drop(a);
        assert_eq!(b.message.as_str(), "T".repeat(300));
    }
}
