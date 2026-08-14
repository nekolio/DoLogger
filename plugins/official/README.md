# DoLogger Official Plugins

The official plugins are a curated set of plugins maintained by the DoLogger
Core Team, analogous to a language standard library. They cover the most
common logging, formatting, security, and observability needs. Third-party
plugins extend this foundation for domain-specific requirements.

See [OfficialPluginRoadmap.md](../../docs/en_US/OfficialPluginRoadmap.md) for
the current inventory of official plugins. This page intentionally carries
no future roadmap — it documents what exists today.

## Directory Convention

```
plugins/official/
├── README.md                  ← This file
├── bundle/                    ← THE official-plugins library: ONE cdylib
│   ├── Cargo.toml             ← package dologger-official-plugins
│   ├── src/lib.rs             ← plugin_query_multi + init/shutdown fan-out
│   └── tests/bundle_cdylib.rs ← dlopen integration test (C-ABI end-to-end)
├── fmt_json/                  ← JSON formatter (rlib logic)
│   ├── Cargo.toml
│   └── src/lib.rs
├── fmt_text/                  ← Human-readable text formatter (rlib)
│   ├── Cargo.toml
│   └── src/lib.rs
├── filter_level/              ← Log level filter (rlib logic)
│   ├── Cargo.toml
│   └── src/lib.rs
├── field_container/           ← Container metadata injector (rlib)
│   ├── Cargo.toml
│   └── src/lib.rs
└── trust-anchors/             ← PUBLIC signing keys + revocation list (CRL)
    ├── active.pub             ← one 64-hex public key per line
    ├── revoked.txt            ← <fingerprint> [reason] [unix-ts]
    └── README.md              ← bootstrap + rotation/revocation runbooks
```

The official plugins are compiled into **ONE** dynamic library —
`dologger-official-plugins` (`libdologger_official_plugins.so` / `.dylib` /
`dologger_official_plugins.dll`) — not one plugin per file. The bundle
exports `plugin_query_multi`, which returns a `DologgerPluginInfoList`
registering every official plugin; the host registers them all from the
single library handle.

Each official plugin **member** crate:
- Is a Cargo workspace member (`plugins/official/<name>/`)
- Is an **rlib** (`crate-type = ["rlib"]`) providing plugin LOGIC only —
  `pub static INFO: DologgerPluginInfo`, `plugin_info()`, `init()`, `shutdown()`
- Declares `license.workspace = true` (Apache-2.0 OR MIT)
- Has unit tests covering the VTable contract and its registry entry
- Carries no `#[no_mangle]` exports — the bundle is the only C ABI surface

The **bundle** crate (`plugins/official/bundle`) is a **cdylib** that links all
four members statically and exposes the C ABI.

## Signing & Trust

Official plugins are Blue-trust: the release workflow signs the built bundle
with the project Ed25519 seed (secret `DOLOGGER_PLUGIN_SIGNING_KEY`, never in
the repo) and ships the `.sig` sidecar next to the asset. At load time the host
verifies `<library>.sig` against the **multi-anchor trust store** committed in
`trust-anchors/` (`active.pub` + `revoked.txt`); a signature that verifies
against any active, non-revoked anchor grants Blue trust, a signature from a
revoked key is rejected even in dev mode (the CRL wins), and unsigned libraries
are rejected outside dev mode. See
[Signature Verification & Trust Anchors](../../docs/en_US/guides/PluginDevelopmentGuide.md#signature-verification--trust-anchors).

Locally, sign and verify with:

```console
dologctl plugin keygen signing.key
dologctl plugin sign target/release/dologger_official_plugins.dll signing.key
dologctl plugin verify --trust-store trust-anchors
```

Protect the seed on your machine: `dologctl plugin wrap-key signing.key
signing.key.enc`, then sign via `--wrapped-key`. Rotation and emergency
revocation runbooks live in
[`trust-anchors/README.md`](trust-anchors/README.md).

## Building

```bash
# Build the official plugins bundle (ONE cdylib hosting every official plugin)
cargo build --release -p dologger-official-plugins

# Build and test a single member's logic
cargo test -p dologger-plugin-fmt-json

# Run the bundle's end-to-end tests (unit + dlopen of the built cdylib)
cargo test -p dologger-official-plugins
```

## License

All official plugins are licensed under Apache-2.0 OR MIT, matching the
workspace root license.
