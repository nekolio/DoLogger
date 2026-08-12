# DoLogger Plugin Index

> Local directory-based plugin index consumed by `dologctl plugin install`.

## Directory layout

```
plugins/index/
  .gitkeep          Ensures the directory is tracked by git
  index.toml        The main plugin index (this is what dologctl reads)
  README.md         This file
```

## What is the plugin index?

The plugin index is a TOML file that `dologctl` reads to discover available
plugins, resolve platform-specific download URLs, verify checksums, and
validate plugin signatures before installation.

The index lives **locally** in the repository (`plugins/index/index.toml`). In
M4+, `dologctl` can be pointed at a remote index URL for fetching the latest
catalogue:

```
dologctl plugin install kafka-sink
dologctl plugin install gdpr-formatter --version 1.0.0
```

## Index schema (`index.toml`)

### Top-level `[index]` section

| Field | Type | Description |
|-------|------|-------------|
| `schema_version` | `u32` | Format version (currently `1`). `dologctl` rejects unknown versions. |
| `description` | `string` | Human-readable label for the index. |
| `generated_at` | `string` | ISO-8601 timestamp of index generation. |
| `min_core_abi` | `hex u32` | Minimum core ABI version for all plugins (packed `major.minor.patch`). |
| `signing.root_public_key` | `hex` | Ed25519 public key (32 bytes) for Blue-plugin signature verification. |

