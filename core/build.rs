//! Build script for libdologger_core.
//!
//! Responsibilities:
//! - Declare rebuild triggers. The crate version is exposed to host apps via
//!   `dologger_version()` (FFI), which reads `CARGO_PKG_VERSION` directly —
//!   no build-time env emission is needed.
//! - Planned: FlatBuffers code generation (SIF schema), bindgen for plugin headers

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
