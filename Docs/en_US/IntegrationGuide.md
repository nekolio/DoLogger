# DoLogger Integration Guide

> **Version**: v0.2.0 | **Last Updated**: 2026-08-12 | **Target Audience**: Application Developers
>
> **Purpose**: Learn how to embed DoLogger into your application. Covers the C API, configuration, domain inheritance, plugin selection, and language adapters. If you are brand new, start with the [Quick Start Guide](QuickStart.md) first.
>
> 🌐 **语言 / Language**: [English](IntegrationGuide.md) | [中文文档索引](../zh_CN/)
>
> **Reading Path**: C developers should read [C API Basics](#c-api-basics) and [Configuration Deep-Dive](#configuration-deep-dive). Rust developers can skip to [Language Adapters](#language-adapters). For the complete C ABI reference with every function signature and error code, see the [Host Integration Guide](guides/HostIntegrationGuide.md).

---

## Table of Contents

1. [Before You Start](#before-you-start)
2. [C API Basics](#c-api-basics)
3. [Configuration Deep-Dive](#configuration-deep-dive)
4. [Domain Inheritance](#domain-inheritance)
5. [Performance Profile Selection Guide](#performance-profile-selection-guide)
6. [Record Field System](#record-field-system)
7. [Plugin Selection Guide](#plugin-selection-guide)
8. [Language Adapters](#language-adapters)
9. [Common Patterns and Recipes](#common-patterns-and-recipes)
10. [Troubleshooting FAQ](#troubleshooting-faq)

---

## Before You Start

### Prerequisites

- DoLogger engine library built and available on your system. See the [Quick Start Guide](QuickStart.md) for build instructions.
- A C compiler (GCC, Clang, or MSVC) for C API integration. Not required for Rust/Python/Go adapters.
- Basic understanding of TOML configuration syntax.

### What You Get

After integration, your application can:
- Submit log records with 102 ns median latency
- Output to 11 different sink types simultaneously
- Maintain a cryptographically verifiable audit trail (Ed25519 + LSN chain)
- Extend logging with sandboxed plugins that cannot compromise your application

### Integration Approaches

| Approach | When to Use | Latency | Isolation |
|:-:|:-:|:-:|:-:|
| **Embedded** (dynamic link) | You control the binary, need minimum latency | Lowest (102 ns P50) | Shared process |
| **Sidecar** (sink_shm) | Polyglot services, fault isolation | Low (~1 us) | Separate process |
| **Daemon** (local socket) | Legacy applications, system-wide logging | Moderate | Separate process |

---

## C API Basics

### Initialize, Log, Shutdown

The minimal integration is three function calls. For complete function signatures and error handling, see the [Host Integration Guide](guides/HostIntegrationGuide.md).

```c
#include "dologger_core.h"

int main(void) {
    dologger_handle_t *logger = NULL;

    // 1. Initialize with default configuration
    int rc = dologger_init(NULL, &logger);
    if (rc != DO_LOG_OK) return 1;

    // 2. Submit a log record
    DO_LOG_INFO(logger, "Application started successfully");

    // 3. Shutdown (drains in-flight records)
    dologger_shutdown(&logger);
    return 0;
}
```

### Convenience Macros

Use these macros to automatically capture `__FILE__`, `__func__`, and `__LINE__`:

```c
DO_LOG_TRACE(h, "Frame-level detail: variable x = %d", x);
DO_LOG_DEBUG(h, "Diagnostic: connection pool size = %d", pool_size);
DO_LOG_INFO(h,  "User %s logged in from %s", username, ip);
DO_LOG_WARN(h,  "Retry %d/3 for upstream service %s", attempt, svc);
DO_LOG_ERROR(h, "Database query failed: %s", db_error);
DO_LOG_FATAL(h, "Unrecoverable error in module %s -- shutting down", module);
DO_LOG_AUDIT(h, "User %s deleted record id=%s -- non-repudiable", user, rec_id);
```

### Log Levels

| Level | Constant | Use For |
|:-:|:-:|:-:|
| TRACE | `DO_LOG_TRACE` | Frame-level detail. Use sparingly in production. |
| DEBUG | `DO_LOG_DEBUG` | Developer diagnostics. |
| INFO | `DO_LOG_INFO` | Normal operational events. |
| WARN | `DO_LOG_WARN` | Potentially harmful situations. |
| ERROR | `DO_LOG_ERROR` | Errors that do not halt the application. |
| FATAL | `DO_LOG_FATAL` | Severe errors causing termination. |
| AUDIT | `DO_LOG_AUDIT` | Non-repudiable audit records. May block under backpressure. |

### Linking

**Linux / macOS:**
```bash
cc -o myapp myapp.c -ldologger_core -L/usr/lib/dologger
```

**Windows (MSVC):**
```bash
cl /Fe:myapp.exe myapp.c dologger_core.lib
```

### Verifying the ABI

```c
uint32_t abi = dologger_get_abi_version();
printf("DoLogger ABI: v%u\n", abi);
```

---

## Configuration Deep-Dive

### How Configuration is Resolved

DoLoader uses a 7-layer priority system. Lower-numbered layers have lower priority:

```
Priority (lowest to highest):

  Layer 1: Hardcoded defaults
     │
  Layer 2: System config  (/etc/dologger/default.toml)
     │
  Layer 3: Drop-in fragments  (/etc/dologger/conf.d/*.toml)
     │
  Layer 4: Project-local config  (./dologger.toml, searched upward)
     │
  Layer 5: Environment variables  (DO_LOG_LEVEL, etc.)
     │
  Layer 6: Runtime API  (dologger_config_load_from_string)
     │
  Layer 7: Per-record metadata tags
     │
     ▼
  Effective Configuration

Non-downgradable items: Layers can only tighten security, never loosen it.
```

### Core Configuration Keys

```toml
[dologger]
# ── Required ────────────────────────────────────────────────
level = "INFO"                          # Minimum log level
performance_profile = "prod-performance" # Performance preset

# ── Performance ─────────────────────────────────────────────
ring_buffer_size = 262144               # Power of two required
batch_size = 256                        # Records per pipeline batch
enable_signature = false                # Ed25519 signing (required for AUDIT)
ring_buffer_coop_helping = true         # Producer helps drain at 90% full

# ── Shutdown ────────────────────────────────────────────────
shutdown_policy = "graceful"            # "graceful" or "immediate"
shutdown_timeout_ms = 5000              # Max wait for drain

# ── Key Management ──────────────────────────────────────────
key_rotation_grace_period_days = 7      # Old keys valid after rotation
```

### Environment Variables

| Variable | Overrides | Example |
|:-:|:-:|:-:|
| `DO_LOG_LEVEL` | `level` | `DO_LOG_LEVEL=DEBUG` |
| `DO_LOG_BUF_SIZE` | `ring_buffer_size` | `DO_LOG_BUF_SIZE=524288` |
| `DO_LOG_PERF_PROFILE` | `performance_profile` | `DO_LOG_PERF_PROFILE=balanced` |
| `DO_LOG_CONFIG_FILE` | Config file path | `DO_LOG_CONFIG_FILE=/opt/app/dologger.toml` |
| `DO_LOG_PLUGIN_DIR` | Plugin directory | `DO_LOG_PLUGIN_DIR=/opt/app/plugins` |
| `DO_LOG_CONFIG_LOCK` | Lock config at load | `DO_LOG_CONFIG_LOCK=1` |
| `DO_LOG_SIGN_KEY` | Signing key path | `DO_LOG_SIGN_KEY=/secure/signing.key` |
| `DO_LOG_VERIFY_KEY` | Verification key | `DO_LOG_VERIFY_KEY=/secure/verify.pub` |

### Sink Configuration

Enable any combination of sinks. All enabled sinks receive every record:

```toml
[sinks.console]
type = "sink_console"
enabled = false                         # Disable in production

[sinks.file]
type = "sink_file"
enabled = true
path = "/var/log/dologger/app.log"
max_size = "100MB"
rotation_interval = "24h"
compression = "zstd"                    # gzip | zstd | none
retention_days = 90
retention_total_size = "10GB"

[sinks.kafka]
type = "sink_kafka"
enabled = false
brokers = ["kafka1:9092", "kafka2:9092", "kafka3:9092"]
topic = "app-logs"
tls = true
sasl_mechanism = "SCRAM-SHA-256"

[sinks.syslog]
type = "sink_syslog"
enabled = false
server = "syslog.internal:6514"
protocol = "tcp"
tls = true

[sinks.webhook]
type = "sink_webhook"
enabled = false
url = "https://logs.internal/api/v1/ingest"
bearer_token = "tok_abc123"
```

### Validation

Always validate configuration before deploying:

```bash
# Strict validation
dologctl config validate --config dologger.toml --strict

# With compliance check
dologctl config validate --config dologger.toml --compliance gdpr

# Show effective configuration after all layers merge
dologctl config show --effective
```

---

## Domain Inheritance

### Concept

Domains let you define separate logging configurations for different subsystems of your application. Child domains inherit from parents and can only tighten security settings.

### ASCII Art

```
                    ┌──────────────────────┐
                    │     root domain      │
                    │                      │
                    │ level       = INFO   │
                    │ profile     = prod   │
                    │ sign        = false  │
                    │ sinks       = [file] │
                    └──────────┬───────────┘
                               │ inherits from
               ┌───────────────┼───────────────┐
               │                               │
               ▼                               ▼
    ┌──────────────────────┐     ┌──────────────────────┐
    │   app:security_audit │     │    app:api_service   │
    │                      │     │                      │
    │ inherits: root       │     │ inherits: root       │
    │ level       = DEBUG  │     │ level       = WARN   │
    │ sign        = true   │     │ sinks       = [kafka]│
    │ profile     = audit  │     │  (append to parent)  │
    │ sinks       = [worm] │     │                      │
    └──────────────────────┘     └──────────────────────┘
         AUDIT domain                    API service domain
    (Ed25519 signed, WORM)         (WARN+, Kafka output)
```

### Configuration Example

```toml
# Root domain -- provides defaults for all children
[dologger]
level = "INFO"
performance_profile = "prod-performance"
ring_buffer_size = 262144

[domains]

# Security audit -- independent audit trail
[domains.security_audit]
inherits = "root"
level = "DEBUG"
enable_signature = true                 # Non-downgradable: cannot be loosened
worm_enabled = true                     # Non-downgradable
performance_profile = "prod-audit"
sinks = ["worm_file", "security_file"]
array_merge_policy = "replace"          # Replace parent's sinks entirely

# API service -- Kafka output
[domains.api_service]
inherits = "root"
level = "WARN"                          # Only WARN and above
sinks = ["kafka_prod"]
array_merge_policy = "unique_append"    # Add to parent's sinks (no duplicates)
```

### Array Merge Policies

| Policy | Behavior |
|:-:|:-:|
| `replace` | Child's array completely replaces parent's |
| `append` | Child's items are appended (may duplicate) |
| `unique_append` | Child's items added only if not already present (default) |

### Non-Downgradable Enforcement

Six security items can only be tightened by child domains. Attempting to loosen them triggers a `CONFIG_RELOAD_DENIED` event:

| Item | Tightening | Loosening (REJECTED) |
|:-:|:-:|:-:|
| `enable_signature` | `false` to `true` | `true` to `false` |
| `escape_html` | `false` to `true` | `true` to `false` |
| `worm_enabled` | `false` to `true` | `true` to `false` |
| `fsync_on_write` | `false` to `true` | `true` to `false` |
| `require_tls` | `false` to `true` | `true` to `false` |
| `sign_ring2` | `false` to `true` | `true` to `false` |

---

## Performance Profile Selection Guide

### Profile Comparison

| Property | `dev` | `balanced` | `prod-performance` | `prod-audit` |
|:-:|:-:|:-:|:-:|:-:|
| Block timeout | 100 ms | 2000 ms | 3000 ms | 3000 ms |
| Drop strategy | `drop_newest` | `oldest` | `below_warn` | `below_warn` |
| Ed25519 signing | Off | Optional | Optional | **Required** |
| WORM | Off | Optional | Optional | **Required** |
| Batch size | 32 | 128 | 256 | 128 |
| Ring buffer size | 65536 | 131072 | 262144 | 262144 |
| `escape_html` | Optional | On | On | **On** |
| `fsync_on_write` | Off | Off | Optional | **On** |
| `require_tls` | Off | Warn-only | On | **On** |

### Decision Flowchart

```
Does this deployment require regulatory compliance (GDPR/HIPAA/PCI)?
  ├── YES  → prod-audit  (Ed25519 + WORM + fsync on all records)
  └── NO   → Is this a development machine?
              ├── YES  → dev  (fast startup, small buffers)
              └── NO   → Is raw throughput the top priority?
                          ├── YES  → prod-performance  (up to 13.3M rec/s)
                          └── NO   → balanced  (good default for most workloads)
```

### Performance Data

Measured on AMD Ryzen 9 7950X, DDR5-6000, Samsung 990 Pro NVMe:

| Scenario | Throughput | P50 Latency | P99 Latency |
|:-:|:-:|:-:|:-:|
| Console Sink, no signing | 1,200,000 rec/s | 82 ns | 210 ns |
| File Sink, no signing | 950,000 rec/s | 105 ns | 380 ns |
| File Sink, Ed25519 signing | 58,000 rec/s | 17.1 us | 22.3 us |
| WORM Sink, sign + fsync | 12,000 rec/s | 83.4 us | 140 us |

---

## Record Field System

### The Four Permission Rings

Every log record contains fields organized into four permission rings, modeled after CPU privilege levels:

```
                     ┌─────────────────┐
                     │     Ring 3      │
                     │  (ext.* fields) │
                     │  CRC32C only    │
                     │  Red plugins OK │
                     │  ┌────────────┐ │
                     │  │   Ring 2   │ │
                     │  │(verified.*)│ │
                     │  │Blue/Yellow │ │
                     │  │Ed25519(opt)│ │
                     │  │ ┌─────────┐│ │
                     │  │ │ Ring 1  ││ │
                     │  │ │System   ││ │
                     │  │ │Fields   ││ │
                     │  │ │+HostInfo││ │
                     │  │ │Ed25519  ││ │
                     │  │ │ ┌──────┐││ │
                     │  │ │ │Ring 0│││ │
                     │  │ │ │Core  │││ │
                     │  │ │ │Immut.│││ │
                     │  │ │ └──────┘││ │
                     │  │ └─────────┘│ │
                     │  └────────────┘ │
                     └─────────────────┘
```

### Ring 0 -- Immutable Core Fields

Set once by the engine. No plugin can modify them.

| Field | Type | Description |
|:-:|:-:|:-:|
| `record.id` | uint64 | Unique snowflake ID |
| `record.timestamp` | uint64 | Nanosecond-accurate UTC timestamp |
| `record.signature` | bytes[64] | Ed25519 signature over Ring 0+1 fields |
| `record.origin_lsn` | uint64 | Log Sequence Number |

### Ring 1 -- System Context

Written by the engine and `HostInfoProvider` plugin. Read-only for all other plugins.

| Field | Source |
|:-:|:-:|
| `level`, `message` | Application via C API |
| `source_file`, `source_function`, `source_line` | Application via macros |
| `thread_id`, `thread_name`, `process_id`, `process_name` | Engine |
| `host_name`, `container_id` | HostInfoProvider |
| `app_name`, `app_version` | Application via init params |
| `environment` | Config or env var (`production`/`staging`/`development`) |

### Ring 2 -- Verified Extensions

Blue and Yellow plugins write to the `verified.*` namespace. Every write appends an `audit_tags` entry recording `{plugin_id, plugin_version, timestamp, field}`. This creates a tamper-evident history of field modifications.

```json
{
  "verified.user_id": "u-12345",
  "verified.session_id": "sess-abcdef",
  "audit_tags": [
    {
      "plugin_id": "auth-field-provider",
      "plugin_version": "2.1.0",
      "timestamp": "2026-08-12T14:30:00.123Z",
      "action": "write",
      "field": "verified.user_id"
    }
  ]
}
```

### Ring 3 -- Untrusted Extensions

Red plugins write to `ext.*`. These fields have CRC32C only (hardware-accelerated integrity check, not cryptographic). They are not covered by the Ed25519 signature.

### Using Fields from Your Application

```c
// Write a Ring 1 field (via HostInfoProvider or env)
// Automatically populated -- no code needed

// Write a Ring 2 field (via a FieldProvider plugin)
// Load the field_container plugin in your config and configure its keys

// Write a Ring 3 field (via dologger_record_set_field)
dologger_record_set_field(record, "ext.my_key", "my_value");
```

---

## Plugin Selection Guide

### Plugin Types and Pipeline Position

```
PreFilter ──▶ Filter ──▶ FieldProvider ──▶ Assembly ──▶ Processing ──▶ Formatting ──▶ Sink
   (0)          (1)          (2)             (3)          (4)           (5)          (6)
```

### Which Plugins Do You Need?

| If you need to... | Use this plugin type | Official Plugin |
|:-:|:-:|:-:|
| Control which records are kept | `Filter` | `filter_level`, `filter_sampling` (planned) |
| Add metadata to every record | `FieldProvider` | `field_container`, `field_cloud` (planned) |
| Transform or redact content | `Processor` | `proc_pii_mask` (planned) |
| Change output format | `Formatter` | `fmt_json`, `fmt_text` |
| Write to a different destination | `IOSink` | 11 built-in sinks |
| Use external signing keys | `KeyProvider` | `key_file`, `key_hsm` (planned) |
| Enforce rate limits | `PolicyProvider` | Built-in rate limiter |

### Recommended Plugin Set by Use Case

**Development:**
```
fmt_text (human-readable colored output) + filter_level (drop DEBUG/TRACE in noisy modules)
```

**Production (throughput-first):**
```
fmt_json (machine-parseable) + field_container (container metadata)
```

**Production (compliance):**
```
fmt_json + field_container + proc_pii_mask (mask PII before disk)
```

**Audit/Compliance:**
```
key_file (persistent signing key) + fmt_json + proc_pii_mask
```

### Plugin Trust Colors

| Color | Signed | Syscall Access | File I/O | Network | Process Spawn |
|:-:|:-:|:-:|:-:|:-:|:-:|
| **Blue** | Ed25519 required | Full | Full | Full | Allowed |
| **Yellow** | Recommended | Restricted | Read+Write | Denied | Denied |
| **Red** | Not required | Maximum isolation | Denied | Denied | Denied |

Red plugins are disabled by default. Enable with `allow_red_plugins = true`.

---

## Language Adapters

### Rust

```toml
# Cargo.toml
[dependencies]
dologger-core = "0.1"
```

```rust
use dologger_core::{Engine, DologgerConfig, LogLevel};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DologgerConfig::default();
    let mut engine = Engine::init(config)?;

    engine.log(LogLevel::Info, "Hello from Rust host")?;

    engine.shutdown()?;
    Ok(())
}
```

The Rust crate provides RAII handle management, `Display` + `Error` for all error codes, `serde` support, and a builder pattern for configuration.

### Python

```python
import dologger

logger = dologger.Logger(config_path="/etc/dologger/default.toml")
logger.info("Hello from Python", extra={"request_id": "abc-123"})
logger.shutdown()
```

Uses `ctypes` to load `libdologger_core`. Compatible with the Python `logging.Handler` interface.

### Go

```go
package main

import "github.com/Nekolio/DoLogger-go"

func main() {
    logger, err := dologger.New(dologger.Config{
        Level:   "INFO",
        Profile: "prod-performance",
    })
    if err != nil {
        panic(err)
    }
    defer logger.Shutdown()

    logger.Info("Hello from Go")
}
```

Uses cgo to link against `libdologger_core`.

### C (Direct ABI)

For the complete C ABI reference including every function signature, error code, and callback type, see the [Host Integration Guide](guides/HostIntegrationGuide.md).

---

## Common Patterns and Recipes

### Pattern 1: Development vs Production Config

Use environment variable overrides for dev/prod switching:

```bash
# Development
DO_LOG_LEVEL=DEBUG DO_LOG_PERF_PROFILE=dev ./myapp

# Production
DO_LOG_LEVEL=INFO DO_LOG_PERF_PROFILE=prod-performance ./myapp
```

### Pattern 2: Correlation IDs

Pass request/trace IDs through the `request_id` field for distributed tracing:

```c
dologger_record_params_t params = {
    .level     = DO_LOG_INFO,
    .message   = "Order processed",
    .request_id = trace_id,   // from OpenTelemetry / W3C trace context
};
dologger_log(logger, &params);
```

### Pattern 3: Conditional Logging

Filter out expensive debug computation when the level would drop it:

```c
if (dologger_would_log(logger, DO_LOG_DEBUG)) {
    char *expensive = compute_diagnostic_state();
    DO_LOG_DEBUG(logger, "Diagnostic: %s", expensive);
    free(expensive);
}
```

### Pattern 4: Graceful Shutdown with Signal Handling

```c
#include <signal.h>

static dologger_handle_t *g_logger = NULL;

static void handle_signal(int sig) {
    if (sig == SIGTERM || sig == SIGINT) {
        DO_LOG_INFO(g_logger, "Received signal %d, shutting down", sig);
        dologger_shutdown(&g_logger);
        exit(0);
    }
}

int main(void) {
    dologger_init(NULL, &g_logger);
    signal(SIGTERM, handle_signal);
    signal(SIGINT, handle_signal);

    // ... application loop ...

    dologger_shutdown(&g_logger);
    return 0;
}
```

### Pattern 5: Callback Sink for In-Process Processing

Register a callback to receive formatted log records directly in-process:

```c
static void my_callback(const uint8_t *data, size_t len, void *user) {
    // data points to formatted output (JSON, text, etc.)
    // len is the byte count
    // user is your opaque pointer
    send_to_my_monitoring_system(data, len);
}

int main(void) {
    dologger_handle_t *logger = NULL;
    dologger_init(NULL, &logger);

    dologger_register_callback_sink(logger, my_callback, NULL);

    // ... application logic ...
    dologger_shutdown(&logger);
}
```

Keep callbacks fast -- they execute on the pipeline thread. No blocking I/O.

### Pattern 6: Hot Reload Without Restart

Change the log level at runtime to debug issues in production:

```bash
# Increase verbosity temporarily
curl -X POST http://127.0.0.1:9090/level \
  -H "Content-Type: application/json" \
  -d '{"level": "DEBUG"}'

# Restore after debugging
curl -X POST http://127.0.0.1:9090/level \
  -H "Content-Type: application/json" \
  -d '{"level": "INFO"}'
```

---

## Troubleshooting FAQ

### Engine fails to initialize

**Symptom:** `dologger_init()` returns non-zero.

**Checklist:**
1. Verify `dologger.toml` syntax: `dologctl config validate --config dologger.toml --strict`
2. Check `dologger_internal.log` for parse errors
3. Verify the plugin directory exists and contains valid `.so`/`.dylib`/`.dll` files
4. Ensure `ring_buffer_size` is a power of two

### Logs are not appearing in output

**Symptom:** The engine starts but no log output appears.

**Checklist:**
1. Verify at least one sink has `enabled = true`
2. Check that the log level is not filtering everything: `DO_LOG_LEVEL=TRACE`
3. Look for `SINK_CIRCUIT_OPEN` or `SHM_DROP` in sysmon events
4. Verify file permissions on the output path
5. Check that no Filter plugin is dropping records silently

### Performance is below expectations

**Symptom:** Throughput is lower than benchmark numbers.

**Checklist:**
1. Verify `performance_profile` -- a `dev` profile uses small buffers and batches
2. Check if `enable_signature = true` -- Ed25519 signing adds ~17 us per record
3. Run `curl http://127.0.0.1:9090/status | jq .pipeline` to check drop rate
4. Run `cargo bench` to baseline the engine on your hardware
5. Check if `fsync_on_write = true` -- forces I/O flush on every record

### Ring buffer overflow

**Symptom:** Emergency spill files appearing (`dologger_emergency_*.buf`).

**Causes and Fixes:**
- Consumer thread is falling behind -- increase `ring_buffer_size`
- A slow downstream sink is causing backpressure -- check sink health
- Disk I/O is saturated -- move file sinks to a faster device
- Switch to `prod-performance` profile for larger buffers and better drop strategy

### Plugin fails to load

**Symptom:** Diagnostic log shows `[PLUGIN] load failed`.

**Checklist:**
1. ABI version mismatch: compare `plugin_query()->abi_version` with `DO_LOG_ABI_VERSION`
2. Missing dependency: check `manifest.toml` `[dependencies]` section
3. Blue plugin signature: verify the `.sig` file is present and valid
4. License incompatibility: the plugin's SPDX identifier may be in a denied category
5. Red plugin without `allow_red_plugins = true` in config

### Cannot delete log files on Windows

Windows holds file handles after rotation. Configure the file sink to use `FILE_SHARE_DELETE` and close handles before rotation. If files are locked, stop the engine briefly:

```bash
dologctl stop
# Delete or rotate files
dologctl start
```

### Collecting a Debug Report

```bash
dologctl diag collect --output diag-report.tar.gz
```

This creates an archive with the internal log, active configuration (redacted), plugin manifest, ring buffer statistics, and OS resource limits. Attach it when filing a bug report.

---

## Complete Specification

For the authoritative design document covering every architecture decision, API, and security property: [DoLogger Core Design Document](~/DoLogger/spec/DoLogger核心设计企划书.md) (Chinese).

For the full C ABI reference with every function signature, struct definition, and error code: [Host Integration Guide](guides/HostIntegrationGuide.md).
