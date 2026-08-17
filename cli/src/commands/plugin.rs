//! Plugin management commands for `dologctl`.
//!
//! Complete plugin lifecycle management — list, install, remove,
//! verify, and security-scan plugins in the local and system directories.
//!
//! # Commands
//!
//! | Command | Description |
//! | :- | :- |
//! | `list`  | Scan plugin directories and display loaded plugins with trust colour |
//! | `install` | Copy a plugin file (.so/.dll/.dylib) into the local plugin directory |
//! | `remove`  | Delete a plugin from the local plugin directory by name |
//! | `verify`  | Load a plugin, resolve `plugin_query`, validate ABI, report trust |
//! | `scan`    | Inspect plugin symbol tables for suspicious exports |
//! | `keygen`  | Generate an Ed25519 signing key pair for plugin signatures |
//! | `sign`    | Write an Ed25519 `<library>.sig` sidecar for a plugin library |
//! | `wrap-key`  | Encrypt a signing seed with AES-256-GCM (SSH-style passphrase) |
//! | `unwrap-key` | Decrypt a seed previously wrapped by `wrap-key` |
//! | `totp`      | Show the current TOTP code (or otpauth:// URI) for the 2FA secret |
//!
//! `sign` is an explicit authorization ceremony: the key comes from a seed
//! file, a `--wrapped-key` (prompts for the passphrase), or the
//! `DO_LOG_PLUGIN_SIGNING_KEY` env var — and whenever a base32
//! `DO_LOG_PLUGIN_TOTP_SECRET` is set (or `--require-2fa`), the operator must
//! also enter a live TOTP (RFC 6238) code before the signature is written.
//!
//! `verify` and `list` resolve trust from the committed trust store when
//! `--trust-store <dir>` is given (`active.pub` + `revoked.txt`, multi-anchor
//! with CRL enforcement); otherwise they honour the legacy
//! `DO_LOG_PLUGIN_TRUST_ANCHOR` env var (64 hex chars). Plugins whose `.sig`
//! verifies against any active, non-revoked key are reported as `BLUE`,
//! unsigned ones stay `RED`.

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

/// Read `DO_LOG_PLUGIN_TRUST_ANCHOR` (64 hex chars) into a `[u8; 32]`.
fn trust_anchor_from_env() -> Option<[u8; 32]> {
    let raw = std::env::var("DO_LOG_PLUGIN_TRUST_ANCHOR").ok()?;
    let bytes = dologger_core::hex::decode(raw.trim()).ok()?;
    bytes.try_into().ok()
}

