# DoLogger Official Plugin Program

The official plugins are a curated set of plugins maintained by the DoLogger
Core Team, analogous to a language standard library. They cover the most common
logging, formatting, security, and observability needs. Third-party plugins
extend this foundation for domain-specific requirements.

See [OfficialPluginRoadmap.md](../../Docs/en_US/OfficialPluginRoadmap.md) for
the full roadmap, development strategy, and Tier 2/3 plans.

## Directory Convention

```
plugins/official/
├── README.md                  ← This file
├── fmt_json/                  ← Tier 1: JSON formatter
│   ├── Cargo.toml
│   ├── PluginManifest.toml
│   └── src/lib.rs
├── fmt_text/                  ← Tier 1: Human-readable text formatter
│   ├── Cargo.toml
│   ├── PluginManifest.toml
│   └── src/lib.rs
├── filter_level/              ← Tier 1: Log level filter
│   ├── Cargo.toml
│   ├── PluginManifest.toml
│   └── src/lib.rs
└── field_container/           ← Tier 1: Container metadata injector
    ├── Cargo.toml
    ├── PluginManifest.toml
    └── src/lib.rs
```

Each official plugin crate:
- Is a Cargo workspace member (`plugins/official/<name>/`)
- Exports `plugin_query`, `plugin_init`, `plugin_shutdown` C ABI symbols
- Declares `license.workspace = true` (Apache-2.0 OR MIT)
- Includes a `PluginManifest.toml` for the plugin index
- Has unit tests covering the VTable contract
- Targets Blue trust level (to be signed with the DoLogger root Ed25519 key)

## Tier 1 — Essential (v0.2.0 target)

These four plugins cover the baseline needs of every production deployment.

| Plugin | Type | Phase | Description |
|--------|------|-------|-------------|
| `filter_level` | Filter | Filter (1) | Drop records below a configurable log level with per-domain override support. Replaces the built-in `DropLevelPolicy` for domain-specific use. |
| `fmt_json` | Formatter | Formatting (5) | Serialize Record fields to structured JSON with configurable field inclusion (`pretty`, `include_ring3`, `timestamp_format`). Universal interchange format for ELK, Loki, Datadog. |
| `fmt_text` | Formatter | Formatting (5) | Human-readable colored text output with configurable field columns (`color`, `show_thread`, `show_timestamp`, `timestamp_format`). Moves ConsoleSink formatting into a swappable plugin. |
| `field_container` | FieldProvider | Field (2) | Inject container orchestration metadata: container ID (from `/proc/self/cgroup` or `$CONTAINER_ID`), pod name, namespace, node name. Auto-detects Docker, Kubernetes, and podman. |

## Building

```bash
# Build all official plugins
cargo build --release -p dologger-plugin-fmt-json \
                      -p dologger-plugin-fmt-text \
                      -p dologger-plugin-filter-level \
                      -p dologger-plugin-field-container

# Build and test a single plugin
cargo test -p dologger-plugin-fmt-json
```

## License

All official plugins are licensed under Apache-2.0 OR MIT, matching the
workspace root license.
