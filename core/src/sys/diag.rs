//! Core diagnostic output — routes operational messages to internal_log.
//!
//! Diagnostic messages go to `dologger_internal.log` via direct syscalls.
//! When sysmon is available, events are duplicated to sysmon channel.
//!
//! This module provides the single diagnostic output path for the core engine.
//! It REPLACES all ad-hoc `eprintln!` calls with structured, auditable output.
//!
//! # Safety
//!
//! All `diag::*` functions are safe to call even before `diag::init()`.
//! Uninitialized calls fall back to stderr (platform-native syscall).

use crate::sys::{DiagLevel, InternalLog};
use std::sync::OnceLock;

/// Global diagnostic logger — initialised once, available everywhere.
static DIAG: OnceLock<InternalLog> = OnceLock::new();

/// Initialise the global diagnostic logger.
/// MUST be called before any diag::* function for file-backed logging.
pub fn init(path: &str) {
    let log = InternalLog::new(path);
    log.info(
        "core",
        &format!("Internal diagnostic log started at '{path}'"),
    );
    let _ = DIAG.set(log);
}

/// Log a diagnostic message. Falls back to stderr if uninitialized.
fn emit(level: DiagLevel, component: &str, message: &str) {
    if let Some(log) = DIAG.get() {
        log.log(level, component, message);
    } else {
        // Fallback: write to stderr before diag::init() is called
        let label = match level {
            DiagLevel::Info => "INFO",
            DiagLevel::Warn => "WARN",
            DiagLevel::Error => "ERROR",
            DiagLevel::Critical => "CRITICAL",
        };
        crate::sys::io::stderr_write(
            format!("[DoLogger] [{label}] [{component}] {message}\n").as_bytes(),
        );
    }
}

/// Log a diagnostic message at INFO level.
pub fn info(component: &str, message: &str) {
    emit(DiagLevel::Info, component, message);
}

/// Log a diagnostic message at WARN level.
pub fn warn(component: &str, message: &str) {
    emit(DiagLevel::Warn, component, message);
}

/// Log a diagnostic message at ERROR level.
pub fn error(component: &str, message: &str) {
    emit(DiagLevel::Error, component, message);
}

/// Log a diagnostic message at CRITICAL level.
pub fn critical(component: &str, message: &str) {
    emit(DiagLevel::Critical, component, message);
}

/// Log a diagnostic message at the given level.
pub fn log(level: DiagLevel, component: &str, message: &str) {
    emit(level, component, message);
}

/// Flush the diagnostic log (no-op if uninitialized).
pub fn flush() {
    if let Some(log) = DIAG.get() {
        log.flush();
    }
}

/// Close the diagnostic log (no-op if uninitialized).
pub fn close() {
    if let Some(log) = DIAG.get() {
        log.close();
    }
}
