# DoLogger Quick Start Guide

> **Version**: v0.2.0 | **Last Updated**: 2026-08-12 | **Target Audience**: New Users
>
> **Purpose**: Get DoLogger running in 5 minutes. No prior knowledge assumed.
>
> 🌐 **语言 / Language**: [English](QuickStart.md) | [中文文档索引](../zh_CN/)
>
> **Reading Path**: Read this cover-to-cover, then follow the links to the [Integration Guide](IntegrationGuide.md) when you are ready to embed DoLogger into your application.

---

## Table of Contents

1. [Before You Start](#before-you-start)
2. [5-Minute Setup](#5-minute-setup)
3. [Configuration Walkthrough](#configuration-walkthrough)
4. [Next Steps](#next-steps)

---

## Before You Start

### Prerequisites

| Tool | Minimum Version | Check Command |
|:-:|:-:|:-:|
| Rust | 1.70 | `rustc --version` |
| CMake | 3.20 | `cmake --version` |
| Git | any | `git --version` |

### Platform Support

| Platform | Architectures | Status |
|:-:|:-:|:-:|
| Linux | x86_64, aarch64 | Full support |
| macOS | x86_64, aarch64 | Full support |
| Windows | x86_64 | Full support |

---

## 5-Minute Setup

### Step 1: Clone and Build (60 seconds)

```bash
git clone https://github.com/Nekolio/DoLogger.git
cd dologger
cargo build --release
```

Expected output: `target/release/dologctl` (CLI tool) and `target/release/dologger_core` (engine library).

### Step 2: Generate a Configuration (30 seconds)

```bash
./target/release/dologctl init --template dev
```

This creates `dologger.toml` in your current directory with sensible defaults for development:

```toml
[dologger]
level = "DEBUG"
performance_profile = "dev"
ring_buffer_size = 65536
batch_size = 32
enable_signature = false
```

### Step 3: Start Logging (10 seconds)

```bash
./target/release/dologctl run
```

You should see the engine banner followed by log output:

```text
   ___       __
  / _ \___  / /  ___  ___ ____ ____ ____
 / // / _ \/ /__/ _ \/ _ `/ _ `/ -_) __/
/____/\___/____/\___/\_, /\_, /\__/_/
                    /___//___/

[2026-08-12T14:30:00.123Z] INFO  DoLogger engine started (profile: dev, level: DEBUG)
```

### Step 4: Run the Example (Optional)

To see DoLogger processing real application logs, use the built-in example:

```bash
cargo run --example simple_logger -- --config dologger.toml
```

Output:

```text
[2026-08-12T14:30:01.000Z] INFO  Hello from DoLogger example application
[2026-08-12T14:30:01.001Z] WARN  This is a warning message
[2026-08-12T14:30:01.002Z] ERROR An error occurred: simulated failure
```

### Step 5: Verify the Log File

```bash
cat dologger_output.log
```

The records are written by the File Sink to `dologger_output.log` (one JSON line per record).

---

## Configuration Walkthrough

The five options you will touch most often:

### 1. Log Level

```toml
[dologger]
level = "INFO"          # TRACE | DEBUG | INFO | WARN | ERROR | FATAL
```

Sets the minimum severity written to output. Records below this level are dropped.

### 2. Performance Profile

```toml
[dologger]
performance_profile = "prod-performance"
```

| Profile | Description | When to Use |
|:-:|:-:|:-:|
| `dev` | Small buffers, signing off, fast startup | Local development |
| `balanced` | Moderate throughput, basic protection | General workloads |
| `prod-performance` | Maximum throughput, backpressure control | High-throughput services |
| `prod-audit` | Ed25519 signing on every record, WORM storage | Compliance-mandated auditing |

### 3. Ring Buffer Size

```toml
[dologger]
ring_buffer_size = 262144   # MUST be a power of two (65536, 131072, 262144, 524288)
```

Larger buffers handle bursty workloads better at the cost of memory. Each slot is a record pointer (8 bytes on 64-bit), so 262144 slots use approximately 2 MB.

### 4. Output Sinks

```toml
[sinks.console]
type = "sink_console"
enabled = true

[sinks.file]
type = "sink_file"
enabled = true
path = "/var/log/dologger/app.log"
rotation_interval = "24h"
compression = "zstd"
```

DoLogger has 11 built-in sinks: console, file, callback, Kafka, syslog, webhook, SQLite, WORM, security file, shared memory, and OpenTelemetry. Enable as many as you need--output goes to all enabled sinks simultaneously.

### 5. Plugins

```toml
[plugins.json-formatter]
type = "formatter"
path = "/usr/lib/dologger/plugins/libjson_formatter.so"

[plugins.drop-debug]
type = "filter"
path = "/usr/lib/dologger/plugins/libdrop_debug.so"
```

Plugins extend DoLogger without modifying the engine. See the [Integration Guide](IntegrationGuide.md#plugin-selection-guide) for a complete list with recommendations.

---

## Next Steps

| You want to... | Read this |
|:-:|:-:|
| Embed DoLogger in your C application | [Integration Guide](IntegrationGuide.md) -- C API section |
| Add logging to a Rust project | [Integration Guide](IntegrationGuide.md) -- Rust adapter section |
| Understand how the engine works internally | [Architecture Reference](ArchitectureReference.md) |
| Deploy DoLogger in production | [Operations & Security Guide](OperationsAndSecurity.md) |
| Write a custom plugin | [Plugin Development Guide](guides/PluginDevelopmentGuide.md) |
| Verify audit log integrity | [Operations & Security Guide](OperationsAndSecurity.md#audit-verification) |

### Quick Reference

```bash
# Validate your configuration
dologctl config validate --config dologger.toml --strict

# List loaded plugins
dologctl plugin list

# Check engine health (requires running engine)
curl http://127.0.0.1:9090/status

# Verify Ed25519 audit chains
dologctl verify-log --path /var/lib/dologger/audit/

# Collect diagnostic report
dologctl diag collect --output diag-report.tar.gz
```

### Troubleshooting

| Symptom | Solution |
|:-:|:-:|
| Build fails with "CMake not found" | Install CMake 3.20+: `apt install cmake` / `brew install cmake` |
| `dologctl run` exits immediately | Check `dologger.toml` syntax with `dologctl config validate` |
| No output appears | Verify at least one sink has `enabled = true` |
| Plugin fails to load | Check `dologger_internal.log` for ABI mismatch details |

---

## Complete Specification

For the authoritative design document covering every architecture decision, API, and security property: [DoLogger Core Design Document](~/DoLogger/spec/DoLogger核心设计企划书.md) (Chinese).
