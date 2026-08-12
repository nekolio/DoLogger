# DoLogger Host Integration Guide

> 🌐 **语言 / Language**: [English](HostIntegrationGuide.md) | [中文：宿主集成手册](../../zh_CN/guides/HostIntegrationGuide.md)

> **Version**: v0.2.0 | **Last Updated**: 2026-08-12 | **Target Audience**: Host Application Developers
>
> **Purpose**: This document describes how to integrate the DoLogger logging engine into a host application via the C ABI. It covers initialization, log submission, configuration, callbacks, thread safety, language adapters, performance tuning, and troubleshooting.
>
> **Reading Path**: New integrators should start with [Quick Start](#quick-start). Rust-native users should read [Rust Crate Integration](#rust-crate-integration) first. Operations engineers should also consult the [Operations Manual](OperationsManual.md).

## Table of Contents

1. [Overview](#overview)
2. [Quick Start](#quick-start)
3. [C ABI Initialization and Shutdown](#c-abi-initialization-and-shutdown)
4. [Log Submission](#log-submission)
5. [Configuration System](#configuration-system)
6. [Record Field Permission Rings](#record-field-permission-rings)
7. [Error Handling](#error-handling)
8. [Callback Sink Registration](#callback-sink-registration)
9. [Thread Safety Model](#thread-safety-model)
10. [Language Adapters](#language-adapters)
11. [Performance Tuning](#performance-tuning)
12. [Troubleshooting](#troubleshooting)

---

## Overview

DoLogger is a cross-platform, high-security, plugin-architected logging engine. Host applications integrate by dynamically linking `libdologger_core` (`.so` / `.dylib` / `.dll`) and calling the C ABI.

### Key Features

- **Stable C ABI**: All public APIs use `dologger_*` prefixed C functions with a fixed ABI version guarantee.
- **Zero Rust Toolchain Required**: Hosts link a prebuilt dynamic library; no Rust compiler or cargo needed.
- **Plugin Architecture**: 10 plugin VTable types, loaded on demand at engine startup.
- **Ed25519 Signatures with LSN Audit Chain**: Tamper-evident log integrity protection via cryptographic chaining.
- **Three-Color Trust Model**: Blue, Yellow, and Red plugin isolation tiers (see the [Security Whitepaper](SecurityWhitepaper.md)).

### Supported Platforms

| Platform  | Architectures        | Library Suffix |
|:-:|:-:|:-:|
| Linux     | x86\_64, aarch64     | `.so`          |
| macOS     | x86\_64, aarch64     | `.dylib`       |
| Windows   | x86\_64              | `.dll`         |

### API Stability

The C ABI follows semantic versioning. All `dologger_*` symbols are versioned via the `DO_LOG_ABI_VERSION` constant returned by `dologger_get_abi_version()`. Minor version bumps add symbols without removing or reordering existing ones.

---

## Quick Start

### Step 1: Initialize the Engine

```c
#include "dologger_core.h"

int main(void) {
    dologger_handle_t *logger = NULL;
    int rc = dologger_init(NULL, &logger);
    if (rc != DO_LOG_OK) {
        fprintf(stderr, "Failed to initialize DoLogger: %d\n", rc);
        return 1;
    }

    // ... use logger ...

    dologger_shutdown(&logger);
    return 0;
}
```

### Step 2: Submit a Log Record

```c
dologger_record_params_t params = {
    .level           = DO_LOG_INFO,
    .message         = "Hello from host application",
    .source_file     = "main.c",
    .source_function = "main",
    .source_line     = 42,
};
int rc = dologger_log(logger, &params);
if (rc != DO_LOG_OK) {
    // Handle backpressure — record was dropped or queue is full
    dologger_error_info_t err;
    dologger_get_last_error(logger, &err);
    fprintf(stderr, "Log submission failed: %s (code 0x%04x)\n",
            err.message, err.code);
}
```

### Step 3: Link the Library

**Linux / macOS:**
```bash
cc -o myapp myapp.c -ldologger_core -L/usr/lib/dologger
```

**Windows (MSVC):**
```bash
cl /Fe:myapp.exe myapp.c dologger_core.lib
```

Verify the ABI version your binary was compiled against:
```c
uint32_t abi = dologger_get_abi_version();
printf("DoLogger ABI version: %u\n", abi);
```

---

## C ABI Initialization and Shutdown

### `dologger_init()`

```c
int dologger_init(const dologger_init_params_t *params, dologger_handle_t **handle);
```

**Parameters:**

| Parameter | Direction | Description |
|:-:|:-:|:-:|
| `params`  | In        | Initialization parameters. Pass `NULL` for defaults. |
| `handle`  | Out       | Receives the opaque engine handle. Must not be `NULL`. |

**Return Values:**

| Code              | Value | Meaning |
|:-:|:-:|:-:|
| `DO_LOG_OK`       | 0     | Engine initialized successfully. |
| `DO_LOG_ERR_INIT` | -1    | Internal initialization failure. Check `dologger_internal.log`. |
| `DO_LOG_ERR_CFG`  | -2    | Configuration parse error. See error info for details. |

**`dologger_init_params_t` Structure:**

```c
typedef struct {
    const char *config_path;     // Path to TOML config file (NULL = default search)
    const char *config_string;   // Inline TOML config string (overrides file)
    bool        enable_signature; // Force Ed25519 signing on/off
    bool        enable_sysmon;    // Enable the system monitor event stream
    const char *plugin_dir;      // Override plugin search directory
} dologger_init_params_t;
```

### `dologger_shutdown()`

```c
int dologger_shutdown(dologger_handle_t **handle);
```

Performs a graceful shutdown:

1. Stops accepting new log records.
2. Drains all in-flight records from the ring buffer through the pipeline.
3. Calls `plugin_shutdown()` on each loaded plugin in reverse dependency order.
4. Flushes and closes all Sinks.
5. Sets `*handle` to `NULL`.

**Shutdown Policy** is controlled by the `shutdown_policy` configuration key:

| Policy    | Behavior |
|:-:|:-:|
| `graceful` | Wait up to `shutdown_timeout_ms` for the pipeline to drain. Default for `prod-audit`. |
| `immediate` | Drop in-flight records and terminate immediately. Acceptable only for non-audit deployments. |

---

## Log Submission

### `dologger_log()`

```c
int dologger_log(dologger_handle_t *handle, const dologger_record_params_t *params);
```

This is the hot path. The call pushes the record into a lock-free ring buffer and returns immediately. Filtering, field assembly, formatting, signing, and I/O happen asynchronously on background pipeline threads.

### Parameter Structure

```c
typedef struct {
    uint8_t     level;            // DO_LOG_TRACE (0) through DO_LOG_AUDIT (6)
    const char *message;          // UTF-8 encoded log message (required)
    const char *source_file;      // __FILE__ (optional, may be NULL)
    const char *source_function;  // __FUNCTION__ (optional, may be NULL)
    uint32_t    source_line;      // __LINE__ (optional, 0 if unavailable)
    uint32_t    source_column;    // Column number (optional, 0 if unavailable)
    const char *request_id;       // Request / trace correlation ID (optional)
} dologger_record_params_t;
```

### Log Levels

| Constant          | Value | Severity | Description |
|:-:|:-:|:-:|:-:|
| `DO_LOG_TRACE`    | 0     | Trace    | Frame-level diagnostic detail. Use sparingly in production. |
| `DO_LOG_DEBUG`    | 1     | Debug    | Diagnostic information useful during development. |
| `DO_LOG_INFO`     | 2     | Info     | General operational messages (service start, config load). |
| `DO_LOG_WARN`     | 3     | Warning  | Potentially harmful situations (retry, degraded mode). |
| `DO_LOG_ERROR`    | 4     | Error    | Error events that do not halt the application. |
| `DO_LOG_FATAL`    | 5     | Fatal    | Severe errors causing premature termination. |
| `DO_LOG_AUDIT`    | 6     | Audit    | Non-repudiable audit records. May block under backpressure. |

### Convenience Macros

```c
// Standard logging with automatic file/line/function capture
#define DO_LOG_TRACE(h, msg, ...)  dologger_log_fmt(h, DO_LOG_TRACE,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_DEBUG(h, msg, ...)  dologger_log_fmt(h, DO_LOG_DEBUG,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_INFO(h, msg, ...)   dologger_log_fmt(h, DO_LOG_INFO,   __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_WARN(h, msg, ...)   dologger_log_fmt(h, DO_LOG_WARN,   __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_ERROR(h, msg, ...)  dologger_log_fmt(h, DO_LOG_ERROR,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_FATAL(h, msg, ...)  dologger_log_fmt(h, DO_LOG_FATAL,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_AUDIT(h, msg, ...)  dologger_log_fmt(h, DO_LOG_AUDIT,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
```

### AUDIT-Level Backpressure

Records at `DO_LOG_AUDIT` level follow the **Audit Backpressure Iron Law**: under backpressure, the caller may block until the record is durably committed. The default block timeout is 3000 ms. If the timeout expires, the record is written to an emergency spill file (`dologger_emergency_<pid>.buf`) and replayed on recovery. This behavior is non-configurable — it is a [non-downgradable security item](SecurityWhitepaper.md#non-downgradable-items).

---

## Configuration System

### Configuration Priority (Lowest to Highest)

1. Hardcoded defaults (compiled into `libdologger_core`).
2. System-wide configuration (`/etc/dologger/default.toml` on Linux, `%PROGRAMDATA%\dologger\default.toml` on Windows).
3. Project-local configuration (searched from CWD upward through parent directories).
4. Environment variables (`DO_LOG_LEVEL`, `DO_LOG_CONFIG_FILE`, etc.).
5. Runtime API (`dologger_config_load_from_string()`).
6. Per-record metadata tags.
7. Non-downgradable security items (absolute hard limits, cannot be overridden).

### Core Configuration Keys

```toml
[dologger]
# Log level: TRACE, DEBUG, INFO, WARN, ERROR, FATAL, AUDIT
level = "INFO"

# Performance profile: dev | prod-performance | prod-audit | balanced
performance_profile = "prod-performance"

# Ring buffer capacity. MUST be a power of two.
ring_buffer_size = 262144

# Number of records processed per pipeline batch.
batch_size = 256

# Enable Ed25519 cryptographic signatures on audit records.
enable_signature = false

# Shutdown behavior. "graceful" drains in-flight records before exit.
shutdown_policy = "graceful"
shutdown_timeout_ms = 5000
```

### Performance Profiles

| Profile            | `block_timeout_ms` | `drop_strategy` | Signature | Use Case |
|:-:|:-:|:-:|:-:|:-:|
| `dev`              | 100                 | `drop_newest`   | Off       | Local development and debugging |
| `prod-performance` | 3000                | `below_warn`    | Optional  | High-throughput production services |
| `prod-audit`       | 3000                | `below_warn`    | Required  | Compliance-mandated audit logging |
| `balanced`         | 2000                | `oldest`        | Optional  | General-purpose deployments |

### Environment Variables

| Variable              | Overrides             | Example |
|:-:|:-:|:-:|
| `DO_LOG_LEVEL`        | `level`               | `DO_LOG_LEVEL=DEBUG` |
| `DO_LOG_BUF_SIZE`     | `ring_buffer_size`    | `DO_LOG_BUF_SIZE=524288` |
| `DO_LOG_PERF_PROFILE` | `performance_profile` | `DO_LOG_PERF_PROFILE=balanced` |
| `DO_LOG_CONFIG_FILE`  | Config file path      | `DO_LOG_CONFIG_FILE=/opt/myapp/dologger.toml` |
| `DO_LOG_CONFIG_LOCK`  | Lock config at load   | `DO_LOG_CONFIG_LOCK=1` |

### Configuration Hot Reload

The engine polls the configuration file every 1 second (with a 500 ms debounce). When a change is detected, non-security keys are reloaded without restarting the engine:

```bash
# Change the log level at runtime
sed -i 's/level = "INFO"/level = "DEBUG"/' /etc/dologger/default.toml
# Engine picks up the change within ~1.5 seconds
```

Changes are logged via sysmon as `CONFIG_RELOAD` events. Security-tier keys (non-downgradable items) cannot be loosened via hot reload.

### Control Plane Reload

```bash
curl -X POST http://127.0.0.1:9090/reload
```

The control plane endpoint accepts an optional JSON body to validate before applying:
```bash
curl -X POST http://127.0.0.1:9090/reload \
  -H "Content-Type: application/json" \
  -d '{"dry_run": true}'
```

---

## Record Field Permission Rings

DoLogger enforces a four-ring access control model on log record fields. See also the [Security Whitepaper](SecurityWhitepaper.md#record-field-permission-rings) for the security rationale.

| Ring   | Name                | Write Permitted To         | Read Permitted To          | Integrity |
|:-:|:-:|:-:|:-:|:-:|
| Ring 0 | Engine Core         | Core engine only           | Formatter / Sink (read-only) | Ed25519   |
| Ring 1 | System Trusted      | Core + `HostInfoProvider`  | All plugins (read-only)    | Ed25519   |
| Ring 2 | Verified Plugins    | Blue / Yellow plugins      | All plugins                | Ed25519 (configurable) |
| Ring 3 | Untrusted Extensions| Any plugin                 | Any plugin                 | CRC32C    |

### Ring 0 — Immutable Fields

These fields are set once by the core engine and **MUST NOT** be modified by any plugin:

- `record.id` — globally unique record identifier (snowflake algorithm)
- `record.timestamp` — wall-clock time when the record was enqueued
- `record.signature` — Ed25519 signature over Ring 0 + Ring 1 fields
- `record.origin_lsn` — log sequence number assigned at enqueue time

### Ring 1 — Host Context

Fields in this ring are written by the core engine and the `HostInfoProvider` plugin:

- `host.name`, `host.os`, `host.arch`
- `process.id`, `process.name`, `process.thread_id`
- `environment` (production / staging / development)

### Ring 2 — Verified Extensions

Blue and Yellow plugins may write fields with the `verified.*` namespace prefix. Every write appends an `audit_tags` entry containing `{plugin_id, plugin_version, timestamp}`.

### Ring 3 — Untrusted

Red plugins write to the `ext.*` namespace. These fields are protected by CRC32C only and are **excluded** from the Ed25519 signature coverage.

---

## Error Handling

All C ABI functions return `int`. Zero (`DO_LOG_OK`) indicates success; negative values indicate an error.

### Error Code Categories

The error code space uses hexadecimal nibble categorization:

| Range    | Category          | Example |
|:-:|:-:|:-:|
| `0x01xx` | General           | `DO_LOG_ERR_INIT`, `DO_LOG_ERR_INVALID_ARG` |
| `0x02xx` | Configuration     | `DO_LOG_ERR_CFG_PARSE`, `DO_LOG_ERR_CFG_MISSING` |
| `0x03xx` | I/O               | `DO_LOG_ERR_IO_WRITE`, `DO_LOG_ERR_IO_FLUSH` |
| `0x04xx` | Signature/Security| `DO_LOG_ERR_SIG_VERIFY`, `DO_LOG_ERR_SIG_KEY` |
| `0x05xx` | Plugin            | `DO_LOG_ERR_PLUGIN_LOAD`, `DO_LOG_ERR_PLUGIN_ABI` |

### Retrieving Detailed Error Information

```c
typedef struct {
    uint32_t    code;           // Error code (hex nibble format)
    char        message[256];   // Human-readable description
    const char *source_file;    // File where the error originated
    uint32_t    source_line;    // Line where the error originated
    const char *plugin_name;    // Plugin name, if plugin-related
} dologger_error_info_t;

int dologger_get_last_error(dologger_handle_t *handle,
                            dologger_error_info_t *err_info);
```

### Diagnostic Log

Detailed engine diagnostics are written to `dologger_internal.log` (permissions 0600) in the current working directory. This file contains:

- Plugin load / unload events with full symbol resolution traces
- Configuration parse warnings and strict-mode violations
- Sandbox policy enforcement decisions
- Internal assertion failures

Do **not** rely on parsing this file programmatically. Use `dologger_get_last_error()` for machine-readable error details.

---

## Callback Sink Registration

Host applications can register a callback to receive formatted log data in-process, bypassing external Sinks:

```c
typedef void (*dologger_sink_callback_t)(
    const uint8_t *data,       // Formatted output bytes (may not be null-terminated)
    size_t         length,     // Length of formatted data
    void          *user_data   // Opaque user pointer passed at registration
);

int dologger_register_callback_sink(
    dologger_handle_t        *handle,
    dologger_sink_callback_t  callback,
    void                     *user_data
);
```

**Usage Example:**

```c
static void my_callback(const uint8_t *data, size_t len, void *user) {
    FILE *fp = (FILE *)user;
    fwrite(data, 1, len, fp);
    fputc('\n', fp);
}

int main(void) {
    dologger_handle_t *logger = NULL;
    dologger_init(NULL, &logger);

    FILE *fp = fopen("app_output.log", "a");
    dologger_register_callback_sink(logger, my_callback, fp);

    // ... application logic ...

    dologger_shutdown(&logger);
    fclose(fp);
    return 0;
}
```

**Constraints:**

- The callback executes on the pipeline thread. Keep it fast — no blocking I/O, no lock acquisition.
- The `data` buffer is valid only for the duration of the callback. Copy it if you need to retain it.
- A maximum of 8 callback Sinks can be registered per engine instance.

---

## Thread Safety Model

| Component                     | Concurrency Mechanism              |
|:-:|:-:|
| Ring buffer producer side     | Lock-free CAS (single-producer optimization per thread) |
| Ring buffer consumer side     | Single consumer thread per domain  |
| Pipeline worker pool          | Work-stealing thread pool (tokio)  |
| Configuration store           | `Arc<RwLock<Config>>` with copy-on-write snapshot |
| Plugin registry               | `Arc<RwLock<PluginRegistry>>` (cold path only) |
| Error state (`last_error`)    | Thread-local storage               |

### Guarantees

- All `dologger_*` API calls are safe to invoke concurrently from any thread.
- Log submission (`dologger_log`) is signal-safe and reentrant — it may be called from signal handlers (though allocation of rich metadata tags within a signal handler is discouraged).
- Shutdown blocks until all in-flight records are drained (graceful mode) or terminates immediately (immediate mode).

### Known Limitation

The ring buffer does not support true multi-producer lock-free enqueue across threads. Multiple producer threads contend on a single CAS cursor. In practice this is acceptable up to approximately 8 concurrent producer threads. Beyond that, consider using a sharded ring buffer (planned for M4).

---

## Language Adapters

### Rust Crate Integration

```toml
# Cargo.toml
[dependencies]
dologger-core = "0.1"
```

```rust
use dologger_core::{Engine, DologgerConfig, LogLevel, Record};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DologgerConfig::default();
    let mut engine = Engine::init(config)?;

    engine.log(LogLevel::Info, "Hello from Rust host")?;

    engine.shutdown()?;
    Ok(())
}
```

The Rust crate provides a safe, idiomatic wrapper around the C ABI. It handles:

- Automatic handle lifetime management via RAII
- `Display` + `Error` implementations for all error codes
- `serde` support for configuration deserialization
- Builder pattern for `DologgerConfig`

### Python (M4 Milestone)

```python
import dologger

logger = dologger.Logger(config_path="/etc/dologger/default.toml")
logger.info("Hello from Python", extra={"request_id": "abc-123"})
logger.shutdown()
```

The Python adapter uses `ctypes` to load `libdologger_core` and provides a `logging.Handler`-compatible interface.

### Go (M4 Milestone)

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

The Go adapter uses cgo to link against `libdologger_core`.

---

## Performance Tuning

### Key Tuning Parameters

| Parameter            | Default  | Recommendation                      | Impact |
|:-:|:-:|:-:|:-:|
| `ring_buffer_size`   | 262144   | Increase for bursty workloads       | Larger buffer = higher peak throughput. Must be a power of two. |
| `batch_size`         | 256      | 128–512 depending on record size    | Larger batches = higher throughput, higher latency. |
| `enable_signature`   | false    | `false` in dev; `true` in audit prod | Signing adds ~17 us per record (Ed25519). |
| `fsync_on_write`     | false    | `true` for WORM audit sinks         | Forces media durability; I/O latency bound. |

### Benchmarking

```bash
# Run the built-in benchmark suite
cargo bench --bench hot_path

# Profile with perf (Linux)
perf record --call-graph dwarf ./target/release/benchmarks/hot_path
perf report
```

### Representative Performance Data

Hardware: AMD Ryzen 9 7950X, DDR5-6000, Samsung 990 Pro NVMe.

| Scenario                         | Throughput (records/s) | P50 Latency | P99 Latency |
|:-:|:-:|:-:|:-:|
| Console Sink, signature off      | 1,200,000              | 82 ns       | 210 ns      |
| File Sink, signature off         | 950,000                | 105 ns      | 380 ns      |
| File Sink, signature on          | 58,000                 | 17.1 us     | 22.3 us     |
| WORM Sink, signature + fsync     | 12,000                 | 83.4 us     | 140 us      |

### OS-Level Tuning

```bash
# Linux: Pin pipeline threads to isolated CPUs
sudo cset shield --cpu 2-3 --kthread=on

# Increase the max locked memory for the ring buffer (if using huge pages)
sudo sysctl -w vm.max_map_count=262144

# Disable transparent huge pages if measuring latency
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled
```

---

## Troubleshooting

### Common Issues

| Symptom | Likely Cause | Resolution |
|:-:|:-:|:-:|
| `dologger_init()` returns non-zero | Missing config or invalid TOML syntax | Check `dologger_internal.log` (0600 permissions) for the parse error |
| Logs are not appearing in output | Filter plugin dropping records, or Sink circuit breaker is open | Check sysmon events on `stderr` for `SHM_DROP` or `SINK_CIRCUIT_OPEN` |
| Performance below expectations | Wrong performance profile or signature overhead | Run `cargo bench` for baseline; verify `performance_profile` in config |
| Cannot delete log files on Windows | File handles not released | Use `FILE_SHARE_DELETE`; close handles before rotation |
| Ring buffer overflow (emergency file) | Consumer cannot keep up with producer rate | Increase `ring_buffer_size` or switch to `prod-performance` profile |
| `SIGNATURE_FAILURE` in sysmon | Log file tampered with or key mismatch | Run `dologctl verify-log` to identify the tampered record |
| Plugin fails to load | ABI version mismatch or missing dependency | Check `manifest.toml` `abi_version` field; see [Plugin Development Guide](PluginDevelopmentGuide.md) |

### Diagnostic Checklist

1. **Engine health**: `curl http://127.0.0.1:9090/status`
2. **Sysmon events**: Redirect `stderr` and watch for `PIPELINE_BACKLOG`, `SHM_DROP`, `SINK_CIRCUIT_OPEN`, `SANDBOX_VIOLATION`, `SIGNATURE_FAILURE`.
3. **Internal log**: `tail -f dologger_internal.log`
4. **Configuration**: `dologctl config validate --config /path/to/dologger.toml --strict`
5. **Plugin status**: `dologctl plugin list --verbose`

### Collecting a Debug Report

```bash
dologctl diag collect --output diag-report.tar.gz
```

This produces an archive containing:
- `dologger_internal.log`
- Active configuration (with sensitive values redacted)
- Plugin load manifest
- Ring buffer statistics snapshot
- OS resource limits (`ulimit -a` equivalent)

Attach this report when filing a bug at the project repository.
