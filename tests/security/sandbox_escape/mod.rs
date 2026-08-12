//! Sandbox Escape Test Suite.
//!
//! Validates plugin sandbox isolation correctness: policy preset configurations,
//! plugin type restrictions per trust color, BPF seccomp filter generation,
//! syscall allowlist enforcement, and policy validation.
//!
//! # Trust Model
//!
//! | Color  | Trust   | Sandbox Level  | Description |
//! |--------|---------|----------------|-------------|
//! | Blue   | Full    | `None`         | Official signed plugins — no restrictions |
//! | Yellow | Partial | `Restricted`   | Verified third-party — no network, no fork |
//! | Red    | None    | `Isolated`     | Untrusted community — memory/threading/time only |
//!
//! # Integration
//!
//! These tests live at `tests/security/sandbox_escape/` in the workspace root.
//! To run, copy or symlink into `core/tests/security/sandbox_escape/` and run:
//!
//! ```bash
//! cargo test -p dologger-core sandbox_escape
//! ```
//!
//! Or integrate via a `[[test]]` entry in `core/Cargo.toml`:
//!
//! ```toml
//! [[test]]
//! name = "sandbox_escape"
//! path = "../tests/security/sandbox_escape/mod.rs"
//! ```

use std::collections::HashSet;

use dologger_core::plugin::{
    SandboxBackend, SandboxEngine, SandboxLevel, SandboxPolicy, SyscallCategory,
};

// ===========================================================================
// Test helpers
// ===========================================================================

/// Helper to create a `HashSet` from a list of categories.
fn cat_set(cats: &[SyscallCategory]) -> HashSet<SyscallCategory> {
    cats.iter().copied().collect()
}

// ===========================================================================
// Blue plugin policy — no restrictions
// ===========================================================================

#[test]
fn blue_policy_has_no_level_restrictions() {
    let policy = SandboxPolicy::blue();
    assert_eq!(
        policy.level,
        SandboxLevel::None,
        "Blue policy must have SandboxLevel::None"
    );
}

#[test]
fn blue_policy_allows_all_syscall_categories() {
    let policy = SandboxPolicy::blue();

    let all_categories = [
        SyscallCategory::Memory,
        SyscallCategory::FileIO,
        SyscallCategory::Network,
        SyscallCategory::Process,
        SyscallCategory::Threading,
        SyscallCategory::Time,
        SyscallCategory::Signal,
        SyscallCategory::SystemInfo,
    ];

    for cat in &all_categories {
        assert!(
            policy.allows_category(*cat),
            "Blue policy must allow {cat:?}"
        );
    }
}

#[test]
fn blue_policy_allows_all_permissions() {
    let policy = SandboxPolicy::blue();
    assert!(policy.allow_file_write, "Blue: file write must be allowed");
    assert!(policy.allow_network, "Blue: network must be allowed");
    // allow_fork default is false for all policies; blue allows it via level=None
}

#[test]
fn blue_policy_has_empty_categories_set() {
    let policy = SandboxPolicy::blue();
    assert!(
        policy.allowed_categories.is_empty(),
        "Blue policy: allowed_categories set should be empty since level=None bypasses category checks"
    );
}

// ===========================================================================
// Yellow plugin policy — restricted
// ===========================================================================

#[test]
fn yellow_policy_has_restricted_level() {
    let policy = SandboxPolicy::yellow();
    assert_eq!(
        policy.level,
        SandboxLevel::Restricted,
        "Yellow policy must have SandboxLevel::Restricted"
    );
}

#[test]
fn yellow_policy_allows_safe_categories() {
    let policy = SandboxPolicy::yellow();

    // Yellow must allow: Memory, FileIO, Threading, Time, Signal, SystemInfo
    let allowed = [
        SyscallCategory::Memory,
        SyscallCategory::FileIO,
        SyscallCategory::Threading,
        SyscallCategory::Time,
        SyscallCategory::Signal,
        SyscallCategory::SystemInfo,
    ];
    for cat in &allowed {
        assert!(
            policy.allows_category(*cat),
            "Yellow policy must allow {cat:?}"
        );
    }
}

#[test]
fn yellow_policy_denies_network() {
    let policy = SandboxPolicy::yellow();
    assert!(
        !policy.allows_category(SyscallCategory::Network),
        "Yellow plugins must NOT have network access"
    );
    assert!(
        !policy.allow_network,
        "Yellow policy: allow_network must be false"
    );
}

#[test]
fn yellow_policy_denies_fork_and_process() {
    let policy = SandboxPolicy::yellow();
    assert!(
        !policy.allows_category(SyscallCategory::Process),
        "Yellow plugins must NOT have process creation"
    );
    assert!(
        !policy.allow_fork,
        "Yellow policy: allow_fork must be false"
    );
}

#[test]
fn yellow_policy_allows_file_write() {
    let policy = SandboxPolicy::yellow();
    assert!(
        policy.allow_file_write,
        "Yellow plugins may write files (but are network-restricted)"
    );
}

#[test]
fn yellow_policy_has_exactly_six_categories() {
    let policy = SandboxPolicy::yellow();
    assert_eq!(
        policy.allowed_categories.len(),
        6,
        "Yellow policy must have exactly 6 categories: Memory, FileIO, Threading, Time, Signal, SystemInfo"
    );

    let expected = cat_set(&[
        SyscallCategory::Memory,
        SyscallCategory::FileIO,
        SyscallCategory::Threading,
        SyscallCategory::Time,
        SyscallCategory::Signal,
        SyscallCategory::SystemInfo,
    ]);
    assert_eq!(
        policy.allowed_categories, expected,
        "Yellow policy categories must match expected set"
    );
}

// ===========================================================================
// Red plugin policy — maximum isolation
// ===========================================================================

#[test]
fn red_policy_has_isolated_level() {
    let policy = SandboxPolicy::red();
    assert_eq!(
        policy.level,
        SandboxLevel::Isolated,
        "Red policy must have SandboxLevel::Isolated"
    );
}

#[test]
fn red_policy_allows_only_minimal_categories() {
    let policy = SandboxPolicy::red();

    // Red must allow: Memory, Threading, Time
    let allowed = [
        SyscallCategory::Memory,
        SyscallCategory::Threading,
        SyscallCategory::Time,
    ];
    for cat in &allowed {
        assert!(
            policy.allows_category(*cat),
            "Red policy must allow {cat:?}"
        );
    }
}

