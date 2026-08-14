# DoLogger Security Development Specification

> 🌐 **语言 / Language**: [English](SecurityDevelopmentSpec.md) | [中文：安全开发规范](../../zh_CN/guides/SecurityDevelopmentSpec.md)

> **Version**: v0.1.0 | **Last Updated**: 2026-08-12 | **Target Audience**: Plugin Developers, Core Contributors, Security Auditors
>
> **Purpose**: This document defines mandatory security coding standards for DoLogger plugin development. It covers memory safety, input validation, the sandbox model, secret handling, cryptographic guidance, fuzzing requirements, and static analysis tooling. Compliance with this specification is required for all plugins, regardless of trust color.
>
> **Reading Path**: All plugin developers must read [Memory Safety Rules](#memory-safety-rules) and [Input Validation](#input-validation). Plugin authors targeting audit deployments must also read [Secret and Key Material Handling](#secret-and-key-material-handling). Security auditors should start with [The Plugin Sandbox Model](#the-plugin-sandbox-model) and [Fuzzing Requirements](#fuzzing-requirements).

## Table of Contents

1. [Memory Safety Rules](#memory-safety-rules)
2. [Input Validation](#input-validation)
3. [The Plugin Sandbox Model](#the-plugin-sandbox-model)
4. [Secret and Key Material Handling](#secret-and-key-material-handling)
5. [Cryptographic Do's and Don'ts](#cryptographic-dos-and-donts)
6. [Fuzzing Requirements](#fuzzing-requirements)
7. [Static Analysis Tooling](#static-analysis-tooling)
8. [Security Code Review Checklist](#security-code-review-checklist)
9. [Vulnerability Disclosure](#vulnerability-disclosure)

---

## Memory Safety Rules

### Core Principle

**DoLogger plugin code must never be the source of a memory safety violation in the host process.** Because plugins run in-process (even sandboxed ones), a memory corruption in a plugin can compromise the entire application.

### Required Rules

**Table 1: Memory Safety Rules**

| Rule | Description | Enforcement |
|:-:|:-:|:-:|
| **R1: No unsafe memory operations** | Do not use `unsafe` blocks in Rust plugins without explicit review and justification. C plugins must not perform manual memory management without matching alloc/free pairs verified by Valgrind. | Code review + Valgrind |
| **R2: Bounds checking** | Every array access, string operation, and buffer write must be bounds-checked. C plugins: use `snprintf` over `sprintf`, `strncpy` over `strcpy`, size-tracked buffers over raw pointers. | Static analysis (see [Static Analysis Tooling](#static-analysis-tooling)) |
| **R3: No use-after-free** | Do not retain pointers to freed memory. Set pointers to `NULL` after free. In Rust, this is enforced by the borrow checker -- only `unsafe` code can violate it. | Valgrind / AddressSanitizer |
| **R4: No double-free** | Free each allocation exactly once. Use allocation tracking in debug builds to detect double-frees. | Debug allocator + `dologger_internal.log` |
| **R5: No buffer overflow** | All VTable function output buffers are caller-allocated by the engine. The plugin **MUST NOT** write beyond the provided `length` parameter. Return `DO_LOG_ERR_BUFFER_TOO_SMALL` if the buffer is insufficient. | Fuzzing (see [Fuzzing Requirements](#fuzzing-requirements)) |
| **R6: Stack protection** | C plugins: compile with `-fstack-protector-strong`. Rust plugins: this is automatic via LLVM. | Compiler flags |
| **R7: Integer overflow** | Use checked arithmetic for all size calculations (especially buffer size computations). Rust: use `checked_add`, `saturating_add`, or enable `overflow-checks = true` in release. C: use `__builtin_add_overflow`. | Static analysis + fuzzing |

### Rule R1 in Detail: Unsafe Blocks

Rust plugins must treat every `unsafe` block as a security liability:

```rust
// (illustrative example — not compiled; record_ptr is a placeholder for a
// VTable record handle)
// REQUIRED: Every unsafe block must be accompanied by a SAFETY comment
// explaining WHY it is safe, not just WHAT it does.

// GOOD:
// SAFETY: The engine guarantees that record_ptr is a valid, non-null pointer
// for the duration of this VTable call. We only read from it, never mutate.
let record = unsafe { &*record_ptr };

// BAD:
let record = unsafe { &*record_ptr }; // No explanation
```

Consequences for violating R1:
- **Blue plugins**: Code review rejection; must fix before merge
- **Yellow plugins**: Plugin load rejected if `unsafe` count exceeds 5 without justification
- **Red plugins**: Plugin cannot contain `unsafe` blocks at all in the default security policy

### Memory Ownership Rules

Do not free memory that the engine owns, and do not assume the engine will free memory you allocated. See the [Plugin Development Guide](PluginDevelopmentGuide.md#memory-ownership-rules) for the complete ownership matrix.

---

## Input Validation

### What Must Be Validated

All data received from outside the plugin's own code must be treated as **untrusted** and validated before use.

**Table 2: Input Validation Requirements**

| Input Source | Validation Required | Rationale |
|:-:|:-:|:-:|
| `dologger_record_t *` fields | Null-check all pointer fields before dereference. Validate `level` is in range 0-6. | Plugins receive records from an asynchronous pipeline; a corrupted pointer is catastrophic. |
| `dologger_plugin_config_t *` | Validate all config values before use. Check string lengths, numeric ranges, enum values. | Configuration is loaded from TOML files that may be user-edited or corrupted. |
| Character strings in records | Treat all strings as potentially containing null bytes, control characters, or excessively long values. Truncate at a reasonable maximum. | Log injection attacks (CRLF injection, terminal escape sequences) originate from unvalidated strings. |
| Numeric fields | Bounds-check before using as array indices or size parameters. | Integer overflow or out-of-bounds access. |
| Batch arrays (`records`, `count`) | Verify `count > 0` before iterating. Verify each array element is non-null. | Defensive programming against engine bugs or malicious plugin chains. |

### Validation Pattern (C)

```c
// (pseudocode — illustrative, not compiled; the real Filter VTable callback is
// `int (*filter)(const dologger_record_handle_t *rec, void *config)` and the
// symbol names below are placeholders for the validation pattern)
dologger_error_t my_filter(dologger_record_t *record,
                           dologger_filter_result_t *result) {
    // Rule 1: Null-check all pointer parameters
    if (record == NULL || result == NULL) {
        return DO_LOG_ERR_INVALID_ARG;
    }

    // Rule 2: Validate record fields before use
    if (record->level > DO_LOG_AUDIT) {
        // Invalid level -- drop suspicious record, do not crash
        result->action = DO_LOG_FILTER_DROP;
        return DO_LOG_OK;  // Return OK so pipeline continues
    }

    // Rule 3: Validate string pointer and length
    if (record->message != NULL) {
        // Enforce a maximum message length to prevent memory exhaustion
        size_t msg_len = strnlen(record->message, DO_LOG_MAX_MESSAGE_LEN);
        if (msg_len >= DO_LOG_MAX_MESSAGE_LEN) {
            // Message too long -- drop, do not crash
            result->action = DO_LOG_FILTER_DROP;
            return DO_LOG_OK;
        }
    }

    // ... business logic ...

    return DO_LOG_OK;
}
```

### Validation Pattern (Rust)

```rust
// (illustrative example — not compiled; the Rust adapter API is `Logger` with
// `trace/debug/info/warn/error/fatal/audit` — see adapters/rust/src/lib.rs)
fn my_filter(record: &Record, result: &mut FilterResult) -> DoLogError {
    // Rule 1: Validate level
    if record.level > LogLevel::Audit as u8 {
        result.action = FilterAction::Drop;
        return Ok(());
    }

    // Rule 2: Validate message
    if let Some(msg) = record.message() {
        if msg.len() > DO_LOG_MAX_MESSAGE_LEN {
            result.action = FilterAction::Drop;
            return Ok(());
        }
    }

    // ... business logic ...

    Ok(())
}
```

### Principles

1. **Fail safe, not fail open**: When validation fails, the default action must be to drop/discard, not to pass through.
2. **Never crash**: Do not `panic!()`, `abort()`, or `exit()` in a VTable function. Return an error code.
3. **Log violations**: Report validation failures via the engine's diagnostic log so operators can detect attacks.
4. **Defense in depth**: Validate at the plugin boundary even if the engine also validates. Plugins may be loaded out of order or in unexpected combinations.

---

## The Plugin Sandbox Model

### What the Sandbox Does

The sandbox restricts which operating system operations a plugin can perform. It is applied **after** `dlopen()` and **before** `plugin_init()`. Once a sandbox is active, it cannot be relaxed -- only tightened.

**Table 3: Sandbox Capabilities by Trust Color**

| Capability | Blue | Yellow | Red |
|:-:|:-:|:-:|:-:|
| Memory allocation (`mmap`, `munmap`, `brk`) | Yes | Yes | Yes |
| Thread operations (`clone`, `futex`) | Yes | Yes | Yes |
| Time functions (`clock_gettime`) | Yes | Yes | Yes |
| File I/O (`open`, `read`, `write`, `close`) | Yes | Yes | **No** |
| Network (`socket`, `connect`, `sendto`) | Yes | **No** | **No** |
| Process creation (`fork`, `execve`) | Yes | **No** | **No** |
| Signal handling (`sigaction`, `tgkill`) | Yes | Yes | **No** |

### What the Sandbox Means for Plugin Developers

**If you are developing a Blue plugin**: The sandbox is not applied. You have full access to the operating system. But you are expected to use that access responsibly -- you are running with the host application's privileges and a vulnerability in your plugin is a vulnerability in the application.

**If you are developing a Yellow plugin**: The sandbox is partially applied.

- You **can** read and write files (for configuration, state persistence, temporary data).
- You **cannot** open network connections. If your plugin needs network access (e.g., a `ConfigProvider` that fetches from a remote URL), you must request Blue trust with appropriate justification.
- You **cannot** spawn child processes. Use the engine's built-in parallelism -- do not fork.
- Attempting a disallowed syscall results in **immediate thread termination** (`SECCOMP_RET_KILL_PROCESS` on Linux). There is no error code, no recovery -- the plugin thread dies.

**If you are developing a Red plugin**: The sandbox is maximally restrictive.

- You **cannot** access the filesystem, network, or create processes.
- You **can** allocate memory, use threads, and query time. This is sufficient for stateless or purely computational plugins (e.g., a Filter that checks record fields, a Processor that redacts text).
- All output goes to the `ext.*` field namespace (Ring 3, CRC32C integrity only). You cannot write to the `verified.*` namespace.
- Red plugins are **disabled by default**. The host operator must explicitly set `allow_red_plugins = true`.

### Developing Within Sandbox Constraints

```c
// (illustrative pseudocode — not compiled; the v0.1.0 actual plugin entry is
// `int plugin_init(const void *config)` and `dologger_plugin_config_t` does
// not exist)
// YELLOW PLUGIN: Do NOT do this -- network is denied
dologger_error_t my_plugin_init(const dologger_plugin_config_t *config) {
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    // This will trigger SECCOMP_RET_KILL_PROCESS on Linux.
    //   Your plugin thread is dead. No error code. No recovery.
}

// YELLOW PLUGIN: Do this instead -- use the engine's ConfigProvider chain:
dologger_error_t my_plugin_init(const dologger_plugin_config_t *config) {
    // The engine provides configuration. Use it.
    const char *remote_url = dologger_config_get(config, "remote_url");
    // If remote fetching is essential, upgrade to Blue trust.
}
```

### Testing Sandbox Compliance

Test your plugin under sandbox constraints before shipping:

```bash
# Linux: run under strace to audit syscall usage
sudo strace -f -e trace=file,network,process \
    ./target/debug/examples/simple_logger 2>&1 | grep -v ENOENT

# Check: any unexpected open(), socket(), fork() calls?

# Force Yellow sandbox for testing a Blue plugin
# (edit dologger.toml: trust.color = "yellow" for test run)
```

---

## Secret and Key Material Handling

### The Prime Directive

**Never log secrets.** A logging engine is the single worst place to leak credentials -- logs are persisted, replicated, shipped to centralized platforms, and may be retained for years.

### Rules

**Table 4: Secret Handling Rules**

| Rule | Description |
|:-:|:-:|
| **S1: Never log raw secrets** | Do not write API keys, passwords, tokens, private keys, session cookies, or PII to any `DO_LOG_*` call or any `record.message` field. |
| **S2: Never store secrets in plugin state** | Plugin state is serialized during hot reload. The `dologger_state_buf_t` is stored in plaintext. Do not place key material in serialized state. |
| **S3: Use the SecretDetector API** | If your plugin processes text that may contain secrets, call the engine's `dologger_secret_scan()` API before logging or formatting. |
| **S4: Redact before shipping** | If a `Processor` plugin enriches records with sensitive context (e.g., user PII), ensure a downstream Processor or Filter redacts or masks it before it reaches a networked Sink. |
| **S5: KeyProvider is for keys** | If your plugin needs signing or encryption keys, do not hardcode them, read them from config, or store them in plugin memory. Use a `KeyProvider` plugin with HSM/KMS backing. |
| **S6: Audit all secret access** | Every time your plugin accesses a secret (reads a key, decrypts data), emit an `DO_LOG_AUDIT` record documenting the access. |

### Using the SecretDetector API

```c
// (illustrative pseudocode — not compiled; the engine's SecretDetector lives
// in core/src/security/secret_detector.rs and is exposed through the Rust
// pipeline, not through this C symbol)
// Before logging text that may contain secrets, scan it
dologger_secret_scan_result_t scan_result;
dologger_error_t rc = dologger_secret_scan(
    untrusted_input_text,
    strlen(untrusted_input_text),
    &scan_result
);

if (scan_result.secret_detected) {
    // Replace the detected secret with a placeholder
    // e.g., "api_key=sk-abc123" -> "api_key=<REDACTED>"
    memset(scan_result.secret_start, '*', scan_result.secret_length);
    // Emit an audit record documenting the redaction
    DO_LOG_AUDIT(logger, "SecretDetector: redacted %zu bytes at offset %zu",
                 scan_result.secret_length, scan_result.secret_offset);
}
```

### What the SecretDetector Detects

The built-in `SecretDetector` scans for these patterns:

| Pattern | Example | Regex |
|:-:|:-:|:-:|
| AWS Access Key | `AKIAIOSFODNN7EXAMPLE` | `AKIA[0-9A-Z]{16}` |
| Stripe API Key | `sk_live_example_placeholder_key` | `sk_live_[0-9a-zA-Z]{24}` |
| GitHub Token | `ghp_example_placeholder_token` | `ghp_[0-9a-zA-Z]{36}` |
| JWT Token | `eyJhbGciOiJIUzI1NiIs...` | `eyJ[0-9a-zA-Z_-]+` |
| Private Key (PEM) | `-----BEGIN RSA PRIVATE KEY-----` | `-----BEGIN .* PRIVATE KEY-----` |
| Password in URL | `postgres://user:secret@host/db` | URI with password component |
| Base64 high-entropy | 40+ character base64 with entropy > 4.5 bits/char | Shannon entropy check |

Custom patterns can be added via the `SecretDetector` Processor plugin configuration.

---

## Cryptographic Do's and Don'ts

### Approved Algorithms

**Table 5: Cryptographic Algorithm Policy**

| Operation | Use | Do NOT Use |
|:-:|:-:|:-:|
| Signatures | Ed25519 (via `KeyProvider`) | RSA, DSA, ECDSA |
| Hashing | SHA-256, SHA-512 | MD5, SHA-1 |
| Integrity (non-crypto) | CRC32C (hardware-accelerated) | CRC32, Adler-32 |
| Encryption at rest | AES-256-GCM | AES-ECB, DES, 3DES |
| Key exchange | X25519 (if needed for plugin-to-plugin) | DH-1024, static RSA |
| Random numbers | OS CSPRNG (`getrandom` syscall, `/dev/urandom`) | `rand()`, `srand()`, `drand48()` |

### Cryptographic Do's

1. **DO** delegate signing to the engine's `KeyProvider` chain. Do not implement signing yourself.
2. **DO** use constant-time comparison for all security-sensitive data (`CRYPTO_memcmp` in C, `subtle` crate in Rust).
3. **DO** zeroize key material after use (`explicit_bzero` / `zeroize` crate).
4. **DO** store keys in HSM/KMS-backed KeyProvider plugins, never in plugin state or config files.
5. **DO** verify the Ed25519 signature of audit records before relying on their contents (if your plugin consumes audit data).

### Cryptographic Don'ts

1. **DON'T** implement your own cryptography. Use the engine's API or well-audited libraries (`ring`, `ed25519-dalek`, `rustls`).
2. **DON'T** use predictable or low-entropy seeds. Always seed from `/dev/urandom` or `getrandom()`.
3. **DON'T** reuse the same key for signing and encryption. Separate keys for separate purposes.
4. **DON'T** use deprecated hash functions (MD5, SHA-1) for any security purpose. CRC32C is acceptable only for non-security integrity checks on Ring 3 data.
5. **DON'T** hardcode cryptographic keys, IVs, or nonces. Every key must come from a `KeyProvider`. Every nonce must be generated fresh.

### Handling Ed25519 Signatures

If your plugin processes or verifies Ed25519 signatures:

```c
// (illustrative pseudocode — not compiled; no dologger_verify_record_signature
// symbol exists yet — use dologctl verify-log for offline verification)
// DO: Use the engine's verification API
dologger_error_t rc = dologger_verify_record_signature(
    engine_handle,
    record,
    &verification_result    // -> DO_LOG_SIG_VALID / INVALID / NOT_SIGNED
);

// DON'T: Re-implement signature verification yourself
// ed25519_dalek_verify(record->signature, ...)  <-- NO
```

The engine manages public key distribution, key rotation, and CRL checking. Your plugin should not duplicate this infrastructure.

---

## Fuzzing Requirements

### When Fuzzing is Required

**Table 6: Fuzzing Requirements by Plugin Type**

| Plugin Type | Fuzzing Required? | Rationale |
|:-:|:-:|:-:|
| `Filter` | No | Only reads record fields; no parsing |
| `PolicyProvider` | No | Only reads metrics counters |
| `FieldProvider` | No | Only writes fields; no parsing |
| `HostInfoProvider` | No | Reads OS APIs; no external input |
| `Processor` | **Yes** | Transforms record content -- may parse structured data |
| `Formatter` | **Yes** | Serializes records -- must handle all field values and malformed UTF-8 |
| `ConfigProvider` | **Yes** | Parses external configuration formats (TOML, JSON, YAML) |
| `KeyProvider` | **Yes** | Handles cryptographic key material and signature operations |
| `SyscallBroker` | **Yes** | Intercepts and proxies arbitrary syscall arguments |

### Fuzzing Targets

For each plugin that requires fuzzing, provide at least one fuzz target:

```rust
// (template — not compiled; `my_formatter` is a placeholder. The engine's real
// fuzz targets live in core/fuzz/fuzz_targets/: fuzz_ring_buffer,
// fuzz_sif_record, fuzz_toml_config)
// fuzz/fuzz_targets/format_json.rs
#![no_main]

use libfuzzer_sys::fuzz_target;
use my_formatter::format_record;

fuzz_target!(|data: &[u8]| {
    // Construct a mock record from fuzzer input
    if let Ok(record) = mock_record_from_bytes(data) {
        let mut output = vec![0u8; 4096];
        // The formatter must never panic or corrupt memory
        let _ = format_record(&record, &mut output);
    }
});
```

### Fuzzing Requirements Checklist

- [ ] At least one fuzz target per plugin type marked "Yes" in Table 6
- [ ] Fuzz target linked to CI via `cargo fuzz`
- [ ] 24 hours of fuzzing with zero crashes before plugin release
- [ ] AddressSanitizer (`-Z sanitizer=address`) enabled during fuzzing
- [ ] Fuzzing corpus checked into the repository (`fuzz/corpus/`)
- [ ] OSS-Fuzz integration (planned)

### Running Fuzz Tests Locally

```bash
# (illustrative template — `format_json` is a placeholder; substitute your own
# target name, e.g. the engine's core/fuzz/fuzz_targets targets)
# Install cargo-fuzz
cargo install cargo-fuzz

# Run a specific fuzz target for 60 seconds
cargo fuzz run format_json -- -max_total_time=60

# Run with AddressSanitizer
RUSTFLAGS="-Z sanitizer=address" cargo +nightly fuzz run format_json

# Minimize a crashing input
cargo fuzz tmin format_json fuzz/artifacts/format_json/crash-xxxxx

# Replay a crash
cargo fuzz run format_json fuzz/artifacts/format_json/crash-xxxxx
```

### What Constitutes a Fuzzing Failure

- **Crash**: Segmentation fault, assertion failure, `panic!()` -- **BLOCKING**. Must fix before release.
- **Timeout**: Function takes > 5 seconds with 4 KB input -- **WARNING**. Review for algorithmic complexity attacks.
- **OOM**: Allocates > 1 GB -- **WARNING**. Review for memory exhaustion DoS.
- **Incorrect output**: Produces output that fails the plugin's own round-trip test -- **BLOCKING**. Indicates logic bug.

---

## Static Analysis Tooling

### Required Tools and Configuration

All DoLogger plugins must pass the following static analysis checks in CI:

**Table 7: Static Analysis Tool Chain**

| Tool | Command | What It Checks | Severity on Failure |
|:-:|:-:|:-:|:-:|
| **cargo audit** | `cargo audit` | Known vulnerabilities (CVEs) in dependency tree | **BLOCKING** |
| **cargo deny** | `cargo deny check advisories` | RustSec advisory database | **BLOCKING** |
| **cargo deny** | `cargo deny check licenses` | License compliance (see [Plugin Development Guide](PluginDevelopmentGuide.md#license-compliance)) | **BLOCKING** |
| **cargo deny** | `cargo deny check bans` | Duplicate crate versions, wildcard dependencies | **WARNING** |
| **cargo deny** | `cargo deny check sources` | Unknown or untrusted crate sources | **BLOCKING** |
| **clippy** | `cargo clippy -- -D warnings` | Idiomatic Rust, correctness lints, perf lints | **BLOCKING** |
| **rustfmt** | `cargo fmt --check` | Code formatting consistency | **WARNING** |

### Cargo Audit Configuration

```bash
# Run cargo audit -- fails on any vulnerability
cargo audit

# Run with specific severity threshold
cargo audit --deny unsound --deny warnings

# Ignore a specific advisory (requires justification in audit.toml)
cargo audit --ignore RUSTSEC-2024-XXXX
```

### Cargo Deny Configuration

The project `deny.toml` (repository root) contains the canonical deny configuration. The excerpt below shows the intent; the shipped file's exact allow/deny lists may differ, so always check the real `deny.toml` in the repository root:

```toml
# deny.toml (excerpt — matches the repository's actual file)
[graph]
all-features = true

[licenses]
version = 2
private = { ignore = true }

[licenses.allow]
mit = "allow"
apache-2.0 = "allow"
bsd-2-clause = "allow"
bsd-3-clause = "allow"
isc = "allow"
zlib = "allow"
# ... (see the deny.toml in the repo root for the full list) ...

[licenses.deny]
gpl-2.0-only = "deny"
gpl-2.0-or-later = "deny"
gpl-3.0-only = "deny"
gpl-3.0-or-later = "deny"
agpl-3.0-only = "deny"
agpl-3.0-or-later = "deny"
# ... (see the deny.toml in the repo root for the full list) ...

[bans]
multiple-versions = "warn"
wildcards = "deny"           # Deny wildcard dependencies

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

(Note: this file uses cargo-deny's `version = 2` mapping format, which requires cargo-deny 1.x+; cargo-deny 0.x fails with an "expected an array" parse error. The repository currently has no `[advisories]` section.)

### Clippy Configuration

```bash
# Run clippy with all lints
cargo clippy --all-targets --all-features -- -D warnings

# Additional security-critical lints
cargo clippy -- -W clippy::unwrap_used \
                 -W clippy::expect_used \
                 -W clippy::integer_arithmetic \
                 -W clippy::cast_possible_truncation \
                 -W clippy::cast_possible_wrap \
                 -W clippy::indexing_slicing
```

### CI Integration

```yaml
# (illustrative template — not the literal workflow; the repository's actual
# pipeline is .github/workflows/security.yml)
# .github/workflows/security-checks.yml
name: Security Checks

on: [push, pull_request]

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: cargo install cargo-audit cargo-deny
      - run: cargo audit --deny warnings
      - run: cargo deny check advisories
      - run: cargo deny check licenses
      - run: cargo deny check bans

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: cargo clippy --all-targets --all-features -- -D warnings

  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: cargo fmt --all -- --check
```

### For C Plugins

C plugin developers should add:

| Tool | Command | What It Checks |
|:-:|:-:|:-:|
| **Valgrind** | `valgrind --leak-check=full --show-leak-kinds=all` | Memory leaks, use-after-free, double-free, uninitialized reads |
| **AddressSanitizer** | `-fsanitize=address` | Buffer overflow, use-after-free, stack overflow |
| **UndefinedBehaviorSanitizer** | `-fsanitize=undefined` | Integer overflow, null pointer dereference, alignment violations |
| **Coverity / CodeQL** | CI integration | Comprehensive inter-procedural analysis |

```bash
# (example — Linux; replace my_plugin.c/my_plugin.so with your own file names)
# C plugin security build flags
cc -shared -fPIC \
   -fstack-protector-strong \
   -D_FORTIFY_SOURCE=2 \
   -fsanitize=address \
   -fsanitize=undefined \
   -O2 -g \
   -o my_plugin.so my_plugin.c
```

---

## Security Code Review Checklist

Every plugin code review must verify the following items before merge.

### Memory Safety

- [ ] No unsafe blocks without `// SAFETY:` justification comment
- [ ] All pointer parameters null-checked before dereference
- [ ] All array accesses bounds-checked
- [ ] All buffer writes respect the provided size limit
- [ ] Integer arithmetic uses checked operations for size calculations
- [ ] Plugin does not free memory owned by the engine

### Input Validation

- [ ] All config values validated (ranges, enums, lengths)
- [ ] All record fields validated before use
- [ ] String lengths checked before copy
- [ ] Batch counts validated before iteration
- [ ] Invalid input causes `Drop` or returns an error, never crashes

### Sandbox Compliance

- [ ] Plugin's `manifest.toml` `[capabilities]` match actual syscall usage
- [ ] No network operations in Yellow/Red plugins
- [ ] No process creation in Yellow/Red plugins
- [ ] No file I/O in Red plugins
- [ ] Plugin tested under sandbox constraints

### Secrets and Cryptography

- [ ] No hardcoded keys, tokens, or passwords
- [ ] No secrets in log messages
- [ ] No secrets in serialized plugin state
- [ ] `SecretDetector` used before logging untrusted text
- [ ] Cryptographic operations delegated to engine API
- [ ] Outdated algorithms (MD5, SHA-1, DES) not used

### Static Analysis

- [ ] `cargo audit` passes with zero vulnerabilities
- [ ] `cargo deny check` passes (advisories, licenses, bans, sources)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] (C plugins) Valgrind reports zero errors
- [ ] (C plugins) ASan + UBSan report zero errors

### Fuzzing (if applicable)

- [ ] Fuzz target exists for the plugin type
- [ ] 24 hours crash-free on the final commit
- [ ] Fuzz corpus committed to repository

---

## Vulnerability Disclosure

### Reporting a Vulnerability

If you discover a security vulnerability in DoLogger or any plugin:

1. **DO NOT** file a public issue.
2. Email `nekoliowork+DoLogger@gmail.com` with:
   - Description of the vulnerability
   - Steps to reproduce
   - Affected versions (engine, plugin, platform)
   - Any proof-of-concept code
3. Allow up to 72 hours for an initial response.

### Disclosure Timeline

| Severity | Patch Timeline | Disclosure |
|:-:|:-:|:-:|
| **Critical** (RCE, sandbox escape, signature bypass) | 7 days | Coordinated with reporter |
| **High** (information disclosure, privilege escalation) | 14 days | Coordinated with reporter |
| **Medium** (DoS, minor data leak) | 30 days | Public disclosure in release notes |
| **Low** (defense-in-depth improvements) | Next release | Public disclosure in release notes |

### Security Advisories

Published security advisories are available at:

```
https://github.com/Nekolio/DoLogger/security/advisories
```

Each advisory includes:
- CVE identifier (if assigned)
- Affected version range
- Patched version
- Severity rating (CVSS v3.1 score)
- Mitigation steps for users who cannot immediately upgrade

### Plugin Developer Responsibility

If a security vulnerability is found in **your** plugin:

1. You will be notified via the contact email in your `manifest.toml`.
2. You are expected to ship a patch within the timeline appropriate for the severity.
3. If the vulnerability is critical and unpatched after 14 days, the plugin will be removed from the official plugin repository and blacklisted by the engine's advisory check.
4. DoLogger's own `cargo audit` / `cargo deny` pipeline will flag your plugin as vulnerable if it depends on a crate with a known CVE. Keep your dependencies updated.
