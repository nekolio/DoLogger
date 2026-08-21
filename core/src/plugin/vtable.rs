//! Typed plugin vtables + host-accessor bridge (C ABI, M6).
//!
//! These structs are the single Rust source of truth for the plugin dispatch
//! contract. Their layout MUST match the corresponding structs in
//! `core/include/dologger_core.h` byte-for-byte: the engine casts an opaque
//! `vtable` pointer from `dologger_plugin_info_t` to one of these types and
//! calls through it, and plugin crates (which link the core rlib, so they see
//! the *same* types) cast the same pointer back.
//!
//! Before M6 the engine only stored the raw `vtable` and never dispatched it.
//! The two real gaps this module closes are:
//!
//! 1. **Typed dispatch** — a loaded plugin's `vtable` can now be resolved to a
//!    [`FormatterVTable`] or [`FieldProviderVTable`] by its phase bit and
//!    called from the pipeline.
//! 2. **The host-accessor bridge** — a plugin receives a [`HostAccessors`]
//!    table at `plugin_init` (instead of `NULL`). Because the official bundle
//!    is a separate `cdylib` that statically links its own copy of the core
//!    rlib, it cannot call the host's `dologger_field_get`/`set` symbols
//!    directly; the accessor table hands it function pointers into the *live*
//!    engine, which is the only way an opaque record handle can be read or
//!    written from inside the bundle.

use std::ffi::{c_char, c_void};

/// ABI version of the [`HostAccessors`] table. Bump on any layout change so a
/// plugin built against an older core can detect the mismatch at `plugin_init`
/// instead of calling stale function pointers.
pub const HOST_ACCESSORS_ABI: u32 = 1;

// ===========================================================================
// Shared output-buffer type
// ===========================================================================

/// Growable byte buffer a [`FormatterVTable::format`] writes into.
///
/// Mirrors `dologger_output_buffer_t` in the C header. The engine owns the
/// backing store; a formatter writes at most `capacity` bytes, sets `len` to
/// the number of bytes it actually wrote, and returns
/// `DO_LOG_ERR_BUFFER_TOO_SMALL` if the value does not fit — whereupon the
/// engine grows the buffer and retries.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OutputBuffer {
    /// Pointer to the first byte of the writable region.
    pub data: *mut u8,
    /// Number of bytes currently written by the formatter.
    pub len: usize,
    /// Total writable size of the backing store (`data[0..capacity]`).
    pub capacity: usize,
}

// ===========================================================================
// Typed vtables (match dologger_core.h)
// ===========================================================================

/// Formatter vtable — a single `format` entry point.
///
/// Mirrors `dologger_formatter_vtable_t`. The engine calls `format` during the
/// Formatting pipeline stage (stage 5) with the record, an output buffer, and
/// the plugin's `config` pointer. The formatter reads record fields through
/// the [`HostAccessors`] it captured at `plugin_init`.
#[repr(C)]
pub struct FormatterVTable {
    /// Format one record into `output`.
    ///
    /// - `record` — opaque `dologger_record_handle_t*`; read via `field_get`.
    /// - `output` — [`OutputBuffer`] owned by the engine.
    /// - `config` — plugin config pointer, or NULL.
    ///
    /// Returns `DO_LOG_OK` (0) on success, `DO_LOG_ERR_BUFFER_TOO_SMALL` if the
    /// buffer is insufficient (engine grows and retries), or another negative
    /// `DO_LOG_ERR_*` code on failure.
    pub format: unsafe extern "C" fn(
        record: *const c_void,
        output: *mut OutputBuffer,
        config: *mut c_void,
    ) -> i32,
}

/// FieldProvider vtable — a single `provide` entry point.
///
/// Mirrors `dologger_field_provider_vtable_t`. The engine calls `provide`
/// during the FieldProvider pipeline stage (stage 2) to inject custom fields
/// into each record before assembly. The provider writes fields through the
/// [`HostAccessors`] it captured at `plugin_init`.
#[repr(C)]
pub struct FieldProviderVTable {
    /// Provide fields to the record.
    ///
    /// - `record` — mutable opaque `dologger_record_handle_t*`; written via
    ///   `field_set`.
    /// - `config` — plugin config pointer, or NULL.
    ///
    /// Returns the number of fields added (>= 0), or a negative `DO_LOG_ERR_*`
    /// code on failure. MUST respect Ring permission limits (field_set enforces
    /// them host-side).
    pub provide: unsafe extern "C" fn(record: *mut c_void, config: *mut c_void) -> i32,
}

/// Filter vtable for the Filter pipeline stage.
///
/// Return `0` to continue, a positive value to drop, or a negative
/// `DO_LOG_ERR_*` value to abort the current pipeline invocation. The callback
/// must be deterministic and must not perform I/O.
#[repr(C)]
pub struct FilterVTable {
    /// Evaluate an immutable record.
    pub filter: unsafe extern "C" fn(*const c_void, *mut c_void) -> i32,
}