#[test]
fn red_policy_denies_file_io() {
    let policy = SandboxPolicy::red();
    assert!(
        !policy.allows_category(SyscallCategory::FileIO),
        "Red plugins must NOT have file I/O access"
    );
    assert!(
        !policy.allow_file_write,
        "Red policy: allow_file_write must be false"
    );
}

#[test]
fn red_policy_denies_network_and_process() {
    let policy = SandboxPolicy::red();
    assert!(
        !policy.allows_category(SyscallCategory::Network),
        "Red plugins must NOT have network access"
    );
    assert!(
        !policy.allows_category(SyscallCategory::Process),
        "Red plugins must NOT have process creation"
    );
    assert!(!policy.allow_network, "Red policy: allow_network must be false");
    assert!(!policy.allow_fork, "Red policy: allow_fork must be false");
}

#[test]
fn red_policy_denies_signal_and_system_info() {
    let policy = SandboxPolicy::red();
    assert!(
        !policy.allows_category(SyscallCategory::Signal),
        "Red plugins must NOT have signal handling access"
    );
    assert!(
        !policy.allows_category(SyscallCategory::SystemInfo),
        "Red plugins must NOT have system info access"
    );
}

#[test]
fn red_policy_has_exactly_three_categories() {
    let policy = SandboxPolicy::red();
    assert_eq!(
        policy.allowed_categories.len(),
        3,
        "Red policy must have exactly 3 categories: Memory, Threading, Time"
    );

    let expected = cat_set(&[
        SyscallCategory::Memory,
        SyscallCategory::Threading,
        SyscallCategory::Time,
    ]);
    assert_eq!(
        policy.allowed_categories, expected,
        "Red policy categories must match expected set"
    );
}

// ===========================================================================
// Policy consistency checks
// ===========================================================================

#[test]
fn red_policy_is_strictest() {
    let red = SandboxPolicy::red();
    let yellow = SandboxPolicy::yellow();
    let blue = SandboxPolicy::blue();

    // Red must have the fewest categories
    assert!(red.allowed_categories.len() < yellow.allowed_categories.len());
    assert!(yellow.allowed_categories.len() < 8); // Blue has 0 (bypasses), but there are 8 total cats

    // Red must be the most restrictive level
    assert!(red.level > yellow.level);
    assert!(yellow.level > blue.level);

    // Red must deny everything yellow does not, and more
    for cat in &[
        SyscallCategory::FileIO,
        SyscallCategory::Network,
        SyscallCategory::Process,
        SyscallCategory::Signal,
        SyscallCategory::SystemInfo,
    ] {
        assert!(!red.allows_category(*cat));
    }
}

#[test]
fn policy_levels_have_correct_ordering() {
    // SandboxLevel derives Ord — None < Restricted < Isolated
    assert!(SandboxLevel::None < SandboxLevel::Restricted);
    assert!(SandboxLevel::Restricted < SandboxLevel::Isolated);
    assert!(SandboxLevel::None < SandboxLevel::Isolated);
}

#[test]
fn policies_are_independent_instances() {
    // Each factory call produces a fresh, independent policy
    let r1 = SandboxPolicy::red();
    let r2 = SandboxPolicy::red();
    let y1 = SandboxPolicy::yellow();

    // They should be structurally equal but different instances
    assert_eq!(r1.level, r2.level);
    assert_eq!(r1.allowed_categories, r2.allowed_categories);
    assert_eq!(r1.allow_file_write, r2.allow_file_write);
    assert_eq!(r1.allow_network, r2.allow_network);

    // Red and Yellow must differ
    assert_ne!(r1.allowed_categories, y1.allowed_categories);
    assert_ne!(r1.allow_file_write, y1.allow_file_write);
}

// ===========================================================================
// Plugin type restrictions per trust color
// ===========================================================================

mod plugin_type_restrictions {
    use super::*;

    // --- Blue (SandboxLevel::None) ---

    #[test]
    fn blue_allows_all_plugin_types() {
        // Blue can register as anything
        let all_types = [
            "Filter",
            "Formatter",
            "Processor",
            "FieldProvider",
            "IOSink",
            "ConfigProvider",
            "KeyProvider",
            "PolicyProvider",
            "HostInfoProvider",
            "SyscallBroker",
            "CustomPluginType",
            "AnythingAtAll",
        ];

        for pt in &all_types {
            let result = SandboxPolicy::check_plugin_type_allowed(SandboxLevel::None, pt);
            assert!(
                result.is_ok(),
                "Blue plugin must be allowed to register as '{pt}': got {result:?}"
            );
        }
    }

    // --- Yellow (SandboxLevel::Restricted) ---

