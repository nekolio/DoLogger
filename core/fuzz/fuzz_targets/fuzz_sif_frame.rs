#![no_main]

//! Fuzz target for the SIF frame validator and decoder.
//!
//! The target intentionally exercises validation before decode and keeps the
//! default resource limits. A malformed input must return an error, never
//! panic, or allocate unbounded memory.

use dologger_core::sif::{decode_record_with, entries, validate_frame_with, DecodeOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let options = DecodeOptions::untrusted();
    let _ = validate_frame_with(data, options);
    let _ = entries(data);
    let _ = decode_record_with(data, options);
});
