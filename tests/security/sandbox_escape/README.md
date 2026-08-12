# Sandbox Escape Test Suite

Plugin sandbox isolation validation for the DoLogger engine. Verifies that the three-colour
trust model (Blue / Yellow / Red) correctly enforces syscall allowlists, plugin type restrictions,
and BPF seccomp filter generation.

Covers trust model enforcement, platform isolation policies, and seccomp-bpf filter validation.

## Trust Model Summary

| Colour  | Trust   | Sandbox Level | Allowed Plugin Types | Syscall Categories |
|---------|---------|--------------|----------------------|-------------------|
| Blue    | Full    | `None`       | ALL                  | ALL (no filter)   |
| Yellow  | Partial | `Restricted` | Most (no Config/Key/Policy/HostInfo/Syscall providers) | Memory, FileIO, Threading, Time, Signal, SystemInfo |
| Red     | None    | `Isolated`   | Filter, FieldProvider, Processor, Formatter, IOSink only | Memory, Threading, Time |

## Test Categories

### Policy Presets
Tests that `SandboxPolicy::blue()`, `SandboxPolicy::yellow()`, and `SandboxPolicy::red()`
produce correct configurations with the expected permission gates.

### Plugin Type Restrictions (5)
Validates `SandboxPolicy::check_plugin_type_allowed()` enforces the plugin type matrix
from Section 11.2, including case sensitivity and descriptive error messages.

### Policy Validation (6)
Tests `SandboxPolicy::validate()` for internal consistency: missing Memory category,
contradictory `allow_file_write` without FileIO, `allow_network` without Network, etc.

### Sandbox Engine Lifecycle (7)
Engine creation, enable/disable toggling, policy application paths, and backend detection.

### Syscall Allowlist (8)
Enumeration correctness of `SyscallCategory::linux_syscalls()`, category mutual exclusivity,
and cross-category denial verification for the red policy.

### Policy Structural (10)
Clone independence, field-by-field construction, resource limit configuration, and
equivalence testing.

### BPF Filter Validation (12) -- Linux only
Comprehensive seccomp-bpf filter analysis:
- Instruction type verification (LD_W_ABS, JEQ, RET)
- Relative jump offset (`jt`) correctness and monotonicity
- Syscall number sorting and deduplication
- Filter size formulas
- Red-excludes-network and Yellow-includes-fileio cross-checks
- Mandatory syscall presence (exit, exit_group, restart_syscall)
- Edge cases: empty allowlist, single syscall, large allowlists
- Full x86_64 syscall name-to-number lookup table verification

## Running the Tests

### Prerequisites

These tests depend on `dologger-core`. The test files live at the workspace root under
`tests/security/sandbox_escape/`. To compile and run:

**Option A: Copy into the core crate's test directory**

```bash
# Copy the test files into the core crate
cp -r tests/security/ core/tests/security/

# Run all sandbox escape tests
cargo test -p dologger-core sandbox_escape

# Run a specific test category
cargo test -p dologger-core sandbox_escape::policy_validation
cargo test -p dologger-core sandbox_escape::bpf_filter_validation
cargo test -p dologger-core sandbox_escape::plugin_type_restrictions
```

**Option B: Add a `[[test]]` target to `core/Cargo.toml`**

```toml
[[test]]
name = "sandbox_escape"
path = "../tests/security/sandbox_escape/mod.rs"
```

Then run:
```bash
cargo test -p dologger-core --test sandbox_escape
```

**Option C: Create a test crate**

Add a `Cargo.toml` in `tests/`:
```toml
[package]
name = "dologger-security-tests"
version = "0.1.0"
edition = "2021"

[[test]]
name = "sandbox_escape"
path = "security/sandbox_escape/mod.rs"

[dependencies]
dologger-core = { path = "../core" }
```

Add `"tests"` to the workspace members in the root `Cargo.toml`, then:
```bash
cargo test -p dologger-security-tests
```

### Platform Notes

- **BPF filter tests** (`bpf_filter_validation` module) run **only on Linux** (`#[cfg(target_os = "linux")]`).
  On Windows and macOS, these tests are silently skipped.
- **Policy and engine tests** run on all platforms.
- On Windows, the `AppContainer` backend is a skeleton (full process isolation not yet implemented).
  The `supports_isolation()` method returns `false` for `AppContainer`.

### Output

```
running XX tests
test sandbox_escape::blue_policy_has_no_level_restrictions ... ok
test sandbox_escape::blue_policy_allows_all_syscall_categories ... ok
test sandbox_escape::yellow_policy_has_restricted_level ... ok
...
test sandbox_escape::bpf_filter_validation::bpf_filter_starts_with_load_instruction ... ok
test sandbox_escape::bpf_filter_validation::bpf_filter_jeq_instructions_have_correct_relative_jumps ... ok
...
test result: ok. XX passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Extending the Tests

### Adding a new policy check

Add to the relevant section in `mod.rs`:

```rust
#[test]
fn my_new_red_policy_check() {
    let policy = SandboxPolicy::red();
    // Verify something about the red policy
    assert!(policy.allowed_categories.contains(&expected_category));
}
```

### Adding a new BPF filter test (Linux only)

Wrap in the `#[cfg(target_os = "linux")]`-gated `bpf_filter_validation` module or create a new
`#[cfg(target_os = "linux")]` module:

```rust
#[cfg(target_os = "linux")]
mod my_bpf_tests {
    use super::*;

    #[test]
    fn bpf_filter_new_check() {
        let filter = build_test_bpf_filter(&[42]);
        // Assert properties
    }
}
```

### Adding a new sandbox backend test

Add a test that checks platform-specific behavior using `#[cfg]`:

```rust
#[test]
fn windows_appcontainer_test() {
    #[cfg(windows)]
    {
        let engine = SandboxEngine::new();
        assert_eq!(engine.backend(), SandboxBackend::AppContainer);
        // Test AppContainer-specific behavior
    }
}
```

### Testing a new attack vector

Add a `mod` with a name matching the attack:

```rust
mod privilege_escalation_via_policy {
    use super::*;

    #[test]
    fn red_cannot_set_allow_network_and_pass_validation() {
        let mut policy = SandboxPolicy::red();
        policy.allow_network = true;
        // Should fail validation since red has no Network category
        assert!(policy.validate().is_err());
    }
}
```

## Related Test Files

| File | Focus |
|------|-------|
| `core/src/sandbox.rs` (inline tests) | Unit tests for sandbox module internals |
| `core/tests/security_tests.rs` | STRIDE attack penetration tests |
| `tests/security/sandbox_escape/mod.rs` | This file -- comprehensive sandbox validation |

## Design Notes

- The test suite replicates the private `build_bpf_filter` and `syscall_name_to_number` functions
  to validate BPF filter structure without access to internal sandbox module functions. This
  ensures the public API's behavior matches expectations.
- Relative jump offsets (`jt`) in BPF instructions count the number of instructions to skip
  forward. `jt=0` means "execute the next instruction." The tests verify this convention.
- `SandboxPolicy` does not currently derive `serde::Serialize`/`Deserialize`. Serialization
  equivalence is tested through `Clone` and field-by-field structural comparison. Add serde
  derives and JSON round-trip tests when persistence is needed.
