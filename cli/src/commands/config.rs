//! Configuration validation commands for `dologctl`.
//!
//! Strict validation mode checks compliance templates and
//! non-downgradable items with detailed coloured pass/fail reporting.

use std::path::PathBuf;

use dologger_core::config::{ComplianceProfile, DologgerConfig, PerformanceProfile};

use crate::output::{self, color};
use crate::{stderr, stdout};

// ---------------------------------------------------------------------------
// Colour helpers — resolve once per function call
// ---------------------------------------------------------------------------

fn green() -> &'static str {
    output::when_color(color::GREEN)
}
fn red() -> &'static str {
    output::when_color(color::RED)
}
fn yellow() -> &'static str {
    output::when_color(color::YELLOW)
}
fn cyan() -> &'static str {
    output::when_color(color::CYAN)
}
fn bold() -> &'static str {
    output::when_color(color::BOLD)
}
fn dim() -> &'static str {
    output::when_color(color::DIM)
}

// ---------------------------------------------------------------------------
// Config loading helpers
// ---------------------------------------------------------------------------

/// Load configuration from the priority ladder, matching `validate_config`
/// search rules: explicit path, then `dologger.toml`, then `.dologger.toml`.
fn load_config_for_strict(config_path: Option<&str>) -> DologgerConfig {
    if let Some(path) = config_path {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                stdout!("Configuration file: {path}");
                match DologgerConfig::parse(&content, Some(PathBuf::from(path))) {
                    Ok((config, warnings)) => {
                        for w in &warnings {
                            stderr!(
                                "{YELLOW}Warning:{RESET} {w}",
                                YELLOW = yellow(),
                                RESET = output::when_color(color::RESET)
                            );
                        }
                        config
                    }
                    Err((code, msg)) => {
                        stderr!(
                            "{RED}Error{RESET} (code {code}): {msg}",
                            RED = red(),
                            RESET = output::when_color(color::RESET)
                        );
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                stderr!(
                    "{RED}Error:{RESET} Cannot read config file '{path}': {e}",
                    RED = red(),
                    RESET = output::when_color(color::RESET)
                );
                std::process::exit(1);
            }
        }
    } else {
        let candidates = ["dologger.toml", ".dologger.toml"];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                match std::fs::read_to_string(c) {
                    Ok(content) => {
                        stdout!("Configuration file: {c} (auto-detected)");
                        match DologgerConfig::parse(&content, Some(PathBuf::from(c))) {
                            Ok((config, warnings)) => {
                                for w in &warnings {
                                    stderr!(
                                        "{YELLOW}Warning:{RESET} {w}",
                                        YELLOW = yellow(),
                                        RESET = output::when_color(color::RESET)
                                    );
                                }
                                return config;
                            }
                            Err((code, msg)) => {
                                stderr!(
                                    "{RED}Error{RESET} (code {code}): {msg}",
                                    RED = red(),
                                    RESET = output::when_color(color::RESET)
                                );
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        stderr!(
                            "{RED}Error:{RESET} Cannot read '{c}': {e}",
                            RED = red(),
                            RESET = output::when_color(color::RESET)
                        );
                        std::process::exit(1);
                    }
                }
            }
        }
        stderr!("{YELLOW}Note:{RESET} No configuration file found. Using hardcoded defaults for validation.", YELLOW = yellow(), RESET = output::when_color(color::RESET));
        DologgerConfig::default()
    }
}

// ===========================================================================
// Strict validation
// ===========================================================================

/// Run strict configuration validation with compliance template checks
/// and non-downgradable item verification.
///
/// Returns `true` if all checks pass, `false` if any fail.
pub fn cmd_config_validate_strict(config_path: Option<&str>) -> bool {
    let config = load_config_for_strict(config_path);
    let g = green();
    let r = red();
    let c = cyan();
    let b = bold();
    let d = dim();
    let reset = output::when_color(color::RESET);

    stdout!("");
    stdout!("{b}{c}DoLogger Configuration — Strict Validation{reset}");
    stdout!("{d}──────────────────────────────────────────────{reset}");
    stdout!("");

    let mut all_passed = true;

    // --- Non-downgradable item checks ---
    all_passed &= check_non_downgradable_items(&config);

    stdout!("");

    // --- Compliance template checks ---
    all_passed &= check_compliance_templates(&config);

    stdout!("");

    // --- Config summary ---
    print_config_summary(&config);

    stdout!("");

    // --- Overall verdict ---
    if all_passed {
        stdout!("{b}Overall: {g}PASS{reset} — All strict validation checks passed.");
    } else {
        stdout!("{b}Overall: {r}FAIL{reset} — Some checks failed. Review the report above.");
    }

    all_passed
}