/// Configure a manager's trust from an optional committed trust store or the
/// legacy `DO_LOG_PLUGIN_TRUST_ANCHOR` env var. A store is authoritative and
/// ignores the env var (multi-anchor + CRL).
fn apply_trust(mgr: &mut PluginManager, trust_store: Option<&str>) {
    if let Some(dir) = trust_store {
        if let Err(e) = mgr.load_trust_store(Path::new(dir)) {
            stderr!("Error: Cannot load trust store '{dir}': {e}");
            std::process::exit(1);
        }
    } else if let Some(anchor) = trust_anchor_from_env() {
        mgr.set_trust_anchor(anchor);
    }
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
pub fn cmd_plugin_list(trust_store: Option<&str>) {
    let b = bold();
    let d = dim();
    let reset = output::when_color(color::RESET);

    let mut mgr = PluginManager::new(
        vec![PathBuf::from(PLUGIN_DIR), PathBuf::from(SYSTEM_PLUGIN_DIR)],
        true, // dev_mode — allow unsigned (Red) plugins during listing
    );
    apply_trust(&mut mgr, trust_store);

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
/// Pass `--trust-store <dir>` to verify against the committed trust store.
pub fn cmd_plugin_verify(name: Option<&str>, trust_store: Option<&str>) {
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
    apply_trust(&mut mgr, trust_store);

    for path in &files_to_check {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        stdout!("  Verifying: {b}{stem}{reset}");

        match mgr.load_plugin(path) {
            Ok(plugin_names) => {
                // A bundle library (plugin_query_multi) registers several
                // plugins from one file — verify each one.
                for plugin_name in &plugin_names {
                    if let Some(plugin) = mgr.get(plugin_name) {
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

/// `dologctl plugin keygen --output <path>` — generate an Ed25519 signing
/// key pair for plugin signatures.
///
/// Writes the 64-hex-char seed to `<output>` (0600 perms on POSIX) and
/// prints the derived public key. The public key is the trust anchor
/// (`DO_LOG_PLUGIN_TRUST_ANCHOR`) used to verify plugins at load time.
pub fn cmd_plugin_keygen(path: &str) {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let b = bold();
    let d = dim();
    let reset = output::when_color(color::RESET);

    let signing_key = SigningKey::generate(&mut OsRng);
    let seed_hex = dologger_core::hex::encode(signing_key.to_bytes());
    let pubkey_hex = dologger_core::hex::encode(signing_key.verifying_key().to_bytes());

    if let Err(e) = std::fs::write(path, format!("{seed_hex}\n")) {
        stderr!("Error: Cannot write key file '{path}': {e}");
        std::process::exit(1);
    }

    // Restrict permissions on POSIX; Windows ACLs are the user's concern.
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    stdout!("{b}Ed25519 key pair generated{reset}");
    stdout!("  Seed file: {b}{path}{reset} ({d}64 hex chars, keep secret{reset})");
    stdout!(
        "  Public key (trust anchor): {b}{pubkey_hex}{reset}",
        b = blue()
    );
    stdout!("");
    stdout!("Verify a signed plugin with:");
    stdout!("  {d}DO_LOG_PLUGIN_TRUST_ANCHOR={pubkey_hex} dologctl plugin verify{reset}");
}

/// `dologctl plugin sign <library> [<key>]` — sign a plugin library, writing
/// an Ed25519 `<library>.sig` sidecar that the loader verifies against the
/// configured trust anchors.
///
/// Key sources, in order of precedence:
/// 1. a positional `<key>` seed file (or `--key`), unchanged from before;
/// 2. `--wrapped-key <enc>` — prompts for the AES-256-GCM passphrase to
///    unwrap a key previously protected with `plugin wrap-key`;
/// 3. the `DO_LOG_PLUGIN_SIGNING_KEY` environment variable (CI path).
///
/// Every signature is gated behind a TOTP (RFC 6238) 2FA code whenever
/// `DO_LOG_PLUGIN_TOTP_SECRET` (base32) is set — or when `--require-2fa` is
/// passed — so signing is an explicit, deliberate ceremony. The release
/// workflow signs non-interactively and does not set a TOTP secret.
pub fn cmd_plugin_sign(
    library: &str,
    key: Option<&str>,
    wrapped_key: Option<&str>,
    require_2fa: bool,
) {
    use ed25519_dalek::{Signer, SigningKey};

    let b = bold();
    let r = red();
    let reset = output::when_color(color::RESET);

    // TOTP gate first — fail before touching any key material.
    if require_2fa || std::env::var("DO_LOG_PLUGIN_TOTP_SECRET").is_ok() {
        if let Err(e) = require_2fa_gate() {
            stderr!("{r}Error:{reset} 2FA verification failed: {e}");
            std::process::exit(1);
        }
    }

    // Resolve the 64-hex seed from the key source.
    let seed_hex = if let Some(k) = key {
        match std::fs::read_to_string(k) {
            Ok(raw) => raw.trim().to_string(),
            Err(e) => {
                stderr!("Error: Cannot read key file '{k}': {e}");
                std::process::exit(1);
            }
        }
    } else if let Some(enc) = wrapped_key {
        let passphrase = match read_passphrase() {
            Ok(p) => p,
            Err(e) => {
                stderr!("Error: {e}");
                std::process::exit(1);
            }
        };
        // In-memory unwrap — no plaintext seed ever touches disk.
        let wrapped = match std::fs::read(enc) {
            Ok(bytes) => bytes,
            Err(e) => {
                stderr!("Error: Cannot read wrapped key '{enc}': {e}");
                std::process::exit(1);
            }
        };
        let plaintext = match unwrap_key_bytes(&wrapped, &passphrase) {
            Ok(pt) => pt,
            Err(e) => {
                stderr!("Error: Cannot unwrap key '{enc}': {e}");
                std::process::exit(1);
            }
        };
        String::from_utf8_lossy(&plaintext).trim().to_string()
    } else if let Ok(raw) = std::env::var("DO_LOG_PLUGIN_SIGNING_KEY") {
        raw.trim().to_string()
    } else {
        stderr!("Error: No signing key available.");
        stderr!("  Provide a seed file positionally, `--wrapped-key <enc>` (prompts for the passphrase),");
        stderr!("  or set DO_LOG_PLUGIN_SIGNING_KEY.");
        std::process::exit(1);
    };

    let seed_bytes: [u8; 32] = match dologger_core::hex::decode(&seed_hex) {
        Ok(bytes) => match bytes.try_into() {
            Ok(arr) => arr,
            Err(_) => {
                stderr!("{r}Error:{reset} Key must contain exactly 64 hex chars.");
                std::process::exit(1);
            }
        },
        Err(e) => {
            stderr!("{r}Error:{reset} Key is not valid hex: {e}");
            std::process::exit(1);
        }
    };
    let signing_key = SigningKey::from_bytes(&seed_bytes);

    // Read the library to be signed.
    let bytes = match std::fs::read(library) {
        Ok(bytes) => bytes,
        Err(e) => {
            stderr!("Error: Cannot read library '{library}': {e}");
            std::process::exit(1);
        }
    };

    let signature = signing_key.sign(&bytes);
    let sig_path = format!("{library}.sig");
    if let Err(e) = std::fs::write(&sig_path, signature.to_bytes()) {
        stderr!("Error: Cannot write signature '{sig_path}': {e}");
        std::process::exit(1);
    }

    stdout!(
        "{b}Signed{reset} {library} → {}",
        Path::new(&sig_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&sig_path)
    );
    stdout!(
        "  Public key: {}",
        dologger_core::hex::encode(signing_key.verifying_key().to_bytes())
    );
}

// ---------------------------------------------------------------------------
// TOTP (RFC 6238) 2FA gate
// ---------------------------------------------------------------------------

/// Base32-decode a TOTP secret (RFC 4648, uppercase, spaces ignored).
fn base32_decode(s: &str) -> Result<Vec<u8>, String> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let clean: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let mut bits: u32 = 0;
    let mut nbits: u32 = 0;
    let mut out = Vec::new();
    for c in clean.bytes() {
        let val = alphabet
            .iter()
            .position(|&a| a == c)
            .ok_or_else(|| format!("invalid base32 character '{c}'"))? as u32;
        bits = (bits << 5) | val;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    Ok(out)
}

/// Compute a TOTP code for the given 30-second counter (HMAC-SHA1, dynamic
/// truncation, 6 digits).
fn totp_code(secret: &[u8], counter: u64) -> Result<u32, String> {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    type HmacSha1 = Hmac<Sha1>;
    let mut mac =
        HmacSha1::new_from_slice(secret).map_err(|_| "invalid TOTP secret".to_string())?;
    mac.update(&counter.to_be_bytes());
    let result = mac.finalize().into_bytes();
    let offset = (result[result.len() - 1] & 0x0f) as usize;
    let bin = ((result[offset] & 0x7f) as u32) << 24
        | (result[offset + 1] as u32) << 16
        | (result[offset + 2] as u32) << 8
        | result[offset + 3] as u32;
    Ok(bin % 1_000_000)
}

/// True when `code` matches the current (or one neighbouring) 30-second TOTP
/// window for `secret`.
fn verify_totp(secret: &[u8], code: &str) -> Result<bool, String> {
    let code: u32 = code
        .trim()
        .parse()
        .map_err(|_| "TOTP code must be 6 digits".to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_secs();
    let counter = now / 30;
    for offset in 0..=2u64 {
        if totp_code(secret, counter + offset)? == code {
            return Ok(true);
        }
    }
    // Also accept the previous window (drift/typing lag).
    for offset in 1..=2u64 {
        if totp_code(secret, counter.saturating_sub(offset))? == code {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Prompt for and validate a live TOTP code against `DO_LOG_PLUGIN_TOTP_SECRET`.
fn require_2fa_gate() -> Result<(), String> {
    let b32 = std::env::var("DO_LOG_PLUGIN_TOTP_SECRET")
        .map_err(|_| "DO_LOG_PLUGIN_TOTP_SECRET is not set (base32 TOTP secret)".to_string())?;
    let secret = base32_decode(&b32)?;
    let _ = std::io::Write::write_all(
        &mut std::io::stderr(),
        b"Enter the current TOTP code from your authenticator app: ",
    );
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("Cannot read TOTP code: {e}"))?;
    if verify_totp(&secret, line.trim())? {
        Ok(())
    } else {
        Err("code invalid or expired".into())
    }
}

/// Decrypt a wrapped seed in memory (no temp file), returning the plaintext.
fn unwrap_key_bytes(wrapped: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    if wrapped.len() < KEY_WRAP_MAGIC.len() + KEY_WRAP_NONCE_LEN
        || &wrapped[..KEY_WRAP_MAGIC.len()] != KEY_WRAP_MAGIC
    {
        return Err("not a DOLOGKEY1 wrapped file".into());
    }
    let key = derive_aes_key(passphrase);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "invalid AES-256-GCM key".to_string())?;
    let nonce = Nonce::from_slice(
        &wrapped[KEY_WRAP_MAGIC.len()..KEY_WRAP_MAGIC.len() + KEY_WRAP_NONCE_LEN],
    );
    cipher
        .decrypt(nonce, &wrapped[KEY_WRAP_MAGIC.len() + KEY_WRAP_NONCE_LEN..])
        .map_err(|_| "decryption failed (wrong passphrase or corrupt file)".to_string())
}

/// `dologctl plugin totp [secret]` — print the current TOTP code for the 2FA
/// secret, or with `--uri`, an `otpauth://` provisioning URI for importing
/// into an authenticator app (Aegis, Google Authenticator, …).
pub fn cmd_plugin_totp(secret_arg: Option<&str>, uri: bool) {
    let b = bold();
    let d = dim();
    let reset = output::when_color(color::RESET);

    let b32 = match secret_arg {
        Some(s) => s.to_string(),
        None => match std::env::var("DO_LOG_PLUGIN_TOTP_SECRET") {
            Ok(s) => s,
            Err(_) => {
                stderr!("Error: Provide the base32 secret or set DO_LOG_PLUGIN_TOTP_SECRET.");
                std::process::exit(1);
            }
        },
    };

    if uri {
        stdout!("{b}otpauth://totp/DoLogger:plugin-signing?secret={b32}&issuer=DoLogger{reset}");
        stdout!("{d}Import this into your authenticator app, then keep the same secret in DO_LOG_PLUGIN_TOTP_SECRET.{reset}");
        return;
    }

    let secret = match base32_decode(&b32) {
        Ok(s) => s,
        Err(e) => {
            stderr!("Error: Invalid base32 secret: {e}");
            std::process::exit(1);
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_secs();
    let code = totp_code(&secret, now / 30).unwrap_or(0);
    stdout!("Current TOTP code: {b}{code:06}{reset}");
    stdout!(
        "{d}Valid for ~{} seconds. Use it to authorize a guarded `plugin sign`.{reset}",
        30 - (now % 30)
    );
}

// ---------------------------------------------------------------------------
// Key wrapping (AES-256-GCM, SSH-style local key protection)
// ---------------------------------------------------------------------------

/// Magic header for wrapped key files.
const KEY_WRAP_MAGIC: &[u8; 9] = b"DOLOGKEY1";
/// AES-256-GCM 96-bit nonce length.
const KEY_WRAP_NONCE_LEN: usize = 12;

/// `dologctl plugin wrap-key <key> <output>` — encrypt a signing seed with
/// AES-256-GCM. The passphrase comes from `DO_LOG_PLUGIN_KEY_PASSPHRASE` or
/// an interactive prompt. The result is an SSH-style protected local key file
/// (layout: `DOLOGKEY1` ‖ 12-byte nonce ‖ ciphertext+tag); it can be kept on
/// disk and unwrapped with `plugin unwrap-key` when needed.
pub fn cmd_plugin_wrap_key(key_file: &str, out_file: &str) {
    let b = bold();
    let d = dim();
    let reset = output::when_color(color::RESET);

    let seed = match std::fs::read(key_file) {
        Ok(bytes) => bytes,
        Err(e) => {
            stderr!("Error: Cannot read key file '{key_file}': {e}");
            std::process::exit(1);
        }
    };
    let passphrase = match read_passphrase() {
        Ok(p) => p,
        Err(e) => {
            stderr!("Error: {e}");
            std::process::exit(1);
        }
    };
    match wrap_key_impl(&seed, Path::new(out_file), &passphrase) {
        Ok(()) => {
            stdout!("{b}Wrapped{reset} {key_file} → {out_file} ({d}AES-256-GCM, DOLOGKEY1{reset})");
        }
        Err(e) => {
            stderr!("Error: {e}");
            std::process::exit(1);
        }
    }
}

/// `dologctl plugin unwrap-key <enc> <output>` — decrypt a seed previously
/// wrapped by `wrap-key`, writing the exact original bytes back.
pub fn cmd_plugin_unwrap_key(enc_file: &str, out_file: &str) {
    let b = bold();
    let d = dim();
    let reset = output::when_color(color::RESET);

    let passphrase = match read_passphrase() {
        Ok(p) => p,
        Err(e) => {
            stderr!("Error: {e}");
            std::process::exit(1);
        }
    };
    match unwrap_key_impl(Path::new(enc_file), Path::new(out_file), &passphrase) {
        Ok(()) => {
            stdout!("{b}Unwrapped{reset} {enc_file} → {out_file} ({d}restored seed file{reset})");
        }
        Err(e) => {
            stderr!("Error: {e}");
            std::process::exit(1);
        }
    }
}

/// Read the wrap/unwrap passphrase: `DO_LOG_PLUGIN_KEY_PASSPHRASE` first
/// (for non-interactive / CI use), otherwise prompt on stderr. Prefer the env
/// var on untrusted terminals; the prompt does not disable echo.
fn read_passphrase() -> Result<String, String> {
    if let Ok(p) = std::env::var("DO_LOG_PLUGIN_KEY_PASSPHRASE") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    let _ = std::io::Write::write_all(
        &mut std::io::stderr(),
        b"Enter passphrase for the plugin signing key: ",
    );
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("Cannot read passphrase: {e}"))?;
    let trimmed = line.trim_end().to_string();
    if trimmed.is_empty() {
        Err("empty passphrase".into())
    } else {
        Ok(trimmed)
    }
}

/// Derive the AES-256 key from a passphrase (SHA-256). The raw passphrase is
/// never fed to the cipher directly — its length cannot be trusted.
fn derive_aes_key(passphrase: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(passphrase.as_bytes()).into()
}

/// Encrypt `seed` to `out_path` (see [`KEY_WRAP_MAGIC`] for the layout).
fn wrap_key_impl(seed: &[u8], out_path: &Path, passphrase: &str) -> Result<(), String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    use rand::rngs::OsRng;
    use rand::RngCore;

    let key = derive_aes_key(passphrase);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "invalid AES-256-GCM key".to_string())?;
    let mut nonce_bytes = [0u8; KEY_WRAP_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    // GCM appends a 16-byte authentication tag to the ciphertext.
    let ciphertext = cipher
        .encrypt(nonce, seed)
        .map_err(|_| "AES-256-GCM encryption failed".to_string())?;

    let mut out = Vec::with_capacity(KEY_WRAP_MAGIC.len() + KEY_WRAP_NONCE_LEN + ciphertext.len());
    out.extend_from_slice(KEY_WRAP_MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);

    std::fs::write(out_path, &out)
        .map_err(|e| format!("Cannot write '{}': {e}", out_path.display()))?;
    restrict_perms(out_path);
    Ok(())
}

/// Decrypt a file written by [`wrap_key_impl`] back to `out_path`.
fn unwrap_key_impl(enc_path: &Path, out_path: &Path, passphrase: &str) -> Result<(), String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let data = std::fs::read(enc_path)
        .map_err(|e| format!("Cannot read '{}': {e}", enc_path.display()))?;
    if data.len() < KEY_WRAP_MAGIC.len() + KEY_WRAP_NONCE_LEN
        || &data[..KEY_WRAP_MAGIC.len()] != KEY_WRAP_MAGIC
    {
        return Err(format!(
            "'{}' is not a DOLOGKEY1 wrapped file",
            enc_path.display()
        ));
    }

    let key = derive_aes_key(passphrase);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "invalid AES-256-GCM key".to_string())?;
    let nonce =
        Nonce::from_slice(&data[KEY_WRAP_MAGIC.len()..KEY_WRAP_MAGIC.len() + KEY_WRAP_NONCE_LEN]);
    let plaintext = cipher
        .decrypt(nonce, &data[KEY_WRAP_MAGIC.len() + KEY_WRAP_NONCE_LEN..])
        .map_err(|_| "decryption failed (wrong passphrase or corrupt file)".to_string())?;

    std::fs::write(out_path, &plaintext)
        .map_err(|e| format!("Cannot write '{}': {e}", out_path.display()))?;
    restrict_perms(out_path);
    Ok(())
}

/// Restrict a key file to the owner on POSIX; Windows ACLs are the user's
/// concern (mirrors `cmd_plugin_keygen`).
#[cfg(not(windows))]
fn restrict_perms(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(windows)]
fn restrict_perms(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dologctl-wrap-{}-{name}", std::process::id()))
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let seed_bytes = b"0123456789abcdef0123456789abcdef\n".to_vec();
        let enc_path = temp_path("seed.enc");
        let out_path = temp_path("seed.out");

        wrap_key_impl(&seed_bytes, &enc_path, "test-pass").expect("wrap");
        let enc = std::fs::read(&enc_path).unwrap();
        assert_eq!(
            &enc[..KEY_WRAP_MAGIC.len()],
            KEY_WRAP_MAGIC,
            "magic header present"
        );

        unwrap_key_impl(&enc_path, &out_path, "test-pass").expect("unwrap");
        let out = std::fs::read(&out_path).unwrap();
        assert_eq!(out, seed_bytes, "round-trip preserves the exact seed file");

        let _ = std::fs::remove_file(&enc_path);
        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn unwrap_wrong_passphrase_fails() {
        let seed_bytes = b"deadbeef\n".to_vec();
        let enc_path = temp_path("seed2.enc");
        let out_path = temp_path("seed2.out");

        wrap_key_impl(&seed_bytes, &enc_path, "right").expect("wrap");

        let err = unwrap_key_impl(&enc_path, &out_path, "wrong").expect_err("wrong passphrase");
        assert!(err.contains("decryption failed"), "got: {err}");

        let _ = std::fs::remove_file(&enc_path);
        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn unwrap_rejects_non_dologkey1_file() {
        let enc_path = temp_path("junk.enc");
        let out_path = temp_path("junk.out");
        std::fs::write(&enc_path, b"not a wrapped key").unwrap();

        let err = unwrap_key_impl(&enc_path, &out_path, "pass").expect_err("junk file");
        assert!(err.contains("not a DOLOGKEY1"), "got: {err}");

        let _ = std::fs::remove_file(&enc_path);
        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn totp_rfc6238_sha1_vectors() {
        let secret = b"12345678901234567890".to_vec();
        // RFC 6238 Appendix B, SHA-1 vectors (6-digit truncated form).
        assert_eq!(totp_code(&secret, 1).unwrap(), 287082); // T=59
        assert_eq!(totp_code(&secret, 37037036).unwrap(), 81804); // T=1111111109
        assert_eq!(totp_code(&secret, 37037037).unwrap(), 50471); // T=1111111111
        assert_eq!(totp_code(&secret, 41152263).unwrap(), 5924); // T=1234567890
    }

    #[test]
    fn base32_decode_rfc4648() {
        assert_eq!(base32_decode("MZXW6YTBOI").unwrap(), b"foobar");
        assert_eq!(base32_decode("M Z X W 6 Y T B O I").unwrap(), b"foobar");
        assert!(base32_decode("not-valid-1").is_err());
    }

    #[test]
    fn unwrap_key_bytes_round_trip() {
        let seed = b"0123456789abcdef0123456789abcdef\n".to_vec();
        let enc_path = temp_path("bytes.enc");
        wrap_key_impl(&seed, &enc_path, "pw").expect("wrap");
        let wrapped = std::fs::read(&enc_path).unwrap();
        let out = unwrap_key_bytes(&wrapped, "pw").expect("unwrap");
        assert_eq!(out, seed);
        assert!(unwrap_key_bytes(&wrapped, "wrong").is_err());
        let _ = std::fs::remove_file(&enc_path);
    }
}
