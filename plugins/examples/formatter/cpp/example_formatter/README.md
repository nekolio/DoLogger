# C++ Example Formatter Plugin for DoLogger

A minimal reference implementation of a DoLogger **Formatter** plugin in C++.

## What it does

Formats log records as simple human-readable text lines:

```
[INFO ] (log message)
[WARN ] (log message)
[ERROR] (log message)
```

The format template is configurable via `plugin_init`. A real implementation would read the message and metadata from the record handle; this example uses a placeholder to keep the code minimal.

Phase: `DO_LOG_PHASE_FORMATTING` (0x0010) -- runs after processing and before the sink.

## Prerequisites

- GCC 8+, Clang 10+, or MSVC 2019+
- CMake 3.16+ (optional, for CMake build)

## Build (one-liner, no CMake)

```bash
# Linux
g++ -shared -fPIC -fvisibility=default -std=c++17 \
    -o dologger-plugin-formatter-cpp.so \
    formatter.cpp \
    -I../../../../../core/include

# macOS
g++ -shared -fPIC -fvisibility=default -std=c++17 \
    -o dologger-plugin-formatter-cpp.dylib \
    formatter.cpp \
    -I../../../../../core/include

# Windows (MinGW)
g++ -shared -std=c++17 \
    -o dologger-plugin-formatter-cpp.dll \
    formatter.cpp \
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
[[pipeline.formatter]]
plugin = "cpp-example-formatter"
config = "[{level}] {message}"   # format template string
```

3. Start DoLogger -- records will be formatted as text before reaching the sink.

## Exported C ABI symbols

All three symbols use `extern "C"` linkage to ensure C-compatible symbol names:

| Symbol            | Signature                                              | Purpose                             |
|-------------------|--------------------------------------------------------|-------------------------------------|
| `plugin_query`    | `dologger_plugin_info_t *plugin_query(uint32_t)`       | Returns pointer to static PluginInfo |
| `plugin_init`     | `int plugin_init(const void *config)`                  | Stores format template, returns 0    |
| `plugin_shutdown` | `int plugin_shutdown(void)`                            | Resets state, returns 0             |

## PluginInfo layout

Matches `dologger_plugin_info_t` from `core/include/dologger_core.h`:

| Field       | Type        | Value                           |
|-------------|-------------|---------------------------------|
| name        | `const char*` | `"cpp-example-formatter"`    |
| version     | `uint32_t`  | `0x000001` (0.0.1)              |
| abi_version | `uint32_t`  | `0x000001` (0.0.1)              |
| phase       | `uint32_t`  | `0x0010` (DO_LOG_PHASE_FORMATTING) |
| vtable      | `void*`     | Pointer to `dologger_formatter_vtable_t` |

## C ABI contract

### 1. plugin_query

```c
dologger_plugin_info_t *plugin_query(uint32_t core_abi_version);
```

Called once at load time. Must return a pointer to a `dologger_plugin_info_t` struct containing the plugin's identity and VTable. The returned pointer must remain valid for the lifetime of the plugin. Return `NULL` if the core ABI version is incompatible.

### 2. plugin_init

```c
int plugin_init(const void *config);
```

Called after `plugin_query`. Receives an opaque config pointer (a null-terminated string -- the format template). The plugin stores this template and uses it during formatting. Returns 0 on success.

### 3. plugin_shutdown

```c
int plugin_shutdown(void);
```

Called before library unload. Frees any allocated resources and resets state. Returns 0 on success.

### Formatter VTable contract

The `format` function in the VTable:

```c
int format(const dologger_record_handle_t *rec,
           dologger_output_buffer_t *buf, void *config);
```

- `rec`: Opaque record handle -- use `dologger_field_get()` to read fields.
- `buf`: Pre-allocated output buffer. Write formatted text to `buf->data`, set `buf->len` to the number of bytes written (excluding null terminator). Do NOT exceed `buf->capacity`.
- `config`: Plugin instance config pointer.
- Returns 0 on success, non-zero on error.

**Important:** The core owns the output buffer's memory. The formatter must not free it, reallocate it, or write beyond its capacity.

## Limitations

- This example uses a hard-coded placeholder message `"(log message)"`. A production formatter should call `dologger_field_get()` on the record handle to read the actual message and other fields.
- The format template substitution is minimal ({level} only). A real formatter would support a full template syntax.

## Reference

- Core header: `core/include/dologger_core.h`
- C example: `plugins/examples/filter/c/example_filter/`
- Go example: `plugins/examples/filter/go/example_filter/`
- Rust example: `plugins/examples/filter/rust/example_filter/`
