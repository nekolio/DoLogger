# DoLogger

> Next-gen secure logging — Ed25519 audit chains at lock-free speed.

<p align="center">
  <img src="./docs/assets/hero.svg" alt="DoLogger boot sequence — Hello DoLogger, 4 sandboxed plugins, Ed25519 chain armed, 7-stage pipeline online" width="880">
</p>

[English](README.md) | [中文](README.zh_CN.md)

<p align="center">
  <a href="https://github.com/Nekolio/DoLogger/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Nekolio/DoLogger/ci.yml?branch=main&style=flat-square&label=CI" alt="CI"></a>
  <a href="https://github.com/Nekolio/DoLogger/releases"><img src="https://img.shields.io/github/v/release/Nekolio/DoLogger?include_prereleases&style=flat-square&label=release" alt="Release"></a>
  <a href="https://github.com/Nekolio/DoLogger/stargazers"><img src="https://img.shields.io/github/stars/Nekolio/DoLogger?style=flat-square&color=yellow" alt="Stars"></a>
  <a href="https://github.com/Nekolio/DoLogger/blob/main/LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/rust-stable-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/platform-Linux_|_macOS_|_Windows-808080?style=flat-square" alt="Platform">
  <a href="https://github.com/Nekolio/DoLogger/commits/main"><img src="https://img.shields.io/github/last-commit/Nekolio/DoLogger?style=flat-square&label=last%20commit" alt="Last commit"></a>
</p>

---

## Overview

DoLogger is a cross-platform, high-security logging engine for applications that
need signed, tamper-evident audit logs. It combines nanosecond-latency lock-free
record submission with Ed25519-signed audit chains, plugin sandboxing, and
11 built-in output sinks — all driven by TOML configuration with domain
inheritance and non-downgradable security guarantees.

| Capability | DoLogger | Traditional Loggers |
|:-:|:-:|:-:|
| **Submit latency (P50)** | 102 ns | 500–2000 ns |
| **Batch throughput** | 13.3M rec/s | 1–5M rec/s |
| **Audit chain** | Ed25519 + LSN + prev_hash blockchain | Rare / bolt-on |
| **Plugin sandbox** | seccomp-bpf / AppContainer / Sandbox | None |
| **Performance profiles** | 4 profiles (Dev/Prod/ProdAudit/Balanced) | Manual tuning |
| **Output sinks** | 11 built-in (Console, File, Callback, Kafka, Syslog, Webhook, SQLite, WORM, Security, Shared Memory, OTel) | 1–3 typical |
| **Configuration** | TOML + domain inheritance + 7-level priority | Flat config |

---

## Features

- `[PERF]` **Lock-free hot path** — CAS-based ring buffer with a Treiber object pool; record submission does no heap allocation (P50 ≈ 102 ns locally).
- `[SIGN]` **Ed25519 audit chain** — every audit record is signed at assembly time and chained by LSN + prev_hash; verify offline with `dologctl verify-log`.
- `[SINK]` **11 sinks + sandboxed plugins** — Console, File, Kafka, Syslog, Webhook, SQLite, WORM, Security, Shared Memory, OTel, Callback; plugins run under seccomp-bpf / AppContainer / Sandbox isolation with color-coded trust levels.
- `[OBSV]` **Observability built in** — per-record pipeline timing (`--trace`), SIF recording/replay, `dologctl perf` benchmarks, crash recovery reports.

---

## Performance Snapshot

Measured on the same code (release + LTO); no head-to-head numbers against
other Rust loggers are published yet — each release carries fresh measurements
from the GitHub Actions runner in its release notes.

| Environment | Submit P50 | Throughput | Signed submit (Ed25519) |
|:-:|:-:|:-:|:-:|
| GitHub runner — AMD EPYC 7763, v0.1.0 release | **120 ns** | 5.06M rec/s | 19.8 µs |
| Local — Windows 11 LTSC, Intel i5-12400F | **102 ns** | 9.78M rec/s | 16.96 µs |

Criterion (same local machine):

| Benchmark | P50 | Throughput |
|:-:|:-:|:-:|
| Single record submit | **102 ns** | ~9.78M rec/s |
| Ring buffer push (1K) | **121 µs** | ~8.26M rec/s |
| Batch push (256) | **19.2 µs** | ~13.3M rec/s |

CRC32C is hardware-accelerated via SSE 4.2 (`_mm_crc32_u64`) with a
Slicing-by-8 software fallback.

---

## Quick Start

### Prebuilt binaries

