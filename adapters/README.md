# DoLogger Language Adapters

Thin FFI glue layers that make it easy to use DoLogger from non-Rust
host languages.  Each adapter wraps the C ABI exported by `libdologger_core`
and exposes a idiomatic API for the target language.

## Directory Layout

```
adapters/
  rust/         Rust SDK — ergonomic high-level wrapper
  python/       Python ctypes wrapper
  go/           Go cgo wrapper
```

---

## Rust SDK (`adapters/rust`)

A higher-level Rust crate (`dologger-sdk`) that wraps `dologger_core::Engine`
with a simplified `Logger` API.  Handles record allocation, field population,
and ring-buffer submission internally.

### Usage

```rust
use dologger_sdk::Logger;

fn main() {
    let mut log = Logger::init(None).expect("init");
    log.info("Application started");
    log.warn("Disk usage at 85%");
    log.error("Connection refused");
    log.shutdown();
}
```

### API

| Method | Description |
|--------|-------------|
| `Logger::init(config_path)` | Initialize with optional config file path |
| `Logger::init_with_config(cfg)` | Initialize with a pre-built `DologgerConfig` |
| `logger.trace(msg)` | Log at TRACE level |
| `logger.debug(msg)` | Log at DEBUG level |
| `logger.info(msg)` | Log at INFO level |
| `logger.warn(msg)` | Log at WARN level |
| `logger.error(msg)` | Log at ERROR level |
| `logger.fatal(msg)` | Log at FATAL level |
| `logger.audit(msg)` | Log at AUDIT level (WORM, signed) |
| `logger.log(level, msg)` | Log at a specific `LogLevel` |
| `logger.shutdown()` | Gracefully shut down and drain |

### Build

The Rust SDK is part of the workspace.  Build with:

```bash
cargo build -p dologger-sdk
cargo test  -p dologger-sdk
```

---

## Python Adapter (`adapters/python`)

A pure-Python `ctypes` wrapper around the DoLogger C ABI.  No C extension
compilation required.

### Prerequisites

Build `libdologger_core` first:

```bash
cd core/
cargo build --release
```

### Usage

```python
from dologger import DoLogger

log = DoLogger()               # auto-discover config
log.info("Hello from Python")
log.warn("Disk usage at 85%")
log.shutdown()

# Context manager
with DoLogger("/etc/dologger.toml") as log:
    log.info("Inside context manager")
```

### Library Discovery

The adapter searches for the shared library in this order:

1. `DO_LOGGER_LIB_PATH` environment variable (full path to .so/.dylib/.dll)
2. System default (`libdologger_core.so` / `dologger_core.dll`)

```bash
export DO_LOGGER_LIB_PATH=./core/target/release/libdologger_core.so
python test_dologger.py
```

### API

| Method | Description |
|--------|-------------|
| `DoLogger(config_path=None)` | Initialize the engine |
| `.trace(msg)` | Log at TRACE level |
| `.debug(msg)` | Log at DEBUG level |
| `.info(msg)` | Log at INFO level |
| `.warn(msg)` | Log at WARN level |
| `.error(msg)` | Log at ERROR level |
| `.fatal(msg)` | Log at FATAL level |
| `.audit(msg)` | Log at AUDIT level |
| `.shutdown()` | Graceful shutdown |
| `.version` | Property: core version string |
| `with DoLogger() as log:` | Context manager (auto-shutdown) |

---

## Go Adapter (`adapters/go`)

A Go package using `cgo` to link against `libdologger_core`.

### Prerequisites

Build and install the shared library:

```bash
cd core/
cargo build --release

# Linux
sudo cp target/release/libdologger_core.so /usr/local/lib/
sudo ldconfig

# macOS
cp target/release/libdologger_core.dylib /usr/local/lib/

# Windows (MSYS2/MinGW)
cp target/release/dologger_core.dll /mingw64/bin/
```

### Usage

```go
package main

import "github.com/dologger/adapters/go/dologger"

func main() {
    log, err := dologger.NewLogger("")
    if err != nil {
        panic(err)
    }
    defer log.Shutdown()

    log.Info("Hello from Go")
    log.Warn("Disk usage at 85%")
    log.Error("Connection refused")
}
```

### Custom Library Path

```bash
export CGO_LDFLAGS="-L/path/to/core/target/release -ldologger_core"
go test -v ./...
```

### API

| Function / Method | Description |
|-------------------|-------------|
| `NewLogger(configPath)` | Initialize the engine |
| `(*Logger).Trace(msg)` | Log at TRACE level |
| `(*Logger).Debug(msg)` | Log at DEBUG level |
| `(*Logger).Info(msg)` | Log at INFO level |
| `(*Logger).Warn(msg)` | Log at WARN level |
| `(*Logger).Error(msg)` | Log at ERROR level |
| `(*Logger).Fatal(msg)` | Log at FATAL level |
| `(*Logger).Audit(msg)` | Log at AUDIT level |
| `(*Logger).Shutdown()` | Graceful shutdown |
| `Version()` | Return core version string |

---

## ABI Note

The `dologger_log` C symbol uses a compact parameter struct (matching
`core/src/ffi.rs`) that carries `level`, `message`, and optional source
location.  This is intentional and differs from the richer
`dologger_record_params_t` documented in the public header for future
extensions.  All three adapters use the current stable struct layout.
