//! Plugin management commands for `dologctl`.
//!
//! Complete plugin lifecycle management — list, install, remove,
//! verify, and security-scan plugins in the local and system directories.
//!
//! # Commands
//!
//! | Command | Description |
//! |---------|-------------|
//! | `list`  | Scan plugin directories and display loaded plugins with trust colour |
//! | `install` | Copy a plugin file (.so/.dll/.dylib) into the local plugin directory |
//! | `remove`  | Delete a plugin from the local plugin directory by name |
//! | `verify`  | Load a plugin, resolve `plugin_query`, validate ABI, report trust |
//! | `scan`    | Inspect plugin symbol tables for suspicious exports |

use std::path::{Path, PathBuf};

use dologger_core::plugin::PHASE_NAMES;
use dologger_core::plugin::{PluginManager, TrustLevel};

use crate::output::{self, color};
use crate::{stderr, stdout};

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

fn blue() -> &'static str {
    output::when_color(color::BLUE)
}
fn yellow() -> &'static str {
    output::when_color(color::YELLOW)
}
fn red() -> &'static str {
    output::when_color(color::RED)
}
fn bold() -> &'static str {
    output::when_color(color::BOLD)
}
fn dim() -> &'static str {
    output::when_color(color::DIM)
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Local plugin directory (relative to CWD).
const PLUGIN_DIR: &str = "./plugins";

/// System-wide plugin directory.
const SYSTEM_PLUGIN_DIR: &str = "/usr/lib/dologger/plugins";

/// Valid plugin library extensions.
const VALID_EXTENSIONS: &[&str] = &["so", "dll", "dylib"];

/// Symbols considered suspicious when exported by a plugin.
const SUSPICIOUS_SYMBOLS: &[&str] = &["fork", "exec", "execve", "system", "popen", "dlopen"];

/// Return the ANSI colour escape for a trust level.
fn trust_color(level: TrustLevel) -> &'static str {
    match level {
        TrustLevel::Blue => blue(),
        TrustLevel::Yellow => yellow(),
        TrustLevel::Red => red(),
    }
}

/// Return the human-readable trust-level label.
fn trust_name(level: TrustLevel) -> &'static str {
    match level {
        TrustLevel::Blue => "BLUE",
        TrustLevel::Yellow => "YELLOW",
        TrustLevel::Red => "RED",
    }
}

/// Decode a packed `(major << 16) | (minor << 8) | patch` version word
/// into a human-readable string.
fn format_version(v: u32) -> String {
    let major = (v >> 16) & 0xFF;
    let minor = (v >> 8) & 0xFF;
    let patch = v & 0xFF;
    format!("{major}.{minor}.{patch}")
}

/// Map a phase bitmask to a pipe-delimited list of human-readable phase names.
fn phase_type_name(phase: u32) -> String {
    let names: Vec<&str> = PHASE_NAMES
        .iter()
        .filter_map(|&(name, bit)| if phase & bit != 0 { Some(name) } else { None })
        .collect();
    if names.is_empty() {
        format!("UNKNOWN({:#06x})", phase)
    } else {
        names.join("|")
    }
}

/// Check whether a path has a recognised plugin file extension.
fn is_valid_plugin_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VALID_EXTENSIONS.contains(&e))
        .unwrap_or(false)
}

/// Return an iterator over plugin files in the local plugin directory.
fn local_plugins() -> Vec<PathBuf> {
    let dir = Path::new(PLUGIN_DIR);
    if !dir.is_dir() {
        return Vec::new();
    }
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_valid_plugin_extension(p))
        .collect()
}

// ===========================================================================
// Commands
// ===========================================================================

/// `dologctl plugin list` — scan plugin directories and display loaded
/// plugins with their trust colour, version, type, and path.
pub fn cmd_plugin_list() {
    let b = bold();
    let d = dim();
    let reset = output::when_color(color::RESET);

    let mut mgr = PluginManager::new(
        vec![PathBuf::from(PLUGIN_DIR), PathBuf::from(SYSTEM_PLUGIN_DIR)],
        true, // dev_mode — allow unsigned (Red) plugins during listing
    );

    // Discover and load every recognised plugin file in the search paths.
    let errors = mgr.discover();
    let count = mgr.plugin_count();
    let names = mgr.plugin_names();

    stdout!("{b}Plugin Directory Scan{reset}");
    stdout!("  Search paths:");
    stdout!("    {d}{PLUGIN_DIR}{reset}");
    stdout!("    {d}{SYSTEM_PLUGIN_DIR}{reset}");
    stdout!("");

    if count == 0 {
        stdout!("{d}No plugins found.{reset}");
    } else {
        stdout!("{b}Loaded plugins ({count}):{reset}");
        stdout!("");

        for name in &names {
            if let Some(plugin) = mgr.get(name) {
                let tcolor = trust_color(plugin.trust_level);
                let tname = trust_name(plugin.trust_level);
                let ver = format_version(plugin.info.version);
                let ptype = phase_type_name(plugin.info.phase);

                stdout!(
                    "  {b}{}{reset}  v{ver}  {tcolor}{tname}{reset}  [{ptype}]",
                    plugin.info.name,
                );
                stdout!("    {d}{}{reset}", plugin.library_path.display());
            }
        }
    }

    // Report any plugins that failed to load.
    if !errors.is_empty() {
        stdout!("");
        stderr!("{b}Load failures ({len}):{reset}", len = errors.len());
        for (path, err) in &errors {
            stderr!("  {d}{path}{reset}: {err}");
        }
    }
}

