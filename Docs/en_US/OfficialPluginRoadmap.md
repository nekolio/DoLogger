# DoLogger Official Plugin Roadmap

> 🌐 **语言 / Language**: [English](OfficialPluginRoadmap.md) | [中文文档索引](../zh_CN/)

The DoLogger engine ships with a curated set of official plugins — analogous to
a language standard library — that cover the most common logging, formatting,
security, and observability needs.  Third-party plugins extend this foundation
for domain-specific requirements.

## Plugin Types and Pipeline Position

(illustrative pipeline sketch):

```
PreFilter(0) → Filter(1) → FieldProvider(2) → Assembly(3) → Processing(4) → Formatting(5) → Sink(6)
```

| Stage | Plugin Type | Official Count | Status |
|:-:|:-:|:-:|:-:|
| 0 | PolicyProvider | 2 | Built-in (rate_limiter, drop_level) |
| 1 | Filter | 3 | Planned |
| 2 | FieldProvider | 3 | 1 partial (host_info built-in) |
| 3 | Assembly | 0 | Core-only (LSN + Ed25519 sign) |
| 4 | Processor | 3 | 1 done (secret_detector) |
| 5 | Formatter | 3 | Planned |
| 6 | IOSink | 11 | All built-in |
| — | KeyProvider | 2 | Planned |
| — | ConfigProvider | 0 | Deferred (remote config centers) |
| — | SyscallBroker | 0 | Deferred (platform-specific) |

---

## Tier 1 — Essential (v0.2.0 target)

These plugins cover the baseline needs of every production deployment.

### Filter: `filter_level`

| Property | Value |
|:-:|:-:|
| Phase | Filter (1) |
| Trust | Blue |
| Description | Drop records below a configurable log level (per-domain override). |
| Config | `min_level: "INFO"`, `drop_below: true` |
| Rationale | Decouples log-level filtering from the core engine; allows per-domain rules without touching the global rate limiter. Replaces the built-in `DropLevelPolicy` for domain-specific use. |

### Formatter: `fmt_json`

| Property | Value |
|:-:|:-:|
| Phase | Formatting (5) |
| Trust | Blue |
| Description | Serialize Record fields to structured JSON with configurable field inclusion. |
| Config | `pretty: false`, `include_ring3: false`, `timestamp_format: "rfc3339"` |
| Rationale | JSON is the universal interchange format for log aggregation systems (ELK, Loki, Datadog). Every deployment needs this. |

### Formatter: `fmt_text`

| Property | Value |
|:-:|:-:|
| Phase | Formatting (5) |
| Trust | Blue |
| Description | Human-readable colored text output with configurable field columns (matches ConsoleSink format but as a loadable plugin). |
| Config | `color: true`, `show_thread: true`, `show_timestamp: true`, `timestamp_format: "elapsed"` |
| Rationale | Development and debugging. Moves the ConsoleSink formatting logic into a swappable plugin so other sinks can reuse it. |

### FieldProvider: `field_container`

| Property | Value |
|:-:|:-:|
| Phase | FieldProvider (2) |
| Trust | Blue |
| Description | Inject container orchestration metadata: container ID (from `/proc/self/cgroup` or `$CONTAINER_ID`), pod name, namespace, node name (from Kubernetes downward API). |
| Config | `source: "auto"` (auto-detect Docker/Kubernetes/podman) |
| Rationale | In 2026, the majority of production workloads run in containers. Automatic container context injection is table stakes. |

---

## Tier 2 — Production (v0.3.0 target)

These plugins address security, compliance, and operational requirements.

### Processor: `proc_pii_mask`

| Property | Value |
|:-:|:-:|
| Phase | Processing (4) |
| Trust | Blue |
| Description | Mask/replace PII patterns in log messages before they reach any sink. |
| Patterns | Email addresses, credit card numbers (Luhn check), SSN (US), phone numbers (E.164), IBAN (EU), IP addresses (optional) |
| Config | `mode: "mask"` (replace middle chars) or `mode: "hash"` (SHA-256 pseudonym), `custom_patterns: []` |
| Rationale | GDPR/CCPA/HIPAA compliance gate. Run BEFORE formatting so masked data never hits disk or network. Complements the existing `secret_detector` (which handles API keys/tokens). |

