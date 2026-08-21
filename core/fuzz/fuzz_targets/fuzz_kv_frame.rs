#![no_main]

//! Fuzz target for the versioned KV frame validator and compatibility reader.
//!
//! The target intentionally exercises validation before decode and keeps the
//! default resource limits. A malformed input must return an error, never
//! panic, allocate unbounded memory, or call into the legacy decoder unless
//! its magic is valid.

use dologger_core::record::wire::{decode_any, entries, validate_frame_with, DecodeOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let options = DecodeOptions::untrusted();
    let _ = validate_frame_with(data, options);
    let _ = entries(data);
    let _ = decode_any(data, options);
});