/// `dologctl plugin install <source>` — copy a plugin file into the local
/// directory after validating its extension.
pub fn cmd_plugin_install(source: &str) {
    let source_path = Path::new(source);
    let b = bold();
    let y = yellow();
    let reset = output::when_color(color::RESET);

    // Validate source file exists.
    if !source_path.exists() {
        stderr!("Error: Source file not found: {source}");
        std::process::exit(1);
    }

    // Validate extension.
    if !is_valid_plugin_extension(source_path) {
        stderr!(
            "Error: Invalid plugin extension '{}'. Expected one of: {}",
            source_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("none"),
            VALID_EXTENSIONS.join(", ")
        );
        std::process::exit(1);
    }

    let filename = source_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(source);
    let dest_path = Path::new(PLUGIN_DIR).join(filename);

    // Ensure plugin directory exists.
    if let Err(e) = std::fs::create_dir_all(PLUGIN_DIR) {
        stderr!("Error: Cannot create plugin directory '{PLUGIN_DIR}': {e}");
        std::process::exit(1);
    }

    // Warn if overwriting.
    if dest_path.exists() {
        stdout!(
            "{y}Warning:{reset} Overwriting existing plugin at {}",
            dest_path.display()
        );
    }

    match std::fs::copy(source_path, &dest_path) {
        Ok(bytes) => {
            stdout!("{b}Plugin installed:{reset} {}", dest_path.display());
            stdout!("  Copied {bytes} bytes from {source}");
        }
        Err(e) => {
            stderr!("Error: Failed to copy plugin: {e}");
            std::process::exit(1);
        }
    }
}

/// `dologctl plugin remove <name>` — delete a plugin from the local
/// directory by filename stem (matching any recognised extension).
pub fn cmd_plugin_remove(name: &str) {
    let plugin_dir = Path::new(PLUGIN_DIR);
    let b = bold();
    let reset = output::when_color(color::RESET);

    if !plugin_dir.is_dir() {
        stderr!("Error: Plugin directory '{PLUGIN_DIR}' does not exist.");
        std::process::exit(1);
    }

    // Search for a file whose stem matches `name` with a valid extension.
    let found = local_plugins()
        .into_iter()
        .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(name));

    match found {
        Some(ref path) => match std::fs::remove_file(path) {
            Ok(()) => {
                stdout!(
                    "{b}Plugin removed:{reset} {}",
                    path.file_name().and_then(|f| f.to_str()).unwrap_or(name)
                );
                stdout!("  Removed: {}", path.display());
            }
            Err(e) => {
                stderr!("Error: Cannot remove plugin '{}': {e}", path.display());
                std::process::exit(1);
            }
        },
        None => {
            stderr!("Error: Plugin '{name}' not found in '{PLUGIN_DIR}'.");
            writeln_plugin_hint(name);
            std::process::exit(1);
        }
    }
}

/// Print a hint about what filenames were searched for.
fn writeln_plugin_hint(name: &str) {
    let hints: Vec<String> = VALID_EXTENSIONS
        .iter()
        .map(|ext| format!("{name}.{ext}"))
        .collect();
    stderr!("  Looked for: {}", hints.join(", "));
}

