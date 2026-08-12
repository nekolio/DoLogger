//! Build script for libdologger_core.
//!
//! Responsibilities:
//! - Generate version info, embed git hash
//! - Planned: FlatBuffers code generation (SIF schema), bindgen for plugin headers

fn main() {
    // Embed version info from Cargo
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../.git/HEAD");

    // Emit crate version as env var for compile-time embedding
    let version = env!("CARGO_PKG_VERSION");
    println!("cargo:rustc-env=DOLOGGER_VERSION={version}");

    // Emit target platform info
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=DOLOGGER_TARGET={target}");
}