Every [GitHub Release](https://github.com/Nekolio/DoLogger/releases) attaches
`dologctl-<version>-<os>-<arch>` binaries (`.exe` on Windows) plus per-arch
core libraries and official plugins. Official plugins ship as ONE bundle per
platform — `dologger-official-plugins-<version>-<os>-<arch>.{so|dll|dylib}` —
hosting every official plugin (formatter-json, formatter-text, filter-level,
field-container). Verify each download against the attached
`checksums-sha256.txt`:

```bash
curl -fLO https://github.com/Nekolio/DoLogger/releases/download/v0.1.0/dologctl-linux-x86_64
chmod +x dologctl-linux-x86_64
./dologctl-linux-x86_64 init --template dev
./dologctl-linux-x86_64 run --config dologger.toml
```

### Build from source

```bash
git clone https://github.com/Nekolio/DoLogger.git
cd DoLogger
cargo build --release

./target/release/dologctl init --template dev
./target/release/dologctl run --config dologger.toml
```

### Rust SDK

The SDK ships in the repository (`adapters/rust`); add it as a path
dependency: `dologger-sdk = { path = "adapters/rust" }`.

```rust
use dologger_sdk::Logger;

fn main() {
    let mut logger = Logger::init(None).expect("init"); // default config
    logger.info("Application started");
    logger.audit("User 42 deleted record #7"); // signed + WORM
    logger.shutdown();
}
```

Audit records are Ed25519-signed; verify the log offline:

```shell
dologctl verify-log audit.log
```

### Shell Completions

```bash
source <(dologctl completions bash)                              # bash
source <(dologctl completions zsh)                               # zsh
dologctl completions fish | source                               # fish
dologctl completions powershell | Out-String | Invoke-Expression # PowerShell
```

> [!TIP]
> Persist the completion script in your shell profile so every new terminal has it, e.g. `dologctl completions bash > ~/.dologctl-complete.bash && echo 'source ~/.dologctl-complete.bash' >> ~/.bashrc`.

---

## Architecture

> [!IMPORTANT]
> DoLogger is **pre-1.0**. MINOR releases may include breaking changes and the ABI may change — pin to an exact version in production. See the [Versioning & Deprecation Policy](docs/en_US/guides/VersioningAndDeprecation.md).

![Architecture](./docs/assets/architecture.svg)

The application pushes records straight into a lock-free MPSC ring buffer — no
locks on the hot path. A background pipeline runs seven stages
(PreFilter → Filter → FieldProvider → Assembly → Processing → Formatting →
Sink), signing audit records with Ed25519 at the Assembly stage and verifying
checksums at Processing. Batched drains fan out to the sink layer, so slow I/O
never blocks the producer.

### Key Design Decisions

- **Lock-free hot path**: CAS-based ring buffer + Treiber stack object pool — zero malloc on record submission
- **Ring 0–3 field permissions**: CPU-style privilege rings for log fields; Ring 2 modifications auto-appended to audit trail
- **AUDIT iron law**: `block_timeout_ms=0`, `drop_strategy=Never` — audit records are never dropped
- **Backpressure**: 90% alert + cooperative helping, 95% emergency + optional drop
- **6 non-downgradable items**: `enable_signature`, `escape_html`, `worm_enabled`, `fsync_on_write`, `require_tls`, `sign_ring2`
- **4 performance profiles**: Dev / ProdPerformance / ProdAudit / Balanced — each binding to concrete timeouts and strategies

---

## Configuration & Deployment

Default configuration works out of the box — `dologctl run` with no config
uses built-in defaults, and `dologctl init --template dev` generates a
development template.

| Environment variable | Purpose |
|:-:|:-:|
| `DO_LOGGER_LIB_PATH` | Path to the shared library for language adapters |
| `DO_LOG_PLUGIN_DIR` | Plugin search path (overrides `./plugins`) |
| `DO_LOG_CONFIG_FILE` | Config file for `dologctl config validate` |

```shell
dologctl init --template gdpr    # EU GDPR
dologctl init --template hipaa   # US HIPAA
dologctl init --template pci     # PCI-DSS
```

Compliance templates activate the non-downgradable items and enforce audit
requirements automatically.

---

## dologctl CLI

```
dologctl init                    Generate config template
dologctl run --trace             Run engine with per-record timing
dologctl plugin list             List installed plugins with trust colors
dologctl plugin install <path>   Install a plugin
dologctl plugin verify [name]    Verify plugin signature and ABI
dologctl plugin scan             Security scan for suspicious symbols
dologctl plugin keygen <path>    Generate an Ed25519 signing key pair
dologctl plugin sign <lib> <key> Sign a plugin (writes <lib>.sig sidecar)
dologctl plugin wrap-key/unwrap  AES-256-GCM encrypt/decrypt a signing seed
dologctl plugin verify --trust-store  Verify plugins against committed anchors+CRL
dologctl config validate         Validate config with --strict compliance
dologctl verify-log <file>       Offline audit log verification
dologctl verify-anchor           External anchoring verification
dologctl recovery-report         Crash recovery report
dologctl record / replay         SIF recording and replay
dologctl shm status              Shared memory channel inspection
dologctl perf                    Performance benchmarks
dologctl completions <shell>     Shell completion script
dologctl version                 Project banner with system info
dologctl version --licenses      Third-party license attributions
```

Global flags: `--output json|text`, `--color auto|always|never`, `--quiet`, `--config <path>`

---

## Plugin System (9 VTable Types)

| Plugin Type | Phase | Description |
|:-:|:-:|:-:|
| **Filter** | 1 | Drop or pass records based on rules |
| **FieldProvider** | 2 | Inject fields (HostInfoProvider is a restricted subtype) |
| **Processor** | 4 | Transform / enrich / detect secrets |
| **Formatter** | 5 | Serialize records to output format |
| **ConfigProvider** | — | External config source (remote config center) |
| **KeyProvider** | — | Ed25519 key service (externalize to HSM) |
| **PolicyProvider** | 0 | Pre-submit policy (rate limiting, level filtering) |
| **HostInfoProvider** | 2 | System info injection (ring1_only=true) |
| **SyscallBroker** | — | System call proxy for sandboxed plugins |

Sink is **not** a plugin type. It is a core built-in output executor: the 11
built-in sinks (Console, File, Callback, Kafka, Syslog, Webhook, SQLite, WORM,
Security, Shared Memory, OTel) run at pipeline stage 6 and are executed directly
by the core; plugins process records before the Sink stage. Callback Sink is the
standard extension point through which a host application receives the final
output.

### Trust Levels

| Level | Color | Signature Required | Syscall Access | Plugin Types |
|:-:|:-:|:-:|:-:|:-:|
| **Blue** | blue | Ed25519 signed | Full | All |
| **Yellow** | yellow | Self-signed | Restricted | Limited |
| **Red** | red | None (dev mode) | Minimal allowlist | Filter, Formatter, Processor only |

---

## Security

- **Ed25519 audit chain**: Every audit record is signed; LSN + prev_hash forms a blockchain-like tamper-proof chain
- **WORM storage**: Write-Once-Read-Many with fsync + read-only permission enforcement
- **Plugin sandbox**: seccomp-bpf (Linux), AppContainer (Windows), Sandbox (macOS) with trust-colored capability matrices
- **Secret detection**: 14 prefix-matching rules across Critical/High/Medium severity (AWS, GCP, GitHub tokens, private keys)
- **Key rotation + CRL**: Multi-key parallel verification, rotation lifecycle, emergency revocation
- **External anchoring**: Periodic root hash anchoring to immutable storage (S3/HTTP)
- **Circuit breaker**: 3-state (CLOSED→OPEN→HALF_OPEN→CLOSED) for remote sink fault isolation
- **Emergency mmap buffer**: AES-256-GCM encrypted spill buffer for ring buffer overflow

---

## Language Adapters

| Language | Location | Status |
|:-:|:-:|:-:|
| **Rust** | `adapters/rust/` | SDK crate (dologger-sdk) |
| **Python** | `adapters/python/` | ctypes wrapper |
| **Go** | `adapters/go/` | cgo wrapper |

---

## Project Structure

<details>
<summary>Repository layout</summary>

The top-level layout mirrors the three-layer architecture — **core** (stable
kernel) / **product packages** (built on the core C ABI) / **host apps**
(examples consuming the C ABI):

| Layer | Directory |
|:-:|:-:|
| Stable kernel | [`core/`](core/) |
| Product packages | [`cli/`](cli/), [`adapters/`](adapters/), [`plugins/`](plugins/), [`compliance/`](compliance/) |
| Host app examples | [`examples/`](examples/) |
| Docs & peripheral | [`docs/`](docs/), [`peripheral/site/`](peripheral/site/), [`peripheral/tools/`](peripheral/tools/) |
| Product support | [`config/`](config/), [`tests/`](tests/) |
| Build / CI / infra | `scripts/`, `cmake/`, `docker/`, `.github/` |

```
DoLogger/
├── core/                       # Stable kernel — Core engine (Rust cdylib + rlib, stable C ABI)
│   ├── src/                    # 50+ modules — ring buffer, pipeline, 11 sinks, plugins, security
│   ├── include/                # C ABI public headers (dologger_core.h, dologger_shm.h)
│   ├── benches/                # Criterion benchmarks (throughput, latency, percentiles)
│   ├── examples/               # C-FFI usage examples (file, simple, sqlite)
│   ├── fuzz/                   # Fuzz targets (ring buffer, SIF, TOML config)
│   ├── sif/                    # SIF record schema (FlatBuffers)
│   └── tests/                  # Core integration + security suites
├── cli/                        # dologctl CLI tool
│   └── src/commands/           # Subcommands (config, perf, plugin, record, run, shm, verify)
├── adapters/                   # Language SDKs (C, Rust, Python, Go)
│   └── c/                      # Thin C adapter (dologger_adapter.h)
├── plugins/                    # Plugin ecosystem
│   ├── official/               # Official plugins (formatter_json, formatter_text, filter_level, field_container)
│   │   └── trust-anchors/      # Public signing keys (active.pub) + revocation list (revoked.txt)
│   └── examples/               # Multi-language examples (Rust, C, C++, Go)
├── examples/                   # Minimal host-app examples (C ABI consumers)
├── compliance/                 # GDPR/HIPAA/PCI-DSS compliance templates
├── config/                     # Example configuration (dologger.example.toml)
├── docker/                     # Container images (Dockerfile.dev; runtime in v1.0.0)
├── docs/                       # Technical documentation (EN + zh, auto-synced to the wiki)
│   └── assets/                 # hero.svg, architecture.svg/-zh (mmd source + rendered SVG)
├── tests/                      # Test suites (common/, perf/, smoke/)
├── scripts/                    # Build and setup scripts (local + CI)
├── cmake/                      # CMake helper modules (cross-compile, Conan toolchain)
└── peripheral/                 # Non-product: marketing site + maintainer tools
    ├── site/                   # Vue 3 + TypeScript landing page (GitHub Pages)
    ├── github/                 # GitHub publishing automation
    │   └── scripts/            #   build-site · sync-wiki · generate-release-notes
    └── tools/                  # Maintainer-only aux tools (see peripheral/tools/README.md)
```

Key files at the root: `Cargo.toml` (workspace), `CMakeLists.txt`, `conanfile.py`, `deny.toml`, `rustfmt.toml`, `SECURITY.md`, `LICENSE-APACHE`, `LICENSE-MIT`, `NOTICE`. Example config lives in `config/dologger.example.toml`.

</details>

---

## Building

```bash
# Prerequisites: Rust stable, CMake ≥ 3.20
cargo build --release

# With Kafka support (requires librdkafka)
cargo build --release --features sink-kafka

# Cross-platform targets
cargo check --target x86_64-unknown-linux-gnu
cargo check --target x86_64-apple-darwin
cargo check --target aarch64-apple-darwin
```

> [!WARNING]
> `sink-kafka` requires librdkafka (via Conan or the system package manager). CI and release builds include it on Linux x86_64 only; macOS, Windows and Linux aarch64 builds exclude it.

---

## Some Documentations

| Guide | Content |
|:-:|:-:|
| [Architecture Reference](docs/en_US/ArchitectureReference.md) | Pipeline, ring buffer, audit chain, security model |
| [dologctl Command Reference](docs/en_US/guides/DologctlCommandReference.md) | Every CLI subcommand, option, and exit code |
| [Plugin Development QuickStart](docs/en_US/PluginDevelopmentQuickStart.md) | C/C++/Go plugin development |
| [Plugin Development Guide](docs/en_US/guides/PluginDevelopmentGuide.md) | Rust plugin development |
| [Security Whitepaper](docs/en_US/guides/SecurityWhitepaper.md) | Threat model & cryptographic design |
| [Repository Layout](docs/en_US/guides/RepositoryLayout.md) | Six-zone root map — product vs build-infra vs peripheral |
| [Naming Convention](docs/en_US/guides/NamingConvention.md) | Path-as-namespace, role suffixes, abbreviation rules |
| [Documentation Index](docs/README.md) | All guides, English + 中文 |

---

## Contributing

Contributions are welcome! Bug reports and feature requests use the [issue templates](https://github.com/Nekolio/DoLogger/issues/new/choose); pull requests must satisfy the [PR checklist](.github/pull_request_template.md). Security vulnerabilities are handled privately — see [SECURITY.md](SECURITY.md), do not file them as public issues.

### Contributors

<a href="https://github.com/Nekolio">
  <img src="https://images.weserv.nl/?url=https://github.com/Nekolio.png&w=96&h=96&fit=cover&mask=circle" width="96" height="96" alt="@Nekolio" />
</a>

[@Nekolio](https://github.com/Nekolio) — project author & maintainer

---

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

---

## Star History

<a href="https://star-history.com/#Nekolio/DoLogger&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=Nekolio/DoLogger&type=Date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=Nekolio/DoLogger&type=Date" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=Nekolio/DoLogger&type=Date" />
  </picture>
</a>

---

*Built by [@Nekolio](https://github.com/Nekolio) | nekoliowork+DoLogger@gmail.com*
