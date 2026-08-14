//! Build script for libdologger_core.
//!
//! Responsibilities:
//! - Declare rebuild triggers. The crate version is exposed to host apps via
//!   `dologger_version()` (FFI), which reads `CARGO_PKG_VERSION` directly —
//!   no build-time env emission is needed.
//! - Regenerate FlatBuffers bindings (SIF schema) from `core/sif/dologger_sif.fbs`
//!   when `flatc` is available. The generated file is committed, so builds
//!   without `flatc` fall back to it.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=sif/dologger_sif.fbs");

    let out_path = Path::new("src/sif/dologger_sif_generated.rs");
    match Command::new("flatc")
        .args(["--rust", "-o", "src/sif/", "sif/dologger_sif.fbs"])
        .output()
    {
        Ok(out) if out.status.success() => {
            if !out_path.exists() {
                panic!("flatc succeeded but did not produce src/sif/dologger_sif_generated.rs");
            }
        }
        Ok(out) => {
            eprintln!(
                "flatc failed ({}): {}{}",
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            eprintln!("falling back to committed generated bindings");
        }
        Err(e) => {
            eprintln!("flatc not available ({e}); using committed generated bindings");
        }
    }
}
