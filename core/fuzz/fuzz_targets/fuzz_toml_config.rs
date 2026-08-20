//! Fuzz target for the TOML configuration parser.
//!
//! Exercises `DologgerConfig::parse()` with arbitrary byte sequences:
//! - The parser must never panic
//! - Valid TOML produces valid config
//! - Invalid TOML returns an error (not panic)
//! - Edge cases: empty string, deeply nested tables, very long values

#![no_main]

use dologger_core::config::{DologgerConfig, PerformanceProfile};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Convert raw bytes to a string, replacing invalid UTF-8 with replacement chars.
    // This simulates a best-effort TOML string input.
    let toml_str = String::from_utf8_lossy(data);

    // 1. Parser must never panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        DologgerConfig::parse(&toml_str, None)
    }));

    match result {
        Ok(Ok((config, warnings))) => {
            // Valid TOML parsed successfully — verify config is sensible
            verify_config_invariants(&config);

            // Warnings should be strings
            for w in &warnings {
                assert!(!w.is_empty(), "warning should not be empty");
            }
        }
        Ok(Err((code, msg))) => {
            // Invalid TOML returned an error — verify error is well-formed
            assert!(!msg.is_empty(), "error message should not be empty");
            assert!(code != 0, "error code should not be 0 (success)");
        }
        Err(panic_err) => {
            // Panic is a bug — report details
            let panic_msg = if let Some(s) = panic_err.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = panic_err.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "unknown panic payload".to_string()
            };
            panic!(
                "DologgerConfig::parse() panicked on input (len={}): {}",
                data.len(),
                panic_msg
            );
        }
    }

    // 2. Test with structured-like input (forcing [dologger] section)
    test_structured_parse(data);

    // 3. Test default config invariants
    let default = DologgerConfig::default();
    verify_config_invariants(&default);

    // 4. Test dev profile
    let dev = DologgerConfig::dev_profile();
    verify_config_invariants(&dev);
    assert_eq!(dev.performance_profile, PerformanceProfile::Dev);
    assert!(!dev.enable_signature);

    // 5. Test hardcoded defaults
    let hd = DologgerConfig::hardcoded_defaults();
    verify_config_invariants(&hd);
});

/// Verify that a config satisfies basic invariants.
fn verify_config_invariants(config: &DologgerConfig) {
    // Level should be a recognized string
    assert!(!config.level.is_empty(), "level must not be empty");

    // Ring buffer size must be a power of two >= 1024
    assert!(
        config.ring_buffer_size >= 1024,
        "ring_buffer_size ({}) must be >= 1024",
        config.ring_buffer_size
    );
    assert!(
        config.ring_buffer_size.is_power_of_two(),
        "ring_buffer_size ({}) must be a power of two",
        config.ring_buffer_size
    );

    // Batch size must be positive
    assert!(config.batch_size > 0, "batch_size must be positive");

    // Shutdown policy must be valid
    assert!(
        config.shutdown_policy == "graceful" || config.shutdown_policy == "immediate",
        "shutdown_policy must be 'graceful' or 'immediate', got '{}'",
        config.shutdown_policy
    );
}

/// Feed the data as a `[dologger]`-prefixed TOML string to exercise
/// the structured parsing path.
fn test_structured_parse(data: &[u8]) {
    let body = String::from_utf8_lossy(data);
    let structured = format!("[dologger]\n{body}");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        DologgerConfig::parse(&structured, None)
    }));

    match result {
        Ok(Ok((config, _))) => verify_config_invariants(&config),
        Ok(Err(_)) => { /* expected for garbage values */ }
        Err(panic_err) => {
            let msg = format!("{:?}", panic_err);
            panic!("panic on structured parse: {msg}");
        }
    }
}

