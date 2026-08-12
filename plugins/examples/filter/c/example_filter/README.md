# C Example Filter Plugin for DoLogger

A minimal reference implementation of a DoLogger **Filter** plugin in C.

## What it does

Drops log records whose severity level is **below** a configurable threshold.

- Levels (from `dologger_core.h`): `DO_LOG_TRACE` (0) ... `DO_LOG_AUDIT` (6)
- Default `min_level`: `DO_LOG_WARN` (3) -- TRACE, DEBUG, and INFO are dropped.

## Prerequisites

- GCC, Clang, or MSVC
- CMake 3.16+ (optional, for CMake build)

## Build (one-liner, no CMake)

```bash
# Linux
gcc -shared -fPIC -fvisibility=default -o dologger-plugin-filter-c.so \
    example_filter.c \
    -I../../../../../core/include

# macOS
gcc -shared -fPIC -fvisibility=default -o dologger-plugin-filter-c.dylib \
    example_filter.c \
    -I../../../../../core/include

# Windows (MinGW)
gcc -shared -o dologger-plugin-filter-c.dll \
    example_filter.c \
    -I../../../../../core/include
```

## Build (CMake)

```bash
mkdir build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release
cmake --build .
```

The shared library will be in `build/`.

## Usage

1. Copy the built `.so` / `.dylib` / `.dll` into the DoLogger plugins directory.
2. Configure the plugin in your DoLogger TOML configuration:

```toml
[[pipeline.filter]]
plugin = "c-example-filter"
config = "3"            # min_level as integer string: 0=TRACE … 6=AUDIT
```

3. Start DoLogger -- records below the configured level will be silently dropped.

## Exported C ABI symbols

| Symbol            | Signature                                              | Purpose                             |
|-------------------|--------------------------------------------------------|-------------------------------------|
| `plugin_query`    | `dologger_plugin_info_t *plugin_query(uint32_t)`       | Returns pointer to static PluginInfo |
| `plugin_init`     | `int plugin_init(const void *config)`                  | Parses config integer, stores min_level |
| `plugin_shutdown` | `int plugin_shutdown(void)`                            | Resets state, returns 0             |

## PluginInfo layout

Matches `dologger_plugin_info_t` from `core/include/dologger_core.h`:

| Field       | Type        | Value                      |
|-------------|-------------|----------------------------|
| name        | `const char*` | `"c-example-filter"`    |
| version     | `uint32_t`  | `0x000100` (0.1.0)         |
| abi_version | `uint32_t`  | `0x000100` (0.1.0)         |
| phase       | `uint32_t`  | `0x0002` (DO_LOG_PHASE_FILTER) |
| vtable      | `void*`     | Pointer to `dologger_filter_vtable_t` |

## C ABI contract

This plugin implements the three required C ABI exports:

1. **plugin_query** -- Called once at load time. Returns a `dologger_plugin_info_t*` with the plugin name, version, ABI version, phase, and a pointer to the Filter VTable. The returned pointer must remain valid for the lifetime of the plugin.

2. **plugin_init** -- Called after `plugin_query`. Receives an opaque `config` pointer (a null-terminated string containing the minimum log level as a decimal integer). Returns 0 on success.

3. **plugin_shutdown** -- Called before library unload. Performs cleanup and resets state. Returns 0 on success.

### Filter VTable contract

The `evaluate` function inside the VTable:
- Receives a `dologger_record_handle_t*` and a `void* config` (the record's level as `int*`).
- Returns 1 if the record level is at or above `min_level` (keep), 0 otherwise (drop).
- Must not perform I/O.

**Note:** The core's `dologger_filter_vtable_t::filter` convention returns non-zero to *drop*. This example uses the opposite convention (1 = keep, 0 = drop) for clarity of the reference implementation. A production plugin should match the core's convention exactly.

## Reference

- Core header: `core/include/dologger_core.h`
- Go example: `plugins/examples/filter/go/example_filter/`
- Rust example: `plugins/examples/filter/rust/example_filter/`