### Processor: `proc_field_enrich`

| Property | Value |
|:-:|:-:|
| Phase | Processing (4) |
| Trust | Blue |
| Description | Add user-defined static or computed key-value fields to every record passing through the pipeline. |
| Config | `fields: { "datacenter": "us-east-1", "team": "payments" }`, `env_inherit: ["DEPLOY_VERSION", "REGION"]` |
| Rationale | Common operational need — tagging records with deployment metadata without changing application code. |

### FieldProvider: `field_cloud`

| Property | Value |
|:-:|:-:|
| Phase | FieldProvider (2) |
| Trust | Blue |
| Description | Inject cloud provider metadata: instance ID, region, availability zone, account ID (AWS IMDSv2 / GCP metadata server / Azure IMDS). |
| Config | `provider: "auto"`, `timeout_ms: 100` |
| Rationale | Essential for multi-cloud / hybrid-cloud deployments. Avoids baking cloud specifics into application config. |

### Filter: `filter_sampling`

| Property | Value |
|:-:|:-:|
| Phase | Filter (1) |
| Trust | Blue |
| Description | Probabilistic record sampling — keep 1/N records deterministically (by trace_id hash) or randomly. |
| Config | `rate: 0.01` (keep 1%), `key: "trace_id"` (deterministic by field), `min_level: "WARN"` (always keep WARN+) |
| Rationale | High-throughput systems cannot afford to store every DEBUG/TRACE record. Deterministic sampling preserves trace continuity. |

### KeyProvider: `key_file`

| Property | Value |
|:-:|:-:|
| Phase | KeyProvider (load-time) |
| Trust | Blue |
| Description | Read Ed25519 signing key from the filesystem with permission checks (must be 0600, owner-only). |
| Config | `path: "/etc/dologger/signing_key"`, `require_owner: true` |
| Rationale | Production deployments cannot embed keys in config TOML. This is the baseline external key provider. |

---

## Tier 3 — Extended (v0.4.0+)

These plugins address advanced or specialized use cases.

| Plugin | Phase | Description | Priority |
|:-:|:-:|:-:|:-:|
| `fmt_csv` | Formatting | RFC 4180 CSV output for analytics/warehouse import | Medium |
| `filter_regex` | Filter | Drop records matching regex patterns on `message` or named fields | Medium |
| `proc_geoip` | Processing | Add `geo.country`, `geo.city` from MaxMind GeoLite2 database | Low |
| `field_process` | FieldProvider | Process stats: PID, parent PID, command line, uptime, RSS | Medium |
| `key_env` | KeyProvider | Read signing key from environment variable (CI/CD, short-lived tokens) | Medium |
| `key_hsm` | KeyProvider | PKCS#11 interface to hardware security modules (YubiHSM, AWS CloudHSM) | Low |
| `policy_quota` | PolicyProvider | Per-domain record quota (count + byte budget per second) | Medium |

---

## Development Strategy

### Phase 1: Scaffolding (now)
1. Create `plugins/official/` directory structure
2. Each plugin gets a Cargo workspace member crate
3. Standardized `Cargo.toml` template with `license.workspace = true`
4. `PluginManifest.toml` per plugin with metadata

### Phase 2: Tier 1 implementation (v0.2.0)
1. `fmt_json` — highest impact, unblocks structured logging for all sinks
2. `field_container` — universal container metadata
3. `filter_level` + `fmt_text` — parity with built-in behaviors, but as plugins

### Phase 3: Tier 2 implementation (v0.3.0)
1. `proc_pii_mask` — compliance gate
2. `key_file` — production key management
3. `proc_field_enrich` + `field_cloud` + `filter_sampling`

### Phase 4: Tier 3 (v0.4.0+)
Community-driven with official reference implementations.

### Plugin Crate Template

(illustrative directory layout):

```text
plugins/official/fmt_json/
├── Cargo.toml
├── PluginManifest.toml
└── src/
    └── lib.rs
```

Each official plugin:
- Exports `plugin_query`, `plugin_init`, `plugin_shutdown` C ABI symbols
- Declares `license.workspace = true`
- Includes a `PluginManifest.toml` for the plugin index
- Has unit tests covering the VTable contract
- Is signed with the DoLogger root key (Blue trust level)

---

*Last updated: 2026-08-12*