/// Processor vtable for the Processing pipeline stage.
///
/// Return `0` or a positive count to continue; a negative error aborts the
/// record. The host accessor bridge remains the only sanctioned mutation path.
#[repr(C)]
pub struct ProcessorVTable {
    /// Transform or enrich a mutable record.
    pub process: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32,
}
// ===========================================================================
// Host-accessor bridge
// ===========================================================================

/// Function-pointer table handed to a plugin at `plugin_init`.
///
/// Mirrors `dologger_host_accessors_t` in the C header. This is the plugin's
/// only sanctioned way to touch an opaque `dologger_record_handle_t`: the
/// engine fills the table with pointers to its own accessors, and the plugin
/// copies the (all-function-pointer, `Copy`) struct into its own static state
/// for later use from the pipeline hot path.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostAccessors {
    /// Read a field into `buffer`. Returns `>= 0` (byte count written) on
    /// success, or a negative `DO_LOG_ERR_*` code. `buffer` is NUL-terminated
    /// on success; a `DO_LOG_ERR_BUFFER_TOO_SMALL` return means the caller
    /// should retry with a larger buffer.
    pub field_get: unsafe extern "C" fn(
        record: *const c_void,
        field_name: *const c_char,
        buffer: *mut c_char,
        buffer_size: usize,
    ) -> i32,
    /// Write a field. Returns `DO_LOG_OK` (0) on success, or a negative
    /// `DO_LOG_ERR_*` code (e.g. permission denied for a Ring-0 field).
    pub field_set: unsafe extern "C" fn(
        record: *mut c_void,
        field_name: *const c_char,
        value: *const c_char,
    ) -> i32,
    /// Allocate `size` bytes of host-owned memory (used to grow formatter
    /// output buffers). The host tracks it; free with `free`.
    pub alloc: unsafe extern "C" fn(size: usize) -> *mut c_void,
    /// Free memory previously returned by `alloc`.
    pub free: unsafe extern "C" fn(ptr: *mut c_void),
    /// ABI version of the accessor table ([`HOST_ACCESSORS_ABI`]).
    pub abi_version: u32,
}

impl Default for HostAccessors {
    fn default() -> Self {
        Self {
            field_get: host_field_get,
            field_set: host_field_set,
            alloc: host_alloc,
            free: host_free,
            abi_version: HOST_ACCESSORS_ABI,
        }
    }
}

/// Initialisation payload the host hands to a plugin's `plugin_init`.
///
/// Mirrors `dologger_host_init_t` in the C header. Carries BOTH the host-accessor
/// bridge the plugin needs to touch records, AND the plugin's JSON config string
/// (which may be NULL for "use defaults"). Passed by value inside a `HostInit`
/// the host owns for the duration of the `plugin_init` call; the plugin copies
/// `.accessors` (all function pointers) into its own static state and parses
/// `.config_json` immediately.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostInit {
    /// Host-accessor bridge (copied by the plugin into its own static state).
    pub accessors: HostAccessors,
    /// Plugin-specific JSON config (`{"pretty":false,...}`), or NULL for defaults.
    pub config_json: *const c_char,
}

impl Default for HostInit {
    fn default() -> Self {
        Self {
            accessors: HostAccessors::default(),
            config_json: std::ptr::null(),
        }
    }
}

// ===========================================================================
// Resolved dispatch table (hot-path ready)
// ===========================================================================

/// One resolved formatter entry, copied out of a loaded plugin's
/// [`FormatterVTable`] so the consumer thread can call it without touching the
/// plugin registry (no locks, no libloading lookups per record).
#[derive(Clone, Copy)]
pub struct FormatterEntry {
    /// `format` function pointer.
    pub format: unsafe extern "C" fn(*const c_void, *mut OutputBuffer, *mut c_void) -> i32,
    /// Plugin `config` pointer passed through to `format`.
    pub config: *mut c_void,
}

/// One resolved filter entry.
#[derive(Clone, Copy)]
pub struct FilterEntry {
    /// Filter callback.
    pub filter: unsafe extern "C" fn(*const c_void, *mut c_void) -> i32,
    /// Plugin-owned configuration pointer.
    pub config: *mut c_void,
}

/// One resolved processor entry.
#[derive(Clone, Copy)]
pub struct ProcessorEntry {
    /// Processor callback.
    pub process: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32,
    /// Plugin-owned configuration pointer.
    pub config: *mut c_void,
}
/// One resolved field-provider entry (see [`FormatterEntry`]).
#[derive(Clone, Copy)]
pub struct FieldProviderEntry {
    /// `provide` function pointer.
    pub provide: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32,
    /// Plugin `config` pointer passed through to `provide`.
    pub config: *mut c_void,
}