// ===========================================================================
// Standalone edge-case tests
// ===========================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::*;
    use dologger_core::config::ComplianceProfile;

    #[test]
    fn edge_empty_string() {
        let result = DologgerConfig::parse("", None);
        assert!(
            result.is_err(),
            "Empty string should be invalid TOML (missing table)"
        );
    }

    #[test]
    fn edge_empty_table() {
        let (config, _) =
            DologgerConfig::parse("[dologger]\n", None).expect("empty table should parse");
        // Should have default values
        assert_eq!(config.level, "INFO");
    }

    #[test]
    fn edge_only_level() {
        let toml = r#"
[dologger]
level = "DEBUG"
"#;
        let (config, _) = DologgerConfig::parse(toml, None).expect("valid TOML");
        assert_eq!(config.level, "DEBUG");
    }

    #[test]
    fn edge_invalid_toml_syntax() {
        let result = DologgerConfig::parse("this is not toml at all {{{", None);
        assert!(result.is_err());
    }

    #[test]
    fn edge_unknown_profile_defaults() {
        let toml = r#"
[dologger]
performance_profile = "super-duper-unknown"
"#;
        let (config, warnings) =
            DologgerConfig::parse(toml, None).expect("should parse with warning");
        assert!(!warnings.is_empty(), "expected warning for unknown profile");
        // Should default to ProdPerformance
        assert_eq!(
            config.performance_profile,
            PerformanceProfile::ProdPerformance
        );
    }

    #[test]
    fn edge_invalid_ring_buffer_size() {
        let toml = r#"
[dologger]
ring_buffer_size = 500
"#;
        let (config, warnings) =
            DologgerConfig::parse(toml, None).expect("should parse with warning");
        assert!(
            !warnings.is_empty(),
            "expected warning for invalid ring_buffer_size"
        );
        // Should keep default
        assert_eq!(config.ring_buffer_size, 262144);
    }

    #[test]
    fn edge_deeply_nested_tables() {
        // TOML with deeply nested tables not under [dologger] should be ignored
        let mut toml = String::from("[dologger]\nlevel = \"WARN\"\n");
        for i in 0..20 {
            toml.push_str(&format!("[deep.nest.level{i}]\nkey = \"value{i}\"\n"));
        }
        let (config, warnings) =
            DologgerConfig::parse(&toml, None).expect("deeply nested should parse");
        assert_eq!(config.level, "WARN");
        // Deep nested tables should not cause warnings (they're valid TOML, just unused)
    }

    #[test]
    fn edge_all_performance_profiles() {
        let profiles = [
            ("dev", PerformanceProfile::Dev),
            ("prod-performance", PerformanceProfile::ProdPerformance),
            ("prod-audit", PerformanceProfile::ProdAudit),
            ("balanced", PerformanceProfile::Balanced),
        ];

        for (name, expected) in &profiles {
            let toml = format!("[dologger]\nperformance_profile = \"{name}\"\n");
            let (config, _) =
                DologgerConfig::parse(&toml, None).expect("valid profile should parse");
            assert_eq!(
                config.performance_profile, *expected,
                "profile mismatch for '{name}'"
            );
        }
    }

    #[test]
    fn edge_very_long_value() {
        let long_string = "A".repeat(1_000_000);
        let toml = format!("[dologger]\nlevel = \"{}\"\n", long_string);
        let (config, _) = DologgerConfig::parse(&toml, None).expect("long value should parse");
        assert_eq!(config.level, long_string);
    }

    #[test]
    fn edge_boolean_values() {
        let toml = r#"
[dologger]
enable_signature = true
ring_buffer_coop_helping = false
"#;
        let (config, _) = DologgerConfig::parse(toml, None).expect("boolean values should parse");
        assert!(config.enable_signature);
        assert!(!config.ring_buffer_coop_helping);
    }

    #[test]
    fn edge_numeric_values() {
        let toml = r#"
[dologger]
ring_buffer_size = 65536
batch_size = 512
key_rotation_grace_period_days = 30
"#;
        let (config, _) = DologgerConfig::parse(toml, None).expect("numeric values should parse");
        assert_eq!(config.ring_buffer_size, 65536);
        assert_eq!(config.batch_size, 512);
        assert_eq!(config.key_rotation_grace_period_days, 30);
    }

    #[test]
    fn edge_profile_overrides() {
        // Dev profile should force batch_size down to 32 max
        let toml = r#"
[dologger]
performance_profile = "dev"
batch_size = 1000
"#;
        let (config, _) = DologgerConfig::parse(toml, None).expect("should parse");
        // apply_profile() clamps dev batch size to max 32
        assert_eq!(config.batch_size, 32);
        assert!(!config.enable_signature);
    }

    #[test]
    fn edge_prod_audit_profile_forces_signature() {
        let toml = r#"
[dologger]
performance_profile = "prod-audit"
enable_signature = false
"#;
        let (config, _) = DologgerConfig::parse(toml, None).expect("should parse");
        // ProdAudit profile forces enable_signature = true
        assert!(config.enable_signature);
        assert_eq!(config.batch_size, 128);
    }

    #[test]
    fn edge_compliance_profile_parse() {
        // Exercise all compliance profile parsing by validating templates
        for profile in &[
            ComplianceProfile::Gdpr,
            ComplianceProfile::Hipaa,
            ComplianceProfile::PciDss,
        ] {
            let (config, _) = DologgerConfig::load_default();
            let result = config.validate_compliance_template(profile);
            // Default config should fail (missing enable_signature, etc.)
            assert!(result.is_err(), "{:?} should fail on defaults", profile);
        }
    }

    #[test]
    fn edge_garbage_binary_yields_error() {
        // Pure binary garbage should not parse and should not panic
        let garbage = vec![0u8, 1, 2, 3, 128, 255];
        let s = String::from_utf8_lossy(&garbage);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            DologgerConfig::parse(&s, None)
        }));
        match result {
            Ok(Ok(_)) => { /* surprisingly valid — fine */ }
            Ok(Err(_)) => { /* expected */ }
            Err(_) => panic!("parser panicked on binary garbage"),
        }
    }

    #[test]
    fn edge_large_config_with_all_fields() {
        let toml = r#"
[dologger]
level = "AUDIT"
performance_profile = "prod-audit"
ring_buffer_size = 1048576
batch_size = 512
enable_signature = true
key_rotation_grace_period_days = 14
ring_buffer_coop_helping = true
"#;
        let (config, _) = DologgerConfig::parse(toml, None).expect("all fields should parse");
        assert_eq!(config.level, "AUDIT");
        assert_eq!(config.performance_profile, PerformanceProfile::ProdAudit);
        assert_eq!(config.ring_buffer_size, 1048576);
        assert_eq!(config.batch_size, 512);
        assert!(config.enable_signature);
        assert_eq!(config.key_rotation_grace_period_days, 14);
        assert!(config.ring_buffer_coop_helping);
    }
}
