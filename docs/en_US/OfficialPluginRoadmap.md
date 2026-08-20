# DoLogger Official Plugins

> **Version**: v0.0.1

> 🌐 **语言 / Language**: [English](OfficialPluginRoadmap.md) | [中文：官方插件](../zh_CN/OfficialPluginRoadmap.md)

DoLogger ships a curated set of official plugins — analogous to a language
standard library — covering the most common logging, formatting, and
observability needs. Third-party plugins extend this foundation for
domain-specific requirements.

**This page is an inventory of what ships in the current release. It is not
a roadmap — nothing here is a commitment to future versions.** It is
updated in the release that adds or changes plugins.

## Plugin Types and Pipeline Position

(illustrative pipeline sketch):

```
PreFilter(0) → Filter(1) → FieldProvider(2) → Assembly(3) → Processing(4) → Formatting(5) → Sink(6)
```

| Stage | Plugin Type | Status in v0.0.1 |
|:-:|:-:|:-:|
| 0 | PolicyProvider | Built into the core: `rate_limiter`, `drop_level` |
| 1 | Filter | Official plugin: `filter_level` |
| 2 | FieldProvider | Built into the core: `host_info`; official plugin: `field_container` |
| 3 | Assembly | Core-only: LSN + Ed25519 signature |
| 4 | Processor | Built into the core: `secret_detector` |
| 5 | Formatter | Official plugins: `formatter_json`, `formatter_text` |
| 6 | Sink (core built-in) | 11 sinks built into the core |
| — | KeyProvider | Not implemented — the core loads signing keys itself |
| — | ConfigProvider | Not implemented |
| — | SyscallBroker | Not implemented |

## Official Plugins

The four official plugins live under `plugins/official/`. They are Cargo
workspace members (`cargo build --workspace` builds them), export the
`plugin_query` / `plugin_init` / `plugin_shutdown` C ABI symbols, and each
ships a `PluginManifest.toml`.

| Plugin | Type | Phase | Description |
|:-:|:-:|:-:|:-:|
| `filter_level` | Filter | Filter (1) | Drops records below a configurable severity level, with per-domain overrides. |
| `formatter_json` | Formatter | Formatting (5) | Serializes `Record` fields to structured JSON. |
| `formatter_text` | Formatter | Formatting (5) | Human-readable text output. |
| `field_container` | FieldProvider | FieldProvider (2) | Injects container metadata: container ID, pod name, namespace, node name (Docker, Kubernetes, podman). |

### filter_level

| Property | Value |
|:-:|:-:|
| Phase | Filter (1) |
| Trust | Blue |
| Config | `min_level` (default `"INFO"`), `drop_trace`, `drop_debug`, per-domain overrides |
| Tests | 17 unit tests |

Drops records below a configurable severity level, with optional per-domain
overrides. Replaces the built-in `DropLevelPolicy` for domain-specific use.

### formatter_json

| Property | Value |
|:-:|:-:|
| Phase | Formatting (5) |
| Trust | Blue |
| Config | Not wired yet — the plugin runs with its default behavior |
| Tests | 9 unit tests |

Serializes a `Record`'s fields (level, message, timestamp, thread, process,
source file/function/line) into a JSON object. Config parsing (`pretty`,
`include_ring3`, `timestamp_format`) is not implemented yet.

### formatter_text

| Property | Value |
|:-:|:-:|
| Phase | Formatting (5) |
| Trust | Blue |
| Config | Not wired yet — the plugin runs with its default behavior |
| Tests | 3 unit tests |

Human-readable text output. Config parsing (`color`, `show_thread`,
`show_timestamp`, `timestamp_format`) is not implemented yet.

### field_container

| Property | Value |
|:-:|:-:|
| Phase | FieldProvider (2) |
| Trust | Blue |
| Config | Not wired yet — the plugin runs with its default behavior (`source: auto`) |
| Tests | 3 unit tests |

Injects container orchestration metadata: container ID (from
`/proc/self/cgroup` or `$CONTAINER_ID`), pod name, namespace, and node name.
Auto-detects Docker, Kubernetes, and podman. Config parsing (`source`) is
not implemented yet.

## Building and Testing

```bash
# Build all official plugins
cargo build --release -p dologger-plugin-filter-level \
                      -p dologger-plugin-formatter-json \
                      -p dologger-plugin-formatter-text \
                      -p dologger-plugin-field-container

# filter_level uses global statics — its tests must run single-threaded
cargo test -p dologger-plugin-filter-level -- --test-threads=1
cargo test -p dologger-plugin-formatter-json
cargo test -p dologger-plugin-formatter-text
cargo test -p dologger-plugin-field-container
```

## Not Implemented Yet

These are deliberately absent from v0.0.1 and have no target version:

- Remote plugin registry (`dologctl plugin search` / `plugin update`) — the
  CLI ships `list`, `install <path>`, `remove`, `verify`, and `scan` only.
- Plugin signing tooling (`dologctl sign`) and root key provisioning.
- KeyProvider, ConfigProvider, and SyscallBroker plugin types.

---

*Last updated: 2026-08-13*