/// Check that all non-downgradable config items are correctly set.
fn check_non_downgradable_items(config: &DologgerConfig) -> bool {
    let g = green();
    let r = red();
    let d = dim();
    let b = bold();
    let reset = output::when_color(color::RESET);

    stdout!("{b}Non-Downgradable Items{reset}");
    stdout!("");

    let mut all_ok = true;

    struct Check {
        name: &'static str,
        current: String,
        expected: &'static str,
        passed: bool,
    }

    let checks = [
        Check {
            name: "enable_signature",
            current: config.enable_signature.to_string(),
            expected: "true",
            passed: config.enable_signature,
        },
        Check {
            name: "performance_profile",
            current: format!("{:?}", config.performance_profile),
            expected: "ProdAudit",
            passed: matches!(config.performance_profile, PerformanceProfile::ProdAudit),
        },
        Check {
            name: "shutdown_policy",
            current: config.shutdown_policy.clone(),
            expected: "\"graceful\"",
            passed: config.shutdown_policy == "graceful",
        },
        Check {
            name: "shutdown_timeout_ms",
            current: format!("{} ms", config.shutdown_timeout_ms),
            expected: ">= 5000 ms",
            passed: config.shutdown_timeout_ms >= 5000,
        },
    ];

    for check in &checks {
        let status = if check.passed {
            format!("{g}PASS{reset}")
        } else {
            all_ok = false;
            format!("{r}FAIL{reset}")
        };
        stdout!(
            "  {status}  {name} = {current}  ({d}expected: {expected}{reset})",
            name = check.name,
            current = check.current,
            expected = check.expected,
        );
    }

    all_ok
}

/// Check all compliance profiles against the config.
fn check_compliance_templates(config: &DologgerConfig) -> bool {
    let g = green();
    let r = red();
    let d = dim();
    let b = bold();
    let reset = output::when_color(color::RESET);

    stdout!("{b}Compliance Templates{reset}");
    stdout!("");

    let profiles = [
        ComplianceProfile::Gdpr,
        ComplianceProfile::Hipaa,
        ComplianceProfile::PciDss,
    ];

    let mut all_ok = true;

    for profile in &profiles {
        let name = profile.display_name();
        match config.validate_compliance_template(profile) {
            Ok(()) => {
                stdout!("  {g}PASS{reset}  {name} — all config-level requirements met");
            }
            Err(gaps) => {
                // Separate config-level violations from domain-level reminders
                let config_gaps: Vec<_> = gaps.iter().filter(|g| !g.contains("REMINDER")).collect();
                let reminders: Vec<_> = gaps.iter().filter(|g| g.contains("REMINDER")).collect();

                if config_gaps.is_empty() {
                    stdout!(
                        "  {g}PASS{reset}  {name} — config-level checks pass \
                         ({reminders_len} domain-level reminder(s))",
                        reminders_len = reminders.len(),
                    );
                    for rem in &reminders {
                        stdout!("    {d}▶{reset} {d}{rem}{reset}");
                    }
                } else {
                    all_ok = false;
                    stdout!(
                        "  {r}FAIL{reset}  {name} — {len} violation(s):",
                        len = config_gaps.len(),
                    );
                    for gap in &config_gaps {
                        stdout!("    {r}▶{reset} {gap}");
                    }
                    // Also show reminders
                    for rem in &reminders {
                        stdout!("    {d}▶{reset} {d}{rem}{reset}");
                    }
                }
            }
        }
    }

    all_ok
}

/// Print a human-readable summary of the loaded configuration.
fn print_config_summary(config: &DologgerConfig) {
    let b = bold();
    let reset = output::when_color(color::RESET);
    stdout!("{b}Configuration Summary{reset}");
    stdout!("  level:                     {}", config.level);
    stdout!(
        "  performance_profile:       {:?}",
        config.performance_profile
    );
    stdout!("  ring_buffer_size:          {}", config.ring_buffer_size);
    stdout!("  batch_size:                {}", config.batch_size);
    stdout!("  enable_signature:          {}", config.enable_signature);
    stdout!("  shutdown_policy:           {}", config.shutdown_policy);
    stdout!(
        "  shutdown_timeout_ms:       {} ms",
        config.shutdown_timeout_ms
    );
    stdout!(
        "  key_rotation_grace_days:   {}",
        config.key_rotation_grace_period_days
    );
    stdout!(
        "  ring_buffer_coop_helping:  {}",
        config.ring_buffer_coop_helping
    );
}

// ---------------------------------------------------------------------------
// Normal (non-strict) validation
// ---------------------------------------------------------------------------

/// Run normal (non-strict) configuration validation.
///
/// This is a separate code path from the inline `validate_config` in
/// `main.rs`, used when `dologctl config validate` is called without
/// `--strict`.  It parses and displays a summary with no compliance
/// checks.
pub fn cmd_config_validate_normal(config_path: Option<&str>) {
    let config = load_config_for_strict(config_path);
    let b = bold();
    let g = green();
    let d = dim();
    let reset = output::when_color(color::RESET);

    stdout!("");
    stdout!("{b}DoLogger Configuration — Validation{reset}");
    stdout!("{d}─────────────────────────────────────{reset}");
    stdout!("");

    print_config_summary(&config);

    stdout!("");
    stdout!("{g}Configuration validation: PASSED{reset}");
}
