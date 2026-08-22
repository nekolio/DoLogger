//! Build script for libdologger_core.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/sif/codec.rs");
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
