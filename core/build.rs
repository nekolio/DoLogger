//! Build script for libdologger_core.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/sif/codec.rs");
    println!("cargo:rerun-if-changed=sif/dologger_sif.fbs");
    println!("cargo:rerun-if-changed=../.git/HEAD");

    let output = Path::new("src/sif/backends/dologger_sif_generated.rs");
    match Command::new("flatc")
        .args(["--rust", "-o", "src/sif/backends/", "sif/dologger_sif.fbs"])
        .output()
    {
        Ok(result) if result.status.success() && output.exists() => {}
        Ok(result) => eprintln!(
            "flatc unavailable or failed ({}): {}; using committed bindings",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        ),
        Err(error) => eprintln!("flatc not available ({error}); using committed bindings"),
    }
}
