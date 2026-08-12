# DoLogger Plugin Ecosystem

```
plugins/
├── official/                    ← Standard library (Blue trust, signed)
│   ├── README.md
│   ├── filter_level/            ← Filter: drop records by level
│   ├── fmt_json/                ← Formatter: structured JSON output
│   ├── fmt_text/                ← Formatter: human-readable colored text
│   └── field_container/         ← FieldProvider: container/pod metadata
│
├── examples/                    ← Reference implementations
│   ├── filter/
│   │   └── rust/
│   │       └── example_filter/  ← Filter in Rust (C ABI cdylib)
│   ├── formatter/
│   │   └── rust/                ← Future: formatter examples
│   └── processor/
│       └── rust/                ← Future: processor examples
│
├── community/                   ← Third-party plugin workspace (gitkeep)
│   └── .gitkeep
│
├── index/                       ← Plugin registry consumed by dologctl
│   ├── index.toml
│   └── README.md
│
└── README.md                    ← This file
```

## Directory Convention

| Directory | Purpose | Trust | Signed |
|-----------|---------|-------|--------|
| `official/` | Standard library — DoLogger core team | Blue | Ed25519 (root key) |
| `examples/` | Reference code by `{plugin_type}/{language}/` | N/A | N/A |
| `community/` | Third-party plugin workspace | Varies | Varies |
| `index/` | Machine-readable catalog for `dologctl plugin install` | — | — |

## Naming Convention

All plugin libraries MUST follow: `dologger-plugin-{type}-{name}`

| Rule | Example |
|------|---------|
| Official filter | `dologger-plugin-filter-level` |
| Official formatter | `dologger-plugin-fmt-json` |
| Third-party (vendor prefix) | `dologger-plugin-sink-acme-kafka` |
| Example/reference | `dologger-filter-example` |

- Only lowercase `[a-z0-9-_.]` characters allowed
- Maximum 128 characters
- Vendor prefix required for third-party plugins (e.g., `acme-`)

## Plugin Search Paths

Resolution order (highest to lowest priority):

1. `DO_LOG_PLUGIN_DIR` environment variable (colon/semicolon-separated)
2. `./plugins` (current working directory)
3. Platform system directory:
   - Linux: `/usr/lib/dologger/plugins`
   - macOS: `/usr/local/lib/dologger/plugins`
   - Windows: `%PROGRAMDATA%\dologger\plugins`

## Plugin Crate Checklist

Every plugin crate MUST have:
- `Cargo.toml` with `crate-type = ["cdylib"]` and `license.workspace = true`
- `PluginManifest.toml` with name, version, phase, trust_level, min_core_abi
- C ABI exports: `plugin_query`, `plugin_init`, `plugin_shutdown`
- VTable struct matching the plugin phase
- Unit tests covering the VTable contract

## Quick Links

- [Official Plugins](../Docs/en_US/OfficialPluginRoadmap.md)
- [Plugin Development Guide](../Docs/en_US/guides/PluginDevelopmentGuide.md)
- [Plugin Index](index/index.toml)

---

*All official plugins are Apache-2.0 OR MIT licensed.*