/// `dologctl plugin verify [name]` — load a plugin via `PluginManager`,
/// resolve `plugin_query`, validate ABI version, and report trust level.
/// When `name` is `None`, verify every plugin in the local directory.
pub fn cmd_plugin_verify(name: Option<&str>) {
    let plugin_dir = Path::new(PLUGIN_DIR);
    let b = bold();
    let d = dim();
    let r = red();
    let reset = output::when_color(color::RESET);

    if !plugin_dir.is_dir() {
        stderr!("Error: Plugin directory '{PLUGIN_DIR}' does not exist.");
        std::process::exit(1);
    }

    // Determine which files to verify.
    let files_to_check: Vec<PathBuf> = if let Some(specific_name) = name {
        let found = local_plugins()
            .into_iter()
            .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(specific_name));
        match found {
            Some(path) => vec![path],
            None => {
                stderr!("Error: Plugin '{specific_name}' not found in '{PLUGIN_DIR}'.");
                writeln_plugin_hint(specific_name);
                std::process::exit(1);
            }
        }
    } else {
        let plugins = local_plugins();
        if plugins.is_empty() {
            stdout!("{d}No plugins found to verify.{reset}");
            return;
        }
        plugins
    };

    stdout!("{b}Plugin Verification{reset}");
    stdout!("  Directory: {PLUGIN_DIR}");
    stdout!("");

    let total = files_to_check.len();
    let mut passed: u32 = 0;
    let mut failed: u32 = 0;

    // Use a single PluginManager for all verifications in this batch.
    let mut mgr = PluginManager::new(vec![plugin_dir.to_path_buf()], true);

    for path in &files_to_check {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        stdout!("  Verifying: {b}{stem}{reset}");

        match mgr.load_plugin(path) {
            Ok(plugin_name) => {
                if let Some(plugin) = mgr.get(&plugin_name) {
                    let tcolor = trust_color(plugin.trust_level);
                    let tname = trust_name(plugin.trust_level);
                    let ver = format_version(plugin.info.version);
                    let ptype = phase_type_name(plugin.info.phase);

                    stdout!("    Status:   {tcolor}{b}PASS{reset}");
                    stdout!("    Trust:    {tcolor}{tname}{reset}");
                    stdout!(
                        "    Version:  v{ver} (encoded {enc:#08x})",
                        enc = plugin.info.version
                    );
                    stdout!(
                        "    ABI:      {abi:#06x} (core {core_abi:#06x})",
                        abi = plugin.info.abi_version,
                        core_abi = mgr.abi_version()
                    );
                    stdout!(
                        "    Phase:    [{ptype}] ({phase:#06x})",
                        phase = plugin.info.phase
                    );
                    stdout!("    Path:     {d}{}{reset}", plugin.library_path.display());
                    passed += 1;
                }
            }
            Err(e) => {
                stderr!("    Status:   {r}{b}FAIL{reset}");
                stderr!("    Error:    {e}");
                failed += 1;
            }
        }
        stdout!("");
    }

    // Summary line.
    stdout!("{b}Results:{reset} {passed} passed, {failed} failed (total: {total})");

    if failed > 0 {
        std::process::exit(1);
    }
}

/// `dologctl plugin scan` — scan every plugin in the local directory for
/// suspicious exported symbols (`fork`, `exec`, `system`, `dlopen`).
///
/// Uses `libloading` to load each library and probe the symbol table
/// without invoking any code paths.
pub fn cmd_plugin_scan() {
    let plugin_dir = Path::new(PLUGIN_DIR);
    let b = bold();
    let d = dim();
    let r = red();
    let y = yellow();
    let reset = output::when_color(color::RESET);

    if !plugin_dir.is_dir() {
        stderr!("Error: Plugin directory '{PLUGIN_DIR}' does not exist.");
        std::process::exit(1);
    }

    let files_to_scan = local_plugins();
    if files_to_scan.is_empty() {
        stdout!("{d}No plugins found to scan.{reset}");
        return;
    }

    stdout!("{b}Plugin Security Scan{reset}");
    stdout!("  Directory: {PLUGIN_DIR}");
    stdout!(
        "  Scanning for suspicious symbols: {}",
        SUSPICIOUS_SYMBOLS.join(", ")
    );
    stdout!("");

    let total = files_to_scan.len();
    let mut clean: u32 = 0;
    let mut suspicious: u32 = 0;

    for path in &files_to_scan {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        stdout!("  Scanning: {b}{stem}{reset}");

        match unsafe { libloading::Library::new(path) } {
            Ok(lib) => {
                let mut found_symbols: Vec<&str> = Vec::new();

                for &sym_name in SUSPICIOUS_SYMBOLS {
                    let found = unsafe {
                        lib.get::<*const std::ffi::c_void>(sym_name.as_bytes())
                            .is_ok()
                    };
                    if found {
                        found_symbols.push(sym_name);
                    }
                }

                if found_symbols.is_empty() {
                    stdout!(
                        "    Status:   {blue}CLEAN{reset} — no suspicious symbols found",
                        blue = blue()
                    );
                    clean += 1;
                } else {
                    stdout!(
                        "    Status:   {r}{b}SUSPICIOUS{reset} — {} symbol(s) found",
                        found_symbols.len()
                    );
                    for sym in &found_symbols {
                        stdout!("      {r}[!]{reset} {sym}");
                    }
                    suspicious += 1;
                }
            }
            Err(e) => {
                stderr!("    Status:   {y}LOAD ERROR{reset} — cannot inspect: {e}");
            }
        }
        stdout!("");
    }

    // Final summary.
    stdout!("{b}Scan Results:{reset} {clean} clean, {suspicious} suspicious (total: {total})");

    if suspicious > 0 {
        stdout!("");
        stdout!(
            "{y}{b}Warning:{reset}{y} {suspicious} plugin(s) contain \
             suspicious symbols. Review before use.{reset}",
        );
    }
}
