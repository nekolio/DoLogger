//! Centralized output configuration for `dologctl`.
//!
//! Provides output format (text/json), color mode (auto/always/never),
//! quiet mode, and helpers for consistent output across all commands.
//!
//! Uses global atomics so command modules can check state without
//! threading a config struct through every function signature.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// Global output state (set once at startup from CLI flags)
// ---------------------------------------------------------------------------

static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);
static QUIET_MODE: AtomicBool = AtomicBool::new(false);

/// Set the global colour-enabled flag.
pub fn set_color_enabled(yes: bool) {
    COLOR_ENABLED.store(yes, Ordering::Release);
}

/// Set the global quiet-mode flag.
pub fn set_quiet(yes: bool) {
    QUIET_MODE.store(yes, Ordering::Release);
}

/// Return `true` when ANSI colour escapes should be emitted.
pub fn color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Acquire)
}

/// Return `true` when non-error output should be suppressed.
pub fn is_quiet() -> bool {
    QUIET_MODE.load(Ordering::Acquire)
}

/// Return `true` when stdout is a terminal (not piped / redirected).
pub fn stdout_is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

// ---------------------------------------------------------------------------
// Output format (text / json)
// ---------------------------------------------------------------------------

/// Output format for command results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable coloured text (default).
    Text,
    /// Machine-readable JSON.
    Json,
}

// ---------------------------------------------------------------------------
// Colour mode
// ---------------------------------------------------------------------------

/// Controls when ANSI colour escapes are emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorMode {
    /// Detect terminal capability — disable when piped.
    Auto,
    /// Force colour output even when piped.
    Always,
    /// Never emit ANSI escapes.
    Never,
}

// ---------------------------------------------------------------------------
// Output encoding
// ---------------------------------------------------------------------------

/// Console text encoding policy (mirrors `dologger_core::sys::io`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputEncoding {
    /// Dynamic detection: Unicode console API on Windows consoles,
    /// UTF-8 bytes for pipes/files and non-Windows targets.
    Auto,
    /// Always emit UTF-8 bytes (legacy consoles need `chcp 65001`).
    Utf8,
    /// Transcode to the console's active codepage (e.g. GBK/936) on
    /// legacy consoles; redirected output stays UTF-8.
    Native,
}

impl From<OutputEncoding> for dologger_core::sys::io::OutputEncoding {
    fn from(e: OutputEncoding) -> Self {
        match e {
            OutputEncoding::Auto => Self::Auto,
            OutputEncoding::Utf8 => Self::Utf8,
            OutputEncoding::Native => Self::Native,
        }
    }
}

// ---------------------------------------------------------------------------
// OutputConfig — bundles the parsed flags
// ---------------------------------------------------------------------------

/// Snapshot of the output flags parsed from the CLI.
#[derive(Clone, Copy, Debug)]
pub struct OutputConfig {
    pub format: OutputFormat,
    pub color: ColorMode,
    pub quiet: bool,
    pub encoding: OutputEncoding,
    /// Optional manually selected Windows console code page.
    pub code_page: Option<u32>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Text,
            color: ColorMode::Auto,
            quiet: false,
            encoding: OutputEncoding::Auto,
            code_page: None,
        }
    }
}

impl OutputConfig {
    /// Return `true` when colour escapes should be emitted.
    pub fn use_color(&self) -> bool {
        match self.color {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => stdout_is_terminal(),
        }
    }

    /// Return `true` when stdout is **not** a terminal (piped / redirected).
    pub fn is_piped(&self) -> bool {
        !stdout_is_terminal()
    }
}

// ---------------------------------------------------------------------------
// Initialisation — call once after parsing CLI flags
// ---------------------------------------------------------------------------

/// Apply the parsed output config to the global atomics so every
/// command module can query them without threading `OutputConfig`.
pub fn init(config: &OutputConfig) {
    set_color_enabled(config.use_color());
    set_quiet(config.quiet);
    dologger_core::sys::io::set_output_encoding(config.encoding.into());
    if let Some(code_page) = config.code_page {
        if dologger_core::codec::validate_code_page(code_page).is_ok() {
            dologger_core::sys::io::set_output_code_page(Some(code_page));
        } else {
            dologger_core::sys::io::set_output_code_page(None);
        }
    } else {
        dologger_core::sys::io::set_output_code_page(None);
    }
}

// ---------------------------------------------------------------------------
// Convenience writing helpers
// ---------------------------------------------------------------------------

/// Write a line to stdout (respects `--quiet`).
#[inline]
pub fn stdout_line(line: &str) {
    if !is_quiet() {
        dologger_core::sys::io::stdout_line(line);
    }
}

/// Write a line to stderr (never suppressed by `--quiet`).
#[inline]
pub fn stderr_line(line: &str) {
    dologger_core::sys::io::stderr_line(line);
}

// ---------------------------------------------------------------------------
// ANSI colour code constants (raw — callers gate via `color_enabled()`)
// ---------------------------------------------------------------------------

pub mod color {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const BLACK: &str = "\x1b[30m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";
    pub const BRIGHT_BLACK: &str = "\x1b[90m";
    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const BRIGHT_BLUE: &str = "\x1b[94m";
    pub const BRIGHT_MAGENTA: &str = "\x1b[95m";
    pub const BRIGHT_CYAN: &str = "\x1b[96m";
    pub const BRIGHT_WHITE: &str = "\x1b[97m";
}

// ---------------------------------------------------------------------------
// Colour helper — returns the escape (or empty string when colour is off)
// ---------------------------------------------------------------------------

/// Return the given ANSI escape if colour is enabled, otherwise empty.
#[inline]
pub fn when_color(escape: &str) -> &str {
    if color_enabled() {
        escape
    } else {
        ""
    }
}

// ---------------------------------------------------------------------------
// Output macros — shared across all command modules
// ---------------------------------------------------------------------------

/// Print a line to stdout (respects `--quiet`).  Variant with no args
/// prints a blank line.
#[macro_export]
macro_rules! stdout {
    () => {
        $crate::output::stdout_line("")
    };
    ($($arg:tt)*) => {
        $crate::output::stdout_line(&format!($($arg)*))
    };
}

/// Print a line to stderr (never suppressed by `--quiet`).
#[macro_export]
macro_rules! stderr {
    () => {
        $crate::output::stderr_line("")
    };
    ($($arg:tt)*) => {
        $crate::output::stderr_line(&format!($($arg)*))
    };
}