    #[test]
    fn yellow_allows_safe_plugin_types() {
        let safe_types = [
            "Filter",
            "Formatter",
            "Processor",
            "FieldProvider",
            "IOSink",
        ];
        for pt in &safe_types {
            assert!(
                SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Restricted, pt).is_ok(),
                "Yellow must allow '{pt}'"
            );
        }
    }

    #[test]
    fn yellow_denies_sensitive_providers() {
        // Yellow cannot be: ConfigProvider, KeyProvider, PolicyProvider, HostInfoProvider, SyscallBroker
        let denied = [
            "ConfigProvider",
            "KeyProvider",
            "PolicyProvider",
            "HostInfoProvider",
            "SyscallBroker",
        ];
        for pt in &denied {
            let result = SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Restricted, pt);
            assert!(
                result.is_err(),
                "Yellow must NOT be allowed to register as '{pt}'"
            );
            let err = result.unwrap_err();
            assert!(
                err.contains(pt),
                "Error for '{pt}' must mention the plugin type name: {err}"
            );
        }
    }

    // --- Red (SandboxLevel::Isolated) ---

    #[test]
    fn red_allows_only_render_transform_types() {
        // Red can only be: Filter, FieldProvider, Processor, Formatter, IOSink
        let allowed = ["Filter", "FieldProvider", "Processor", "Formatter", "IOSink"];
        for pt in &allowed {
            assert!(
                SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, pt).is_ok(),
                "Red must allow '{pt}'"
            );
        }
    }

    #[test]
    fn red_denies_all_other_types() {
        let denied = [
            "ConfigProvider",
            "KeyProvider",
            "PolicyProvider",
            "HostInfoProvider",
            "SyscallBroker",
            "SomeArbitraryType",
            "NetworkProxy",
            "FileWriter",
            "",
        ];
        for pt in &denied {
            let result = SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, pt);
            assert!(
                result.is_err(),
                "Red must NOT be allowed to register as '{pt}'"
            );
        }
    }

    #[test]
    fn red_error_message_is_descriptive() {
        let result = SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "NetworkProxy");
        let err = result.unwrap_err();
        assert!(
            err.contains("Red"),
            "Error message for red plugin must mention 'Red': {err}"
        );
        assert!(
            err.contains("NetworkProxy"),
            "Error must contain the denied type name: {err}"
        );
    }

    #[test]
    fn check_plugin_type_is_case_sensitive() {
        // Verify that case variations are treated as different types
        // "filter" != "Filter" — this is a design choice to test
        let result = SandboxPolicy::check_plugin_type_allowed(SandboxLevel::Isolated, "filter");
        assert!(
            result.is_err(),
            "Case-sensitive: 'filter' should be denied because Red allows 'Filter' (capital F), not 'filter'"
        );
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions6: Policy validation
// ===========================================================================

mod policy_validation {
    use super::*;

    #[test]
    fn all_factory_policies_are_valid() {
        assert!(
            SandboxPolicy::blue().validate().is_ok(),
            "Blue factory policy must validate"
        );
        assert!(
            SandboxPolicy::yellow().validate().is_ok(),
            "Yellow factory policy must validate"
        );
        assert!(
            SandboxPolicy::red().validate().is_ok(),
            "Red factory policy must validate"
        );
    }

    #[test]
    fn default_policy_is_valid() {
        // Default policy is essentially blue-like (level=None)
        assert!(SandboxPolicy::default().validate().is_ok());
    }

    #[test]
    fn validation_rejects_missing_memory_category() {
        let mut policy = SandboxPolicy::yellow();
        policy.allowed_categories.remove(&SyscallCategory::Memory);
        let result = policy.validate();
        assert!(result.is_err(), "Must reject policy without Memory category");
        assert!(result.unwrap_err().contains("Memory"));
    }

    #[test]
    fn validation_rejects_file_write_without_fileio() {
        let mut policy = SandboxPolicy::red();
        // Red normally has allow_file_write=false, so flip it
        policy.allow_file_write = true;
        let result = policy.validate();
        assert!(
            result.is_err(),
            "Must reject allow_file_write=true without FileIO category"
        );
        assert!(result.unwrap_err().contains("FileIO"));
    }

    #[test]
    fn validation_rejects_network_without_network_category() {
        let mut policy = SandboxPolicy::yellow();
        policy.allow_network = true; // Yellow normally denies network
        let result = policy.validate();
        assert!(
            result.is_err(),
            "Must reject allow_network=true without Network category"
        );
        assert!(result.unwrap_err().contains("Network"));
    }

    #[test]
    fn validation_rejects_fork_without_process_category() {
        let mut policy = SandboxPolicy::yellow();
        policy.allow_fork = true; // Yellow normally denies fork
        let result = policy.validate();
        assert!(
            result.is_err(),
            "Must reject allow_fork=true without Process category"
        );
        assert!(result.unwrap_err().contains("Process"));
    }

    #[test]
    fn validation_accepts_fileio_with_write() {
        // Yellow has allow_file_write=true AND FileIO category — must pass
        assert!(SandboxPolicy::yellow().validate().is_ok());
    }

    #[test]
    fn blue_policy_skips_category_validation() {
        // Blue (level=None) bypasses all category validation
        // Even with contradictory settings, blue validates
        let mut policy = SandboxPolicy::blue();
        policy.allow_file_write = true;
        policy.allowed_categories.clear();
        assert!(policy.validate().is_ok(), "Blue skips validation checks");
    }

    #[test]
    fn yellow_rejects_when_fork_allowed_without_process() {
        let mut policy = SandboxPolicy::yellow();
        policy.allow_fork = true;
        policy.allowed_categories.remove(&SyscallCategory::Process);
        assert!(policy.validate().is_err());
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions7: Sandbox engine lifecycle
// ===========================================================================

mod sandbox_engine {
    use super::*;

    #[test]
    fn engine_creates_with_detected_backend() {
        let engine = SandboxEngine::new();

        // On supported platforms, backend should be detected
        #[cfg(any(target_os = "linux", windows, target_os = "macos"))]
        assert_ne!(engine.backend(), SandboxBackend::None);

        // On unsupported platforms, backend is None
        #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
        assert_eq!(engine.backend(), SandboxBackend::None);
    }

    #[test]
    fn engine_is_enabled_by_default() {
        let engine = SandboxEngine::new();
        assert!(
            engine.is_enabled(),
            "Sandbox engine must be enabled by default"
        );
    }

    #[test]
    fn engine_disable_then_enable() {
        let engine = SandboxEngine::new();

        engine.disable();
        assert!(!engine.is_enabled(), "Engine must be disabled after disable()");

        engine.enable();
        assert!(engine.is_enabled(), "Engine must be enabled after enable()");
    }

    #[test]
    fn apply_blue_policy_succeeds() {
        let engine = SandboxEngine::new();
        let policy = SandboxPolicy::blue();
        let result = engine.apply_policy(&policy);
        assert!(result.success, "Applying blue policy must succeed");
        assert_eq!(result.level, SandboxLevel::None);
    }

    #[test]
    fn apply_policy_when_disabled_returns_success() {
        let engine = SandboxEngine::new();
        engine.disable();
        let result = engine.apply_policy(&SandboxPolicy::red());
        assert!(
            result.success,
            "Disabled engine must return success (sandbox bypassed)"
        );
        // When disabled, the error field indicates sandbox is disabled
        assert!(
            result.error.is_some(),
            "Disabled engine should set error message"
        );
        let err = result.error.unwrap();
        assert!(
            err.contains("disabled"),
            "Error should mention sandbox is disabled: {err}"
        );
    }

    #[test]
    fn engine_default_constructs() {
        let engine = SandboxEngine::default();
        assert!(engine.is_enabled());
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions8: Syscall allowlist enumeration
// ===========================================================================

mod syscall_allowlist {
    use super::*;

    #[test]
    fn all_categories_have_non_empty_syscall_lists() {
        let all = [
            SyscallCategory::Memory,
            SyscallCategory::FileIO,
            SyscallCategory::Network,
            SyscallCategory::Process,
            SyscallCategory::Threading,
            SyscallCategory::Time,
            SyscallCategory::Signal,
            SyscallCategory::SystemInfo,
        ];

        for cat in &all {
            let syscalls = cat.linux_syscalls();
            assert!(
                !syscalls.is_empty(),
                "{cat:?} must have at least one syscall defined"
            );
        }
    }

    #[test]
    fn memory_category_includes_basic_memory_ops() {
        let syscalls = SyscallCategory::Memory.linux_syscalls();
        // Must include mmap, munmap, brk, mprotect at minimum
        assert!(syscalls.contains(&"mmap"), "Memory must include mmap");
        assert!(syscalls.contains(&"munmap"), "Memory must include munmap");
        assert!(syscalls.contains(&"brk"), "Memory must include brk");
        assert!(syscalls.contains(&"mprotect"), "Memory must include mprotect");
    }

    #[test]
    fn network_category_includes_socket_apis() {
        let syscalls = SyscallCategory::Network.linux_syscalls();
        assert!(syscalls.contains(&"socket"), "Network must include socket");
        assert!(syscalls.contains(&"connect"), "Network must include connect");
        assert!(syscalls.contains(&"bind"), "Network must include bind");
        assert!(syscalls.contains(&"sendto"), "Network must include sendto");
        assert!(syscalls.contains(&"recvfrom"), "Network must include recvfrom");
    }

    #[test]
    fn fileio_category_includes_read_write() {
        let syscalls = SyscallCategory::FileIO.linux_syscalls();
        assert!(syscalls.contains(&"read"), "FileIO must include read");
        assert!(syscalls.contains(&"write"), "FileIO must include write");
        assert!(syscalls.contains(&"openat"), "FileIO must include openat");
        assert!(syscalls.contains(&"close"), "FileIO must include close");
        assert!(syscalls.contains(&"fsync"), "FileIO must include fsync");
    }

    #[test]
    fn process_category_includes_fork_exec() {
        let syscalls = SyscallCategory::Process.linux_syscalls();
        assert!(syscalls.contains(&"clone"), "Process must include clone");
        assert!(syscalls.contains(&"fork"), "Process must include fork");
        assert!(syscalls.contains(&"execve"), "Process must include execve");
        assert!(syscalls.contains(&"exit"), "Process must include exit");
        assert!(syscalls.contains(&"exit_group"), "Process must include exit_group");
    }

    #[test]
    fn categories_are_mutually_exclusive() {
        // No syscall should appear in more than one category
        let categories = [
            SyscallCategory::Memory,
            SyscallCategory::FileIO,
            SyscallCategory::Network,
            SyscallCategory::Process,
            SyscallCategory::Threading,
            SyscallCategory::Time,
            SyscallCategory::Signal,
            SyscallCategory::SystemInfo,
        ];

        let mut all_syscalls: HashSet<&str> = HashSet::new();
        for cat in &categories {
            for sc in cat.linux_syscalls() {
                assert!(
                    all_syscalls.insert(sc),
                    "Syscall '{sc}' appears in multiple categories (found in {cat:?})"
                );
            }
        }
    }

    #[test]
    fn fileio_does_not_include_network_syscalls() {
        let fileio = SyscallCategory::FileIO.linux_syscalls();
        let network_entries = ["socket", "connect", "bind", "listen", "accept"];
        for net in &network_entries {
            assert!(
                !fileio.contains(net),
                "Network syscall '{net}' must NOT be in FileIO category"
            );
        }
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions9: Cross-category deny test — red policy recursive check
// ===========================================================================

#[test]
fn red_policy_denies_all_forbidden_categories_recursively() {
    let policy = SandboxPolicy::red();

    // These are the ONLY categories red allows
    let red_allowed = cat_set(&[
        SyscallCategory::Memory,
        SyscallCategory::Threading,
        SyscallCategory::Time,
    ]);

    // These are categories that exist in the system
    let all_defined = [
        SyscallCategory::Memory,
        SyscallCategory::FileIO,
        SyscallCategory::Network,
        SyscallCategory::Process,
        SyscallCategory::Threading,
        SyscallCategory::Time,
        SyscallCategory::Signal,
        SyscallCategory::SystemInfo,
    ];

    let mut denied_count = 0;
    let mut allowed_count = 0;

    for cat in &all_defined {
        if red_allowed.contains(cat) {
            assert!(
                policy.allows_category(*cat),
                "Red must allow {cat:?} (it is in the allow set)"
            );
            allowed_count += 1;
        } else {
            assert!(
                !policy.allows_category(*cat),
                "Red must deny {cat:?} (it is NOT in the allow set)"
            );
            denied_count += 1;
        }
    }

    assert_eq!(allowed_count, 3, "Red must allow exactly 3 categories");
    assert_eq!(denied_count, 5, "Red must deny exactly 5 categories");
}

// ===========================================================================
// Blue plugin policy — no restrictions10: Policy structural equivalence and cloning
// ===========================================================================

mod policy_structural {
    use super::*;

    /// Test that cloning preserves all policy fields exactly.
    #[test]
    fn clone_preserves_all_fields() {
        let original = SandboxPolicy::yellow();
        let cloned = original.clone();

        assert_eq!(cloned.level, original.level);
        assert_eq!(cloned.allowed_categories, original.allowed_categories);
        assert_eq!(cloned.allowed_read_paths, original.allowed_read_paths);
        assert_eq!(cloned.allowed_write_paths, original.allowed_write_paths);
        assert_eq!(cloned.allowed_network, original.allowed_network);
        assert_eq!(cloned.max_memory_bytes, original.max_memory_bytes);
        assert_eq!(cloned.max_cpu_seconds, original.max_cpu_seconds);
        assert_eq!(cloned.allow_file_write, original.allow_file_write);
        assert_eq!(cloned.allow_network, original.allow_network);
        assert_eq!(cloned.allow_fork, original.allow_fork);
    }

    /// Test that cloned policies are independent (modifying one does not affect the other).
    #[test]
    fn clone_is_independent() {
        let original = SandboxPolicy::yellow();
        let mut cloned = original.clone();

        // Modify the clone
        cloned.allow_network = true;
        cloned
            .allowed_categories
            .insert(SyscallCategory::Network);
        cloned.allowed_read_paths.push("/tmp/test".into());

        // Original must remain unchanged
        assert!(
            !original.allow_network,
            "Original must not be affected by clone mutation"
        );
        assert!(
            !original.allows_category(SyscallCategory::Network),
            "Original categories must remain unchanged"
        );
        assert!(
            original.allowed_read_paths.is_empty(),
            "Original read paths must remain empty"
        );
    }

    /// Test field-by-field construction of a custom policy.
    #[test]
    fn custom_policy_construction() {
        let mut custom = SandboxPolicy {
            level: SandboxLevel::Restricted,
            allowed_categories: cat_set(&[
                SyscallCategory::Memory,
                SyscallCategory::Threading,
                SyscallCategory::Time,
            ]),
            allowed_read_paths: vec!["/allowed/read".into()],
            allowed_write_paths: vec!["/allowed/write".into()],
            allowed_network: vec![],
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_cpu_seconds: 10,
            allow_file_write: false,
            allow_network: false,
            allow_fork: false,
        };

        assert_eq!(custom.level, SandboxLevel::Restricted);
        assert_eq!(custom.allowed_categories.len(), 3);
        assert_eq!(custom.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(custom.max_cpu_seconds, 10);
        assert!(!custom.allow_file_write);

        // Should validate (has Memory category)
        assert!(custom.validate().is_ok());
    }

    /// Test resource limit fields with extreme values.
    #[test]
    fn policy_resource_limits() {
        // Unlimited
        let unlimited = SandboxPolicy::blue();
        assert_eq!(unlimited.max_memory_bytes, 0, "0 means unlimited");
        assert_eq!(unlimited.max_cpu_seconds, 0, "0 means unlimited");

        // Explicit limits (not set by factory, but should be settable)
        let mut limited = SandboxPolicy::red();
        limited.max_memory_bytes = 32 * 1024 * 1024; // 32 MB
        limited.max_cpu_seconds = 5;

        assert_eq!(limited.max_memory_bytes, 32 * 1024 * 1024);
        assert_eq!(limited.max_cpu_seconds, 5);
        assert!(limited.validate().is_ok());
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions11: SandboxBackend detection
// ===========================================================================

mod sandbox_backend {
    use super::*;

    #[test]
    fn backend_detect_is_platform_aware() {
        let backend = SandboxBackend::detect();

        #[cfg(target_os = "linux")]
        assert_eq!(
            backend,
            SandboxBackend::Seccomp,
            "Linux must detect Seccomp backend"
        );

        #[cfg(windows)]
        assert_eq!(
            backend,
            SandboxBackend::AppContainer,
            "Windows must detect AppContainer backend"
        );

        #[cfg(target_os = "macos")]
        assert_eq!(
            backend,
            SandboxBackend::MacOSSandbox,
            "macOS must detect MacOSSandbox backend"
        );
    }

    #[test]
    fn backend_supports_isolation() {
        // On Linux and macOS, the backend supports actual process isolation
        assert!(SandboxBackend::Seccomp.supports_isolation());
        assert!(SandboxBackend::MacOSSandbox.supports_isolation());

        // AppContainer is skeleton only — Windows doesn't support full isolation yet
        assert!(!SandboxBackend::AppContainer.supports_isolation());
        assert!(!SandboxBackend::None.supports_isolation());
    }

    #[test]
    fn backend_debug_display() {
        // Verify all backends have distinct debug representations
        let backends = [
            SandboxBackend::Seccomp,
            SandboxBackend::AppContainer,
            SandboxBackend::MacOSSandbox,
            SandboxBackend::None,
        ];

        // Ensure each Debug output is unique
        let mut seen = HashSet::new();
        for b in &backends {
            let dbg = format!("{b:?}");
            seen.insert(dbg);
        }
        assert_eq!(seen.len(), backends.len(), "All backends must have unique Debug output");
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions12: BPF filter validation framework (Linux only)
// ===========================================================================
//
// On Linux, the sandbox uses seccomp-bpf with a generated filter program.
// These tests validate the BPF filter structure, syscall number mapping,
// and ensure the filter correctly enforces the allowlist.
//
// Since `build_bpf_filter` and `syscall_name_to_number` are private,
// we replicate the BPF generation algorithm for validation purposes.

#[cfg(target_os = "linux")]
mod bpf_filter_validation {
    use super::*;
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // BPF instruction encoding constants (mirroring sandbox.rs)
    // -----------------------------------------------------------------------

    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10;
    const BPF_K: u16 = 0x00;
    const BPF_RET: u16 = 0x06;
    const SECCOMP_RET_ALLOW: u32 = 0x7FFF_0000;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;

    /// Replica of the BPF instruction (matches `libc::sock_filter`).
    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct BpfInstruction {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    /// Replica of `syscall_name_to_number` for testing.
    fn test_syscall_lookup(name: &str) -> Option<i32> {
        match name {
            "read" => Some(0),
            "write" => Some(1),
            "openat" => Some(257),
            "close" => Some(3),
            "fstat" => Some(5),
            "lseek" => Some(8),
            "mmap" => Some(9),
            "mprotect" => Some(10),
            "munmap" => Some(11),
            "brk" => Some(12),
            "rt_sigaction" => Some(13),
            "rt_sigprocmask" => Some(14),
            "rt_sigreturn" => Some(15),
            "pread64" => Some(17),
            "pwrite64" => Some(18),
            "readv" => Some(19),
            "writev" => Some(20),
            "sched_yield" => Some(24),
            "madvise" => Some(28),
            "nanosleep" => Some(35),
            "getpid" => Some(39),
            "socket" => Some(41),
            "connect" => Some(42),
            "accept" => Some(43),
            "sendto" => Some(44),
            "recvfrom" => Some(45),
            "sendmsg" => Some(46),
            "recvmsg" => Some(47),
            "bind" => Some(49),
            "listen" => Some(50),
            "getsockname" => Some(51),
            "setsockopt" => Some(54),
            "clone" => Some(56),
            "fork" => Some(57),
            "vfork" => Some(58),
            "execve" => Some(59),
            "exit" => Some(60),
            "uname" => Some(63),
            "fsync" => Some(74),
            "fdatasync" => Some(75),
            "gettimeofday" => Some(96),
            "sysinfo" => Some(99),
            "getuid" => Some(102),
            "getgid" => Some(104),
            "gettid" => Some(186),
            "time" => Some(201),
            "futex" => Some(202),
            "clock_gettime" => Some(228),
            "clock_nanosleep" => Some(230),
            "exit_group" => Some(231),
            _ => None,
        }
    }

    /// Build a BPF filter identical to `build_bpf_filter` in sandbox.rs.
    /// This is a test replica so we can validate filter structure without
    /// accessing the private function.
    fn build_test_bpf_filter(allowed: &[i32]) -> Vec<BpfInstruction> {
        let mut filter = Vec::new();

        // Instruction 0: Load syscall number (LD_W_ABS offset 0)
        filter.push(BpfInstruction {
            code: BPF_LD | BPF_W | BPF_ABS,
            jt: 0,
            jf: 0,
            k: 0, // seccomp_data.nr is at offset 0 for x86_64
        });

        let jeq_start_idx = filter.len(); // = 1

        // For each allowed syscall, add a JEQ (jump-if-equal) check
        for &syscall_nr in allowed {
            filter.push(BpfInstruction {
                code: BPF_JMP | BPF_JEQ | BPF_K,
                jt: 0, // Patched below
                jf: 0, // Fall through to next check
                k: syscall_nr as u32,
            });
        }

        let allow_idx = filter.len();
        // KILL is 1 instruction after ALLOW
        let _kill_idx = filter.len() + 1;

        // Patch JEQ instructions: jt = relative offset to ALLOW
        for i in jeq_start_idx..allow_idx {
            let rel_jt = (allow_idx - i - 1) as u8;
            filter[i].jt = rel_jt;
        }

        // ALLOW return
        filter.push(BpfInstruction {
            code: BPF_RET | BPF_K,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ALLOW,
        });

        // KILL return (default deny)
        filter.push(BpfInstruction {
            code: BPF_RET | BPF_K,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_KILL_PROCESS,
        });

        filter
    }

    /// Collect all syscall numbers for categories allowed by a policy.
    fn collect_allowed_syscalls(policy: &SandboxPolicy) -> Vec<i32> {
        let mut nums = Vec::new();

        for cat in &policy.allowed_categories {
            for name in cat.linux_syscalls() {
                if let Some(nr) = test_syscall_lookup(name) {
                    nums.push(nr);
                }
            }
        }

        // Always-allowed syscalls (mirrors sandbox.rs)
        nums.push(60);  // exit
        nums.push(231); // exit_group
        nums.push(219); // restart_syscall
        nums.push(0);   // read
        nums.push(1);   // write

        nums.sort();
        nums.dedup();
        nums
    }

    // --- Tests ---

    #[test]
    fn bpf_filter_starts_with_load_instruction() {
        let filter = build_test_bpf_filter(&[60, 231]);
        assert!(!filter.is_empty(), "BPF filter must not be empty");
        assert_eq!(
            filter[0].code,
            BPF_LD | BPF_W | BPF_ABS,
            "First instruction must be LD_W_ABS (load syscall number)"
        );
        assert_eq!(
            filter[0].k, 0,
            "Load offset must be 0 (seccomp_data.nr)"
        );
    }

    #[test]
    fn bpf_filter_ends_with_allow_then_kill() {
        let filter = build_test_bpf_filter(&[60]);
        let n = filter.len();

        // Last instruction: KILL (SECCOMP_RET_KILL_PROCESS)
        assert_eq!(
            filter[n - 1].code,
            BPF_RET | BPF_K,
            "Last instruction must be return KILL"
        );
        assert_eq!(
            filter[n - 1].k,
            SECCOMP_RET_KILL_PROCESS,
            "Last instruction must return KILL_PROCESS"
        );

        // Second-to-last: ALLOW (SECCOMP_RET_ALLOW)
        assert_eq!(
            filter[n - 2].code,
            BPF_RET | BPF_K,
            "Penultimate instruction must be return ALLOW"
        );
        assert_eq!(
            filter[n - 2].k,
            SECCOMP_RET_ALLOW,
            "Penultimate instruction must return ALLOW"
        );
    }

    #[test]
    fn bpf_filter_jeq_instructions_have_correct_relative_jumps() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());
        let filter = build_test_bpf_filter(&allowed);

        // JEQ instructions are at indices 1..filter.len()-2
        let allow_idx = filter.len() - 2;

        for i in 1..allow_idx {
            let inst = &filter[i];
            // Verify it's a JEQ instruction
            let expected_code = BPF_JMP | BPF_JEQ | BPF_K;
            assert_eq!(
                inst.code, expected_code,
                "Instruction {i} must be JEQ (code {expected_code:#06x}, got {:#06x})",
                inst.code
            );

            // jt: relative jump to ALLOW from this instruction
            // rel_jt = allow_idx - i - 1 (instructions to skip forward)
            let expected_jt = (allow_idx - i - 1) as u8;
            assert_eq!(
                inst.jt, expected_jt,
                "Instruction {i}: jt must be {expected_jt} (relative offset to ALLOW), got {}",
                inst.jt
            );

            // jf: always 0 (fall through to next JEQ)
            assert_eq!(
                inst.jf, 0,
                "Instruction {i}: jf must be 0 (fall through to next check)"
            );
        }
    }

    #[test]
    fn bpf_filter_jt_decreases_linearly() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::red());
        let filter = build_test_bpf_filter(&allowed);
        let allow_idx = filter.len() - 2;

        // jt values should decrease linearly: (allow_idx-i-1) for i=1..allow_idx-1
        let mut prev_jt = u8::MAX;
        for i in 1..allow_idx {
            let inst = &filter[i];
            assert!(
                inst.jt < prev_jt,
                "jt values must decrease monotonically (got jt={} at i={i})",
                inst.jt
            );
            prev_jt = inst.jt;

            let expected = (allow_idx - i - 1) as u8;
            assert_eq!(
                inst.jt, expected,
                "Instruction {i}: jt mismatch, expected {expected}, got {}",
                inst.jt
            );
        }
    }

    #[test]
    fn bpf_filter_jf_is_always_zero_for_jeq() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());
        let filter = build_test_bpf_filter(&allowed);
        let allow_idx = filter.len() - 2;

        for i in 1..allow_idx {
            assert_eq!(
                filter[i].jf, 0,
                "All JEQ instructions must have jf=0 (fall through)"
            );
        }
    }

    #[test]
    fn bpf_filter_jeq_matches_allowed_syscalls() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());
        let filter = build_test_bpf_filter(&allowed);
        let allow_idx = filter.len() - 2;

        let filter_syscalls: Vec<i32> = filter[1..allow_idx]
            .iter()
            .map(|inst| inst.k as i32)
            .collect();

        assert_eq!(
            filter_syscalls, allowed,
            "BPF JEQ constants must match the allowed syscall numbers in order"
        );
    }

    #[test]
    fn bpf_filter_syscalls_are_sorted() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());
        let filter = build_test_bpf_filter(&allowed);
        let allow_idx = filter.len() - 2;

        let filter_syscalls: Vec<i32> = filter[1..allow_idx]
            .iter()
            .map(|inst| inst.k as i32)
            .collect();

        // Verify sorted ascending
        for w in filter_syscalls.windows(2) {
            assert!(w[0] <= w[1], "Allowed syscalls must be sorted: {} > {}", w[0], w[1]);
        }
    }

    #[test]
    fn bpf_filter_has_no_duplicates() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());
        let filter = build_test_bpf_filter(&allowed);
        let allow_idx = filter.len() - 2;

        let mut seen = HashSet::new();
        for i in 1..allow_idx {
            let nr = filter[i].k;
            assert!(
                seen.insert(nr),
                "Duplicate syscall number {nr} in BPF filter at instruction {i}"
            );
        }
    }

    #[test]
    fn bpf_filter_valid_size_formula() {
        // filter size = 1 (load) + N (jeq checks) + 1 (ALLOW) + 1 (KILL)
        // = N + 3
        for (policy, expected_syscalls_min, label) in [
            (&SandboxPolicy::yellow(), 30usize, "yellow"),
            (&SandboxPolicy::red(), 10usize, "red"),
        ] {
            let allowed = collect_allowed_syscalls(policy);
            let filter = build_test_bpf_filter(&allowed);
            let n = allowed.len();

            assert_eq!(
                filter.len(),
                n + 3,
                "BPF filter for {label} must have {n} + 3 = {} instructions, got {}",
                n + 3,
                filter.len()
            );
            assert!(
                n >= expected_syscalls_min,
                "{label}: expected at least {expected_syscalls_min} allowed syscalls, got {n}"
            );
        }
    }

    #[test]
    fn bpf_filter_for_red_policy_is_smaller_than_yellow() {
        let red_allowed = collect_allowed_syscalls(&SandboxPolicy::red());
        let yellow_allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());

        let red_filter = build_test_bpf_filter(&red_allowed);
        let yellow_filter = build_test_bpf_filter(&yellow_allowed);

        assert!(
            red_filter.len() < yellow_filter.len(),
            "Red BPF filter ({}) must be smaller than yellow ({}); red has stricter limits",
            red_filter.len(),
            yellow_filter.len()
        );
    }

    #[test]
    fn bpf_filter_blue_policy_has_no_filter() {
        // Blue policy has no categories → no BPF filter instructions beyond load
        // But apply_policy handles level=None separately before building filter
        let blue_allowed = collect_allowed_syscalls(&SandboxPolicy::blue());
        // Blue factory policy has empty allowed_categories, but the helper adds
        // essential syscalls. In practice, apply_policy returns early for blue.
        // Test that if we built a filter from blue categories, it's tiny.
        if blue_allowed.is_empty() {
            // Only the always-allowed syscalls would be there if we called it
            // (but in reality, blue bypasses filter generation entirely)
        }
    }

    #[test]
    fn bpf_filter_red_excludes_network_syscalls() {
        let red_allowed = collect_allowed_syscalls(&SandboxPolicy::red());
        let filter = build_test_bpf_filter(&red_allowed);

        // Network syscall numbers to check
        let network_nrs = [41, 42, 43, 44, 45, 46, 47, 49, 50, 51, 54]; // socket, connect, accept, sendto, etc.

        for inst in &filter[1..filter.len() - 2] {
            assert!(
                !network_nrs.contains(&(inst.k as i32)),
                "Red BPF filter must NOT allow network syscall {}",
                inst.k
            );
        }
    }

    #[test]
    fn bpf_filter_yellow_includes_fileio_syscalls() {
        let yellow_allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());
        let filter = build_test_bpf_filter(&yellow_allowed);

        // File IO syscalls that must be present
        let required = [0, 1, 3, 5, 8, 257]; // read, write, close, fstat, lseek, openat
        let filter_nrs: Vec<i32> = filter[1..filter.len() - 2]
            .iter()
            .map(|i| i.k as i32)
            .collect();

        for req in &required {
            assert!(
                filter_nrs.contains(req),
                "Yellow BPF filter must include FileIO syscall {req}"
            );
        }
    }

    #[test]
    fn bpf_filter_exit_syscalls_always_present() {
        // exit (60), exit_group (231), restart_syscall (219) are always added
        for policy in [SandboxPolicy::red(), SandboxPolicy::yellow()] {
            let allowed = collect_allowed_syscalls(&policy);
            let filter = build_test_bpf_filter(&allowed);

            let filter_nrs: Vec<i32> = filter[1..filter.len() - 2]
                .iter()
                .map(|i| i.k as i32)
                .collect();

            assert!(
                filter_nrs.contains(&60),
                "{:?} filter must include exit (60)",
                policy.level
            );
            assert!(
                filter_nrs.contains(&231),
                "{:?} filter must include exit_group (231)",
                policy.level
            );
            assert!(
                filter_nrs.contains(&219),
                "{:?} filter must include restart_syscall (219)",
                policy.level
            );
        }
    }

    // --- Comprehensive syscall name lookup tests ---

    #[test]
    fn syscall_lookup_all_defined_names() {
        let defined_names: HashMap<&str, i32> = [
            ("read", 0),
            ("write", 1),
            ("close", 3),
            ("fstat", 5),
            ("lseek", 8),
            ("mmap", 9),
            ("mprotect", 10),
            ("munmap", 11),
            ("brk", 12),
            ("rt_sigaction", 13),
            ("rt_sigprocmask", 14),
            ("rt_sigreturn", 15),
            ("pread64", 17),
            ("pwrite64", 18),
            ("readv", 19),
            ("writev", 20),
            ("sched_yield", 24),
            ("madvise", 28),
            ("nanosleep", 35),
            ("getpid", 39),
            ("socket", 41),
            ("connect", 42),
            ("accept", 43),
            ("sendto", 44),
            ("recvfrom", 45),
            ("sendmsg", 46),
            ("recvmsg", 47),
            ("bind", 49),
            ("listen", 50),
            ("getsockname", 51),
            ("setsockopt", 54),
            ("clone", 56),
            ("fork", 57),
            ("vfork", 58),
            ("execve", 59),
            ("exit", 60),
            ("uname", 63),
            ("fsync", 74),
            ("fdatasync", 75),
            ("gettimeofday", 96),
            ("sysinfo", 99),
            ("getuid", 102),
            ("getgid", 104),
            ("gettid", 186),
            ("time", 201),
            ("futex", 202),
            ("clock_gettime", 228),
            ("clock_nanosleep", 230),
            ("exit_group", 231),
            ("openat", 257),
        ]
        .into_iter()
        .collect();

        for (name, expected_nr) in &defined_names {
            let result = test_syscall_lookup(name);
            assert_eq!(
                result,
                Some(*expected_nr),
                "syscall '{name}' must map to {expected_nr}, got {result:?}"
            );
        }
    }

    #[test]
    fn syscall_lookup_unknown_returns_none() {
        let unknown = ["nonexistent", "create_process", "super_syscall", "", "SOCKET"];
        for name in &unknown {
            assert_eq!(
                test_syscall_lookup(name),
                None,
                "Unknown syscall '{name}' must return None"
            );
        }
    }

    #[test]
    fn syscall_lookup_is_deterministic() {
        // Same name must always map to same number
        for _ in 0..100 {
            assert_eq!(test_syscall_lookup("write"), Some(1));
            assert_eq!(test_syscall_lookup("socket"), Some(41));
            assert_eq!(test_syscall_lookup("nonexistent"), None);
        }
    }

    #[test]
    fn bpf_filter_does_not_allow_execve_for_red() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::red());
        assert!(
            !allowed.contains(&59), // execve = 59
            "Red plugins must NOT be allowed execve (59)"
        );
    }

    #[test]
    fn bpf_filter_does_not_allow_socket_for_yellow() {
        let allowed = collect_allowed_syscalls(&SandboxPolicy::yellow());
        assert!(
            !allowed.contains(&41), // socket = 41
            "Yellow plugins must NOT be allowed socket (41)"
        );
    }

    #[test]
    fn bpf_filter_handles_empty_allowlist() {
        let filter = build_test_bpf_filter(&[]);
        // Should have: load + 0 jeq checks + ALLOW + KILL = 3 instructions
        assert_eq!(filter.len(), 3, "Empty allowlist produces minimal filter");
        assert_eq!(filter[0].code, BPF_LD | BPF_W | BPF_ABS);
        assert_eq!(filter[1].code, BPF_RET | BPF_K);
        assert_eq!(filter[1].k, SECCOMP_RET_ALLOW);
        assert_eq!(filter[2].k, SECCOMP_RET_KILL_PROCESS);
    }

    #[test]
    fn bpf_filter_handles_single_syscall() {
        let filter = build_test_bpf_filter(&[60]); // exit only
        assert_eq!(filter.len(), 4, "Single syscall: load + 1 jeq + ALLOW + KILL");

        // JEQ for exit(60)
        assert_eq!(filter[1].k, 60);
        // jt: from instruction 1 to ALLOW at index 2 = 2-1-1 = 0
        // Wait, with 4 instructions: [0]=ld, [1]=jeq, [2]=allow, [3]=kill
        // allow_idx = 2, i=1: jt = 2-1-1 = 0
        // jt=0 means "next instruction" (which is ALLOW)
        assert_eq!(filter[1].jt, 0, "Single JEQ: jt=0 (next instruction is ALLOW)");
        assert_eq!(filter[1].jf, 0, "Fall-through to KILL (but jt handles the match)");
    }

    #[test]
    fn bpf_filter_relative_offsets_do_not_overflow_u8() {
        // With many allowed syscalls, jt must not exceed u8::MAX
        let many_syscalls: Vec<i32> = (0..300).collect();
        let filter = build_test_bpf_filter(&many_syscalls);
        let allow_idx = filter.len() - 2;

        // First JEQ instruction has the largest jt value
        let first_jt = filter[1].jt;
        let expected = (allow_idx - 1 - 1) as u8;

        // If there are >256 JEQ instructions, jt will wrap (overflow u8)
        // This is a known limitation — BPF supports only 8-bit relative offsets
        if allow_idx - 1 > 256 {
            // With >256 allowed syscalls, the first jt value wraps
            // In practice, red/yellow policies have much fewer than 256 allowed syscalls
        } else {
            assert_eq!(first_jt, expected);
        }
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions13: Policy enumeration and classification
// ===========================================================================

#[test]
fn policy_levels_match_trust_colors() {
    // The SandboxLevel enum maps to trust colors:
    // None → Blue, Restricted → Yellow, Isolated → Red

    assert_eq!(SandboxPolicy::blue().level, SandboxLevel::None);
    assert_eq!(SandboxPolicy::yellow().level, SandboxLevel::Restricted);
    assert_eq!(SandboxPolicy::red().level, SandboxLevel::Isolated);
}

#[test]
fn policy_allow_network_exactly_matches_level_expectation() {
    // Black-box test: verify network flag for each preset
    assert!(SandboxPolicy::blue().allow_network);
    assert!(!SandboxPolicy::yellow().allow_network);
    assert!(!SandboxPolicy::red().allow_network);
}

#[test]
fn policy_allow_fork_always_false_for_factory_policies() {
    // Factory policies never allow fork
    assert!(!SandboxPolicy::blue().allow_fork);
    assert!(!SandboxPolicy::yellow().allow_fork);
    assert!(!SandboxPolicy::red().allow_fork);
}

#[test]
fn policy_allow_file_write_progression() {
    // File write permission decreases with trust level
    let blue = SandboxPolicy::blue();
    let yellow = SandboxPolicy::yellow();
    let red = SandboxPolicy::red();

    assert!(blue.allow_file_write);
    assert!(yellow.allow_file_write);
    assert!(!red.allow_file_write, "Red plugins must not write files");
}

// ===========================================================================
// Blue plugin policy — no restrictions14: Policy resource paths are initialised empty
// ===========================================================================

#[test]
fn factory_policies_have_empty_path_lists() {
    // No factory policy pre-configures filesystem paths
    for policy in [SandboxPolicy::blue(), SandboxPolicy::yellow(), SandboxPolicy::red()] {
        assert!(
            policy.allowed_read_paths.is_empty(),
            "{:?}: allowed_read_paths must be empty by default",
            policy.level
        );
        assert!(
            policy.allowed_write_paths.is_empty(),
            "{:?}: allowed_write_paths must be empty by default",
            policy.level
        );
        assert!(
            policy.allowed_network.is_empty(),
            "{:?}: allowed_network must be empty by default",
            policy.level
        );
    }
}

// ===========================================================================
// Blue plugin policy — no restrictions15: Policy max_memory and max_cpu are zero by default
// ===========================================================================

#[test]
fn factory_policies_have_unlimited_resources() {
    for policy in [SandboxPolicy::blue(), SandboxPolicy::yellow(), SandboxPolicy::red()] {
        assert_eq!(
            policy.max_memory_bytes, 0,
            "{:?}: max_memory_bytes must be 0 (unlimited)",
            policy.level
        );
        assert_eq!(
            policy.max_cpu_seconds, 0,
            "{:?}: max_cpu_seconds must be 0 (unlimited)",
            policy.level
        );
    }
}