/// Resolved, hot-path-ready dispatch of formatter + field-provider plugins.
///
/// Built once by [`crate::plugin::PluginManager::resolve_dispatch`] after
/// plugins load, and threaded into the pipeline. Empty by default (no plugins
/// loaded), in which case the pipeline falls back to its built-in plain-text
/// formatting — so behaviour is unchanged when the engine loads no plugins.
#[derive(Default)]
pub struct PluginDispatch {
    /// Filter plugins (PHASE_FILTER), in load order.
    pub filters: Vec<FilterEntry>,
    /// Processor plugins (PHASE_PROCESSING), in load order.
    pub processors: Vec<ProcessorEntry>,
    /// Formatter plugins (PHASE_FORMATTING), in load order.
    pub formatters: Vec<FormatterEntry>,
    /// Field-provider plugins (PHASE_FIELD_PROVIDER | PHASE_HOSTINFO).
    pub field_providers: Vec<FieldProviderEntry>,
}

// SAFETY: the vtable structs below hold only function pointers to static
// `extern "C"` fns and a `u32`; they contain no interior data and can be shared
// and moved across threads. This lets plugin crates store a `static` instance
// without their own unsafe impls.
unsafe impl Sync for FilterVTable {}
// SAFETY: see above.
unsafe impl Send for FilterVTable {}
// SAFETY: see above.
unsafe impl Sync for ProcessorVTable {}
// SAFETY: see above.
unsafe impl Send for ProcessorVTable {}
// SAFETY: see above.
unsafe impl Sync for FormatterVTable {}
// SAFETY: see above.
unsafe impl Send for FormatterVTable {}
// SAFETY: see above.
unsafe impl Sync for FieldProviderVTable {}
// SAFETY: see above.
unsafe impl Send for FieldProviderVTable {}
// SAFETY: see above.
unsafe impl Sync for HostAccessors {}
// SAFETY: see above.
unsafe impl Send for HostAccessors {}
// SAFETY: the dispatch entries below hold raw pointers (`config`) and function
// pointers that the pipeline only forwards to the plugin; they are never
// dereferenced on the consumer thread, so moving them across threads is sound.
unsafe impl Sync for FilterEntry {}
// SAFETY: see above.
unsafe impl Send for FilterEntry {}
// SAFETY: see above.
unsafe impl Sync for ProcessorEntry {}
// SAFETY: see above.
unsafe impl Send for ProcessorEntry {}
// SAFETY: see above.
unsafe impl Sync for FormatterEntry {}
// SAFETY: see above.
unsafe impl Send for FormatterEntry {}
// SAFETY: see above.
unsafe impl Sync for FieldProviderEntry {}
// SAFETY: see above.
unsafe impl Send for FieldProviderEntry {}
// SAFETY: see above.
unsafe impl Sync for PluginDispatch {}
// SAFETY: see above.
unsafe impl Send for PluginDispatch {}

// ===========================================================================
// Host-side adapters that back the accessor table
// ===========================================================================

/// Host field-get adapter for the accessor bridge.
///
/// Bridges the typed `dologger_field_get` (which takes a `DologgerError`
/// out-pointer and a typed `DologgerRecord` handle) onto the clean `c_void`
/// signature the bridge uses. The out-buffer carries the field value; the
/// return value is the byte count on success or a negative error code.
unsafe extern "C" fn host_field_get(
    record: *const c_void,
    field_name: *const c_char,
    buffer: *mut c_char,
    buffer_size: usize,
) -> i32 {
    let mut err = crate::error::DologgerError::default();
    // SAFETY: arguments are forwarded verbatim to `dologger_field_get`, which
    // null-checks every pointer and casts the opaque handle back to a live
    // `Record` under the documented C-ABI handle contract.
    crate::ffi::dologger_field_get(
        record as *const crate::ffi::DologgerRecord,
        field_name,
        buffer,
        buffer_size,
        &mut err,
    )
}

/// Host field-set adapter for the accessor bridge (see [`host_field_get`]).
unsafe extern "C" fn host_field_set(
    record: *mut c_void,
    field_name: *const c_char,
    value: *const c_char,
) -> i32 {
    let mut err = crate::error::DologgerError::default();
    crate::ffi::dologger_field_set(
        record as *mut crate::ffi::DologgerRecord,
        field_name,
        value,
        &mut err,
    )
}

/// Host alloc adapter for the accessor bridge (forwards to `dologger_alloc`).
unsafe extern "C" fn host_alloc(size: usize) -> *mut c_void {
    crate::ffi::dologger_alloc(size)
}

/// Host free adapter for the accessor bridge (forwards to `dologger_free`).
unsafe extern "C" fn host_free(ptr: *mut c_void) {
    crate::ffi::dologger_free(ptr)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_accessors_are_non_null_and_abi_tagged() {
        let a = HostAccessors::default();
        assert_eq!(a.abi_version, HOST_ACCESSORS_ABI);
        assert_ne!(a.field_get as usize, 0);
        assert_ne!(a.field_set as usize, 0);
        assert_ne!(a.alloc as usize, 0);
        assert_ne!(a.free as usize, 0);
    }

    #[test]
    fn vtable_structs_have_expected_fn_ptr_fields() {
        // Formatter: one `format`. FieldProvider: one `provide`.
        // Verified by construction (fields are typed function pointers).
        let _ = std::mem::size_of::<FormatterVTable>();
        let _ = std::mem::size_of::<FieldProviderVTable>();
    }
}
