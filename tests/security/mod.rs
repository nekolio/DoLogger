//! DoLogger Security Test Suite.
//!
//! This module organizes security tests that validate the correctness
//! of defensive mechanisms in the DoLogger core engine.
//!
//! # Sub-modules
//!
//! | Module | Focus |
//! |--------|-------|
//! | `sandbox_escape` | Plugin sandbox isolation, BPF filter correctness, policy enforcement |
//!
//! # Running
//!
//! These tests live at the workspace root. To compile and run them,
//! copy the relevant test files into `core/tests/` or reference them
//! from a test target in the core crate.
//!
//! ```bash
//! # Run all security tests (from core crate)
//! cargo test -p dologger-core --test security_tests
//!
//! # Run sandbox escape tests specifically
//! cargo test -p dologger-core sandbox_escape
//! ```

pub mod sandbox_escape;
