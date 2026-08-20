# Go Example Filter Plugin for DoLogger

A minimal reference implementation of a DoLogger **Filter** plugin written in Go and compiled to a C shared library via `cgo`.

## What it does

Drops log records whose severity level is **below** a configurable threshold.

- Levels (matching `dologger_core.h`): 0=TRACE, 1=DEBUG, 2=INFO, 3=WARN, 4=ERROR, 5=FATAL, 6=AUDIT
- Default `min_level`: 3 (WARN) -- TRACE, DEBUG, and INFO are dropped.

## Prerequisites

- Go 1.22+ (1.23 recommended)
- `gcc` or `clang` on `PATH` (cgo requires a C compiler)

## Build

```bash
# From this directory (plugins/examples/filter/go/example_filter/)
go build -buildmode=c-shared -o dologger-plugin-filter-go.so main.go
```

This produces `dologger-plugin-filter-go.so` (Linux), `.dylib` (macOS), or `.dll` (Windows).

## Usage

1. Copy `dologger-plugin-filter-go.so` into the DoLogger plugins directory.
2. Configure the plugin in your DoLogger TOML configuration:

```toml
[[pipeline.filter]]
plugin = "dologger-plugin-filter-go"
config = { min_level = 2 }   # 0=TRACE … 6=AUDIT

# Or via JSON config string passed to plugin_init():
# {"min_level": 2}
```

3. Start DoLogger — records below `min_level` will be silently dropped.

## Exported C ABI symbols

| Symbol            | Signature                                         | Purpose                             |
|-------------------|---------------------------------------------------|-------------------------------------|
| `plugin_query`    | `dologger_plugin_info_t *(uint32_t)`              | Returns PluginInfo with VTable      |
| `plugin_init`     | `int(const void *config)`                         | Parses JSON config, stores min_level|
| `plugin_shutdown` | `int(void)`                                       | Resets state, returns 0             |

## PluginInfo layout

Matches `dologger_plugin_info_t` from `core/include/dologger_core.h`:

| Field       | Type        | Value           |
|-------------|-------------|-----------------|
| name        | `const char*`| `"go-example-filter"` |
| version     | `uint32_t`  | `0x000001` (0.0.1) |
| abi_version | `uint32_t`  | `0x000001` (0.0.1) |
| phase       | `uint32_t`  | `0x0002` (DO_LOG_PHASE_FILTER) |
| vtable      | `void*`     | Pointer to `dologger_filter_vtable_t` |

## C ABI contract

- **Filter function** (`filter` in VTable): returns `0` to keep the record, non-zero to drop. Must not perform I/O.
- **plugin_query**: returns a pointer to a valid `dologger_plugin_info_t`. Called once at load time.
- **plugin_init**: receives an opaque config pointer (JSON string). Returns 0 on success.
- **plugin_shutdown**: called before library unload. Returns 0 on success.

## Limitations

- The filter function receives the record level via the `config` pointer rather than reading it from the record handle itself. A production Go plugin should link against the DoLogger core and use the field access API (`dologger_field_get`) to extract the level from the record handle.
- JSON parsing uses Go's `encoding/json` standard library, which is linked into the shared object. This is acceptable for an example but a hardened plugin would use a zero-allocation parser.

## Reference

- Core header: `core/include/dologger_core.h`
- Rust example: `plugins/examples/filter/rust/example_filter/`
