# DoLogger Official Plugins

The official plugins are a curated set of plugins maintained by the DoLogger
Core Team, analogous to a language standard library. They cover the most
common logging, formatting, security, and observability needs. Third-party
plugins extend this foundation for domain-specific requirements.

See [OfficialPluginRoadmap.md](../../Docs/en_US/OfficialPluginRoadmap.md) for
the current inventory of official plugins. This page intentionally carries
no future roadmap — it documents what exists today.

## Directory Convention

```
plugins/official/
├── README.md                  ← This file
├── fmt_json/                  ← JSON formatter
│   ├── Cargo.toml
│   ├── PluginManifest.toml
│   └── src/lib.rs
├── fmt_text/                  ← Human-readable text formatter
│   ├── Cargo.toml
│   ├── PluginManifest.toml
│   └── src/lib.rs
├── filter_level/              ← Log level filter
│   ├── Cargo.toml
│   ├── PluginManifest.toml
│   └── src/lib.rs
└── field_container/           ← Container metadata injector
    ├── Cargo.toml
    ├── PluginManifest.toml
    └── src/lib.rs
```

Each official plugin crate:
- Is a Cargo workspace member (`plugins/official/<name>/`)
- Exports `plugin_query`, `plugin_init`, `plugin_shutdown` C ABI symbols
- Declares `license.workspace = true` (Apache-2.0 OR MIT)
- Includes a `PluginManifest.toml`
- Has unit tests covering the VTable contract
- Declares Blue trust level (signing infrastructure is not provisioned yet)

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