### `[[plugin]]` section (one per release)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | `string` | Yes | Unique plugin identifier (matches `plugin_query` name). |
| `version` | `string` | Yes | Semantic version string (e.g., `"1.2.3"`). |
| `version_encoded` | `hex u32` | Yes | Packed version: `(major << 16) \| (minor << 8) \| patch`. |
| `description` | `string` | Yes | One-line summary. |
| `plugin_type` | `string` | Yes | One of `filter`, `field_provider`, `processor`, `formatter`, `io_sink`, `config_provider`, `key_provider`, `policy_provider`, `host_info_provider`, `syscall_broker`. |
| `phase` | `hex u32` | Yes | Mount-phase bitmask (`DO_LOG_PHASE_*` OR'd together). |
| `trust_level` | `string` | Yes | `"blue"`, `"yellow"`, or `"red"`. |
| `license` | `string` | Yes | SPDX identifier (e.g., `"MIT"`, `"Apache-2.0"`). |
| `homepage` | `string` | No | Project URL or repository link. |
| `keywords` | `[string]` | No | Search tags for `dologctl plugin search`. |
| `min_core_abi` | `hex u32` | No | Per-plugin ABI override (defaults to index-wide value). |

### `[[plugin.platform]]` section

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `os` | `string` | Yes | `"linux"`, `"macos"`, or `"windows"`. |
| `arch` | `string` | Yes | `"x86_64"` or `"aarch64"`. |
| `download_url` | `string` | Yes | URL to the compiled shared library (`.so`/`.dylib`/`.dll`). |
| `sha256` | `hex` | Yes | SHA-256 digest of the library file (64 hex chars). |
| `size_bytes` | `u64` | No | Expected file size for progress reporting. |
| `signature` | `hex` | No | Ed25519 signature over `sha256` (required for Blue/Yellow). |

### `[[plugin.dependency]]` section

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | `string` | Yes | Name of the dependency plugin. |
| `version_min` | `string` | Yes | Minimum semantic version (inclusive). |
| `version_max` | `string` | No | Maximum semantic version (exclusive). |
| `required` | `bool` | No | Whether the dependency is mandatory (defaults to `true`). |

## Phase bitmask reference

| Constant | Bit | Description |
|----------|-----|-------------|
| `DO_LOG_PHASE_PRE_FILTER` | `0x0001` | Rate limiting, drop policy |
| `DO_LOG_PHASE_FILTER` | `0x0002` | Domain-based filtering, custom rules |
| `DO_LOG_PHASE_ASSEMBLY` | `0x0004` | Signature, LSN, prev_hash |
| `DO_LOG_PHASE_PROCESSING` | `0x0008` | Transformation, enrichment |
| `DO_LOG_PHASE_FORMATTING` | `0x0010` | Text/JSON/CSV/SIF encoding |
| `DO_LOG_PHASE_SINK` | `0x0020` | Console/file/network/WORM output |
| `DO_LOG_PHASE_CONFIG` | `0x0040` | Configuration loading/saving |
| `DO_LOG_PHASE_KEY` | `0x0080` | Ed25519 key generation/storage |
| `DO_LOG_PHASE_HOSTINFO` | `0x0100` | System/process metadata injection |
| `DO_LOG_PHASE_SYSCALL` | `0x0200` | Platform syscall interception |

These values are defined in `core/src/phase.rs` and must match the
`dologger_core.h` C header.

## Trust levels

| Level | Colour | Signing requirement | Sandbox |
|-------|--------|---------------------|---------|
| **Blue** | Blue | Ed25519 root-key signature | Full system access |
| **Yellow** | Yellow | Recognised CA or TOFU | Restricted (seccomp/AppContainer) |
| **Red** | Red | None | Maximum isolation (memory + thread + time only) |

See `Docs/en_US/guides/PluginDevelopmentGuide.md` for the full trust model specification.

## Version encoding

Plugin versions are packed into a 32-bit unsigned integer:

```
encoded = (major << 16) | (minor << 8) | patch
```

Examples:
- `0.1.0` -> `0x000100`
- `1.0.0` -> `0x010000`
- `2.15.3` -> `0x020F03`

Core ABI versions use the same encoding. The current core ABI is `0x000100`
(0.1.0), defined as `CORE_ABI_VERSION` in `core/src/plugin_mgr.rs`.

## How to publish a plugin

### 1. Build your plugin

Your plugin must be a C dynamic library (cdylib) exporting at minimum:

- `plugin_query() -> *const PluginInfo`
- `plugin_init(config) -> i32`
- `plugin_shutdown() -> i32`

See `plugins/filters/example_filter/` for a complete Rust example and
`Docs/en_US/guides/PluginDevelopmentGuide.md` for the full C ABI specification.

Build release artifacts for each target platform:

```bash
# Linux x86_64
cargo build --release --target x86_64-unknown-linux-gnu

# macOS x86_64
cargo build --release --target x86_64-apple-darwin

# macOS aarch64 (Apple Silicon)
cargo build --release --target aarch64-apple-darwin

# Windows x86_64
cargo build --release --target x86_64-pc-windows-msvc
```

### 2. Compute SHA-256 checksums

```bash
sha256sum target/*/release/your_plugin.{so,dylib,dll}
```

### 3. Sign the checksum (Blue/Yellow only)

```bash
# Sign the SHA-256 digest with your Ed25519 private key
dologctl sign --key your-key.priv --digest <sha256_hex>
```

### 4. Add an entry to `index.toml`

Copy an existing `[[plugin]]` block and fill in your plugin's metadata,
checksums, download URLs, and signatures.  Open a pull request against the
DoLogger repository.

### 5. Host the binaries

Upload the compiled `.so`/`.dylib`/`.dll` files to the URLs specified in
your `download_url` fields.  The index only stores metadata -- the binaries
themselves are hosted externally.

### Entry requirements

- **Blue** plugins: Must pass code review by the DoLogger maintainers.
  Ed25519 signature from the root key required.
- **Yellow** plugins: Must pass automated sandbox audit. Self-signed or
  TOFU-bound signature required.
- **Red** plugins: Any community plugin. No signature required. Only
  loadable in dev mode (`dologger.toml` → `enable_signature = false`).
  No network, filesystem, or process-spawn access at runtime.

### License compliance

All plugins must use an SPDX Category-A or Category-B license (MIT,
Apache-2.0, BSD, MPL-2.0, LGPL-3.0 with dynamic linking).  Category-C
licenses (GPL, AGPL) and Category-D (BSL, SSPL) are prohibited in the
official index.  See `Docs/en_US/guides/PluginDevelopmentGuide.md` for the full
license matrix and `deny.toml` for the automated enforcement configuration.

## CLI usage examples

```bash
# List all locally installed plugins
dologctl plugin list

# Search the index for a keyword
dologctl plugin search gdpr

# Install a plugin from the index
dologctl plugin install kafka-sink

# Install a specific version
dologctl plugin install gdpr-formatter --version 1.0.0

# Verify an installed plugin (ABI, signature, trust)
dologctl plugin verify kafka-sink

# Security scan installed plugins for suspicious symbols
dologctl plugin scan

# Remove a plugin
dologctl plugin remove example-filter

# Update all installed plugins to their latest index versions
dologctl plugin update
```

## Index caching

`dologctl` caches the index locally in its config directory:

- **Linux**: `~/.config/dologger/plugin-index.cache`
- **macOS**: `~/Library/Application Support/dologger/plugin-index.cache`
- **Windows**: `%APPDATA%\dologger\plugin-index.cache`

Run `dologctl plugin update --refresh-index` to force a re-fetch of the
remote index before checking for plugin updates.
