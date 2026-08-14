# DoLogger Plugin Ecosystem

```
plugins/
├── official/                    ← Standard library (Blue trust, signed)
│   ├── README.md
│   ├── bundle/                  ← THE official-plugins library: ONE cdylib
│   │   └── src/lib.rs           ← hosting every official plugin
│   ├── filter_level/            ← Filter: drop records by level (rlib logic)
│   ├── fmt_json/                ← Formatter: structured JSON output (rlib)
│   ├── fmt_text/                ← Formatter: human-readable text (rlib)
│   ├── field_container/         ← FieldProvider: container metadata (rlib)
│   └── trust-anchors/           ← PUBLIC signing keys + revocation list (CRL)
│
├── examples/                    ← Reference implementations
│   ├── filter/
│   │   ├── c/                   ← Filter in C (C ABI cdylib)
│   │   │   └── example_filter/
│   │   ├── go/                  ← Filter in Go (cgo)
│   │   │   └── example_filter/
│   │   └── rust/                ← Filter in Rust (C ABI cdylib)
│   │       └── example_filter/
│   └── formatter/
│       └── cpp/                 ← Formatter in C++ (C ABI cdylib)
│           └── example_formatter/
│
├── community/                   ← Third-party plugin workspace (gitkeep)
│   └── .gitkeep
│
└── README.md                    ← This file
```

## Directory Convention

| Directory | Purpose | Trust | Signed |
|:-:|:-:|:-:|:-:|
| `official/` | Standard library — DoLogger core team | Blue | Ed25519 (plugin signing key, secret in GitHub) |
| `official/trust-anchors/` | Committed public keys (`active.pub`) + revocation list (`revoked.txt`) | — | public material only |
| `examples/` | Reference code by `{plugin_type}/{language}/` | N/A | N/A |
| `community/` | Third-party plugin workspace | Varies | Varies |

## Naming Convention

The four official plugin crates use `dologger-plugin-{type}-{name}` as their
**Cargo package** names, but they are NOT shipped as individual libraries —
they are compiled into ONE bundle cdylib, `dologger-official-plugins`, which
is the only official-plugin artifact per platform.

| Rule | Package |
|------|---------|
| Official filter | `dologger-plugin-filter-level` |
| Official formatter | `dologger-plugin-fmt-json` |
| Third-party (vendor prefix) | `dologger-plugin-filter-acme-sampler` |
| Example/reference | `dologger-filter-example` |

- Only lowercase `[a-z0-9-_.]` characters allowed
- Maximum 128 characters
- Vendor prefix required for third-party plugins (e.g., `acme-`)

### Release Assets

Official plugins ship with every release as ONE bundle asset per OS/arch:

`dologger-official-plugins-{tag}-{os}-{arch}.{ext}`

| Extension | Platform |
|-----------|----------|
| `.so` | Linux |
| `.dll` | Windows |
| `.dylib` | macOS |

Examples: `dologger-official-plugins-v0.1.0-linux-x86_64.so`,
`dologger-official-plugins-v0.1.0-windows-x86_64.dll`.

The bundle hosts every official plugin (fmt-json, fmt-text, filter-level,
field-container); the host registers them all via `plugin_query_multi` (see
`core/src/plugin/manager.rs`). Third-party plugins keep the single-plugin
`plugin_query` contract and ship per-plugin libraries.

The `-{os}-{arch}` tail is the same one the CLI/core assets use, so the site's
asset parser and the release checksums cover the bundle with no extra handling.

### Signing

Every plugin is verified at load time against an Ed25519 **`.sig` sidecar**
(`<library>.sig`, named by appending `.sig` to the full file name). The loader
holds a **multi-anchor trust store** — a set of active public keys plus a
revocation list keyed by SHA-256 key fingerprint — loaded from
`official/trust-anchors/active.pub` and `revoked.txt`. A plugin whose sidecar
verifies against **any** active, non-revoked anchor is loaded as **Blue**; a
signature matching only a revoked anchor is rejected (`SignatureInvalid`, even
in dev mode — the CRL wins). Unsigned plugins are rejected outside dev mode
unless `set_allow_red_plugins(true)` is set. Sign and verify with the CLI:

```console
dologctl plugin keygen signing.key          # prints the public key (anchor)
dologctl plugin sign libfoo.so signing.key  # writes libfoo.so.sig
dologctl plugin verify --trust-store official/trust-anchors
```

The release workflow signs the official bundle when the
`DOLOGGER_PLUGIN_SIGNING_KEY` secret is configured, shipping `.sig` alongside
the asset. The private seed never enters the repo; rotation and emergency
revocation are documented runbooks (see `official/trust-anchors/README.md`).

## Plugin Search Paths

Resolution order (highest to lowest priority):

1. `DO_LOG_PLUGIN_DIR` environment variable (colon/semicolon-separated)
2. `./plugins` (current working directory)
3. Platform system directory:
   - Linux: `/usr/lib/dologger/plugins`
   - macOS: `/usr/local/lib/dologger/plugins`
   - Windows: `%PROGRAMDATA%\dologger\plugins`

## Plugin Crate Checklist

**Official bundle members** (rlib logic only — no C exports of their own):
- `Cargo.toml` with `crate-type = ["rlib"]` and `license.workspace = true`
- `pub static INFO: DologgerPluginInfo` + a `plugin_info()` accessor
- `pub fn init(config)` / `pub fn shutdown()` lifecycle functions
- VTable struct matching the plugin phase
- Unit tests covering the VTable contract and the registry entry

**The official bundle** (`plugins/official/bundle`):
- `crate-type = ["cdylib"]`, `[lib] name = "dologger_official_plugins"`
- Exports `plugin_query_multi` (required) plus `plugin_init`/`plugin_shutdown`
  that fan out to every member
- One static `DologgerPluginInfoList` registering every member crate

**Third-party plugins** (standalone cdylibs):
- `Cargo.toml` with `crate-type = ["cdylib"]` and `license.workspace = true`
- C ABI exports: `plugin_query(uint32_t core_abi_version)`, `plugin_init`,
  `plugin_shutdown`
- VTable struct matching the plugin phase
- Unit tests covering the VTable contract

**Distribution** (any trust tier ≥ Blue target):
- Sign the built library before shipping: `dologctl plugin keygen <key>`
  then `dologctl plugin sign <lib> <key>` (writes `<lib>.sig`)
- Ship the `.sig` sidecar next to the library
- Verify against the committed store before release:
  `dologctl plugin verify --trust-store <trust-anchors-dir>`

## Quick Links

- [Official Plugins](../Docs/en_US/OfficialPluginRoadmap.md)
- [Plugin Development Guide](../Docs/en_US/guides/PluginDevelopmentGuide.md)

---

*All official plugins are Apache-2.0 OR MIT licensed.*
