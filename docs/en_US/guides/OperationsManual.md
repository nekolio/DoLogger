# DoLogger Operations Manual

> 🌐 **语言 / Language**: [English](OperationsManual.md) | [中文：运维手册](../../zh_CN/guides/OperationsManual.md)

> **Version**: v0.0.1 | **Last Updated**: 2026-08-12 | **Target Audience**: SRE / Operations Engineers
>
> **Purpose**: This document covers the day-to-day operation of DoLogger in production environments — deployment, configuration management, performance tuning, monitoring, log lifecycle management, backup and disaster recovery, security operations, and incident response procedures.
>
> **Reading Path**: New operators should start with [Deployment Architecture](#deployment-architecture) and [Configuration Management](#configuration-management). For incident response, go directly to [Incident Response Procedures](#incident-response-procedures). Security-sensitive deployments should also read the [Security Whitepaper](SecurityWhitepaper.md).

## Table of Contents

1. [Deployment Architecture](#deployment-architecture)
2. [Configuration Management](#configuration-management)
3. [Performance Profile Selection](#performance-profile-selection)
4. [Monitoring and Alerting](#monitoring-and-alerting)
5. [Control Plane Operations](#control-plane-operations)
6. [Log Lifecycle Management](#log-lifecycle-management)
7. [Backup and Disaster Recovery](#backup-and-disaster-recovery)
8. [Security Operations](#security-operations)
9. [Incident Response Procedures](#incident-response-procedures)

---

## Deployment Architecture

### Deployment Modes

**Table 1: Deployment Mode Comparison**

| Mode             | Description | Use Case |
|:-:|:-:|:-:|
| **Embedded**     | Dynamic library linked directly into the host process. Single address space, minimal latency. | Low-latency, single-process services (e.g., a Rust microservice). |
| **Sidecar**      | Independent process receiving logs via `sink_shm` shared memory from one or more host processes. | Polyglot microservices needing operational isolation between app and logger. |
| **Daemon**       | System-level log collection service accepting logs over a local socket or shared memory. | Traditional syslog replacement for legacy applications. |

### Choosing a Deployment Mode

- **Embedded**: Use when you control the host process binary and cannot tolerate IPC overhead. Suitable for Rust and C applications.
- **Sidecar**: Use when the host application is written in a language without a native DoLogger adapter, or when you need process-level fault isolation between your application and the logging infrastructure.
- **Daemon**: Use for system-wide log collection across multiple applications, especially on container hosts or bare-metal servers.

### Filesystem Layout

**Linux:**

```text
(illustrative layout)
/etc/dologger/
  default.toml                  # System-wide default configuration
  conf.d/                       # Drop-in configuration fragments
    10-sinks.toml
    20-plugins.toml

/usr/lib/dologger/
  plugins/                      # System plugin directory
  libdologger_core.so           # Core engine shared library

/var/log/dologger/              # Log output directory
  app.log                       # Current log file
  app.2026-08-12.log.zst        # Rotated and compressed

/var/lib/dologger/
  audit/                        # WORM audit log storage
    audit-000001.worm
    audit-000002.worm
  state/                        # Engine state (LSN cursor, etc.)

/dev/shm/
  dologger_<name>.shm           # Shared memory segments (sink_shm mode)

/run/dologger/
  dologger.pid                  # PID file (daemon mode)
  control.sock                  # Unix domain socket (daemon mode)
```

**Windows:**

```text
(illustrative layout)
%PROGRAMDATA%\dologger\
  default.toml
  conf.d\

%PROGRAMFILES%\dologger\
  plugins\
  dologger_core.dll

%LOCALAPPDATA%\dologger\
  logs\
  audit\
```

### Installation

> [!NOTE]
> OS packages are not published yet — the commands below are illustrative (planned packaging). Today, build from source (`cargo build --release`) and copy the artifacts manually.

**Linux (APT):**
```bash
# (illustrative — packages not yet published)
sudo apt install dologger-core dologger-cli
```

**Linux (RPM):**
```bash
# (illustrative — packages not yet published)
sudo dnf install dologger-core dologger-cli
```

**Linux (manual tarball):**
```bash
# (illustrative — build from source today: cargo build --release)
tar xzf dologger-0.0.1-linux-x86_64.tar.gz
cd dologger-0.0.1-linux-x86_64
sudo cp libdologger_core.so /usr/lib/dologger/
sudo cp dologctl /usr/local/bin/
sudo mkdir -p /etc/dologger /var/log/dologger /var/lib/dologger/audit
sudo cp default.toml /etc/dologger/
```

**macOS (Homebrew):**
```bash
# (illustrative — formula not yet published)
brew install dologger/tap/dologger
```

---

## Configuration Management

### Core Configuration File

```toml
# /etc/dologger/default.toml — Production baseline
# Validate: dologctl config validate --config /etc/dologger/default.toml --strict
# (this file passes lenient validation; --strict fails until signatures are on)

[dologger]
level = "INFO"
performance_profile = "prod-performance"
ring_buffer_size = 262144       # MUST be a power of two
batch_size = 256
enable_audit = false           # Set true for audit deployments
enable_signature = false        # Add signatures when required
# The five domain-level non-downgradable items below are enforced by
# DomainManager, not read from [dologger] in v0.0.1 — listed for completeness:
escape_html = true              # Prevent CRLF / log injection
fsync_on_write = false          # Set true for crash-safe durability
require_tls = true              # Enforce TLS on all network sinks
sign_ring2 = false              # Set true to sign verified extension fields
shutdown_policy = "graceful"
shutdown_timeout_ms = 5000

# ── Sink definitions ──────────────────────────────────────────────
# (illustrative — v0.0.1 config parsing covers [dologger] keys only; sink
# sections are wired per-pipeline in code, see core/src/sink/)

# Sinks disabled in the old schema (enabled = false) are omitted — a sink is
# present iff its table exists; there is no enable flag.

[sinks.file]
type = "file"
path = "/var/log/dologger/app.log"
max_size = 104857600
durability_level = "os_cache"

# ── Plugin definitions ────────────────────────────────────────────

[plugins.json-formatter]
type = "formatter"
path = "/usr/lib/dologger/plugins/libjson_formatter.so"

[plugins.drop-debug]
type = "filter"
path = "/usr/lib/dologger/plugins/libdrop_debug.so"
```

### Configuration Priority (Lowest to Highest)

1. **Hardcoded defaults** — compiled into `libdologger_core`.
2. **System config** — `/etc/dologger/default.toml`
3. **Drop-in fragments** — `/etc/dologger/conf.d/*.toml` (merged alphabetically)
4. **Project-local config** — `dologger.toml` in CWD, searched upward
5. **Environment variables** — `DO_LOG_LEVEL`, `DO_LOG_CONFIG_FILE`, etc.
6. **Runtime API** — `dologger_config_load_from_string()`
7. **Non-downgradable items** — absolute hard limits (cannot be loosened by any lower layer)

### Environment Variables

| Variable               | Overrides          | Example |
|:-:|:-:|:-:|
| `DO_LOG_LEVEL`         | `level`            | `DO_LOG_LEVEL=DEBUG` |
| `DO_LOG_BUF_SIZE`      | `ring_buffer_size` | `DO_LOG_BUF_SIZE=524288` |
| `DO_LOG_PERF_PROFILE`  | `performance_profile` | `DO_LOG_PERF_PROFILE=prod-audit` |
| `DO_LOG_CONFIG_FILE`   | Config file path   | `DO_LOG_CONFIG_FILE=/opt/myapp/dologger.toml` |
| `DO_LOG_PLUGIN_DIR`    | Plugin directory   | `DO_LOG_PLUGIN_DIR=/opt/myapp/plugins` |
| `DO_LOG_CONFIG_LOCK`   | Prevent fallback config search (requires `DO_LOG_CONFIG_FILE`) | `DO_LOG_CONFIG_LOCK=1` |

### Configuration Validation

Use `dologctl` to validate configuration before applying it:

```bash
# Strict validation (enforces non-downgradable security invariants)
dologctl config validate --config /etc/dologger/default.toml --strict

# (planned — the --compliance flag is not shipped in v0.0.1)
# Validate with compliance profile
dologctl config validate \
    --config /etc/dologger/default.toml \
    --compliance gdpr

# (planned — no `config show` / `config diff` subcommands ship in v0.0.1)
# Dry-run showing effective configuration after merge
dologctl config show --effective

# Diff two configurations
dologctl config diff /etc/dologger/default.toml /etc/dologger/staging.toml
```

### Hot Reload

DoLogger can hot-reload the configuration file while `dologctl run` is active.
It is **opt-in**: add a `[watcher]` section to the config file to enable it. By
default the watcher is disabled, so existing deployments are unchanged until a
`[watcher]` section turns it on.

```toml
[dologger]
level = "INFO"

[watcher]
enabled = true
poll_interval_ms = 1000   # polling-only interval
debounce_ms = 500         # settle time after the last change
backend = "auto"          # auto | polling | inotify | read-directory-changes | fsevents
```

- When `enabled` is `true`, `dologctl run` watches the active config file and
  calls `Engine::reload_config` on each detected change.
- Native backends are auto-detected: **inotify** on Linux, **ReadDirectoryChangesW**
  on Windows, and polling on macOS (FSEvents deferred). `backend` overrides the
  auto-detected choice.
- A reload that fails to parse or to build/open its sinks is **rejected**: the
  previous config stays active and a sysmon error is recorded (error `-0x0206`
  `CONFIG_RELOAD_FAILED` / `-0x0208` `CONFIG_RELOAD_INVALID`). A transient bad
  edit does not terminate the engine.
- The active sink is swapped atomically through a shared `SinkRef`: in-flight
  writes complete under the same lock acquisition before the replaced sink is
  closed.
- Plugin changes still require an engine restart (plugins are not re-loaded at
  runtime by a reload).
- Full security-tier / non-downgradable validation of reloaded values is
  planned but not enforced by the reload path in this version.

```bash
# Change log level without restart
sed -i 's/level = "INFO"/level = "DEBUG"/' /etc/dologger/default.toml
# `dologctl run` detects the change and reloads automatically.
```

### Compliance Templates

DoLogger ships pre-built configuration templates for regulated environments:

| Template           | Path                          | Activates |
|:-:|:-:|:-:|
| GDPR               | `compliance/gdpr.toml`        | All non-downgradable security items |
| HIPAA              | `compliance/hipaa.toml`       | All non-downgradable security items |
| PCI DSS            | `compliance/pci-dss.toml`     | All non-downgradable security items |

Apply a compliance template (illustrative — `config merge` is planned; today merge the TOML `[dologger]` sections yourself and then run `dologctl config validate --strict`):

```bash
dologctl config merge \
    --base /etc/dologger/default.toml \
    --overlay compliance/gdpr.toml \
    --output /etc/dologger/gdpr-production.toml
```

---

## Performance Profile Selection

**Table 2: Performance Profile Reference**

| Property              | `dev`         | `balanced`    | `prod-performance` | `prod-audit`  |
|:-:|:-:|:-:|:-:|:-:|
| Block timeout         | 100 ms        | 2000 ms       | 3000 ms            | 3000 ms       |
| Drop strategy         | `drop_newest` | `oldest`      | `below_warn`       | `below_warn`  |
| Ed25519 signing       | Off           | Optional      | Optional           | **Required**  |
| WORM enforcement      | Off           | Optional      | Optional           | **Required**  |
| Batch size            | 32            | 128           | 256                | 128           |
| Ring buffer size      | 65536         | 131072        | 262144             | 262144        |

> [!NOTE]
> Block timeout and drop strategy values are enforced by `core/src/pipeline/backpressure.rs`. Dev / prod-performance / prod-audit batch and ring sizes match the shipped config templates; the `balanced` values are illustrative (no shipped `balanced` template in v0.0.1).
| Escape HTML           | Optional      | On            | On                 | **On**        |
| fsync on write        | Off           | Off           | Optional           | **On**        |
| Require TLS           | Off           | Warn-only     | On                 | **On**        |

### Selecting a Profile

```toml
# In dologger.toml:
[dologger]
performance_profile = "prod-performance"
```

```bash
# Or via environment variable:
export DO_LOG_PERF_PROFILE=prod-audit
```

You can override individual profile values:

```toml
[dologger]
performance_profile = "prod-performance"
ring_buffer_size = 524288       # Override the 262144 default
```

Overrides are merged on top of the profile defaults. Non-downgradable items cannot be relaxed via overrides.

### Drop Strategies

| Strategy       | Behavior |
|:-:|:-:|
| `drop_newest`  | When the ring buffer is full, discard the newest record. Prevents blocking producers. |
| `oldest`       | When the ring buffer is full, discard the oldest unprocessed record. Maintains freshness. |
| `below_warn`   | When the ring buffer is full, drop only records below WARN level. WARN and above are preserved. |
| `block`        | When the ring buffer is full, block the producer until space is available. Risk: can stall the host application. |

---

## Shared Memory Sink (sink_shm)

`sink_shm` delivers SIF records to external consumer processes through a
zero-copy, cross-process shared-memory ring buffer. It is wired **separately**
from `[sinks.*]` and is not a member of the sink registry. Enable it with the
top-level `[shm]` table:

```toml
[shm]
path = "/dologger_default.shm"   # POSIX name on Unix; mapping name on Windows
buffer_size_mb = 64              # power of two, >= 8
slot_size_kb = 64                # per-slot max, >= 64
full_policy = "drop_newest"      # drop_newest | drop_oldest
permissions = 0o660              # Unix only
auto_cleanup = true              # unlink the region on engine shutdown
allowed_consumers = []           # empty = allow all
```

`sink_shm` is **non-persistent** — `durability_level` is forced to `UNSAFE`. It
is therefore forbidden in AUDIT mode (`enable_audit = true` / `prod-audit`),
which requires durable WORM storage; the engine rejects that combination with
`DO_LOG_ERR_AUDIT_SHM_FORBIDDEN`.

### Enabling via the CLI

`dologctl run --shm <path>` enables `sink_shm` and overrides the shared-memory
path, keeping any other `[shm]` fields from the config (or defaults):

```bash
dologctl run --shm /dologger_default.shm
```

### Shared watermark semantics

The ring buffer header carries two sequence numbers:

| Field | Owner | Meaning |
|:-:|:-:|-|
| `producer_seq` | DoLogger (producer) | Next slot to write; advanced per accepted record |
| `consumer_seq` | Consumers (shared) | Recycle watermark — slots below it are safe to overwrite |

`consumer_seq` is a **single shared watermark** advanced cooperatively by
consumers via `compare_exchange`. There is exactly one watermark, so a
slow consumer can cause `drop_oldest`/`drop_newest` to kick in — DoLogger never
blocks producers on the shared-memory path. Consumers that are still draining a
slot that has been recycled must expect `overwritten_count` to increase and
re-read the region.

### Inspecting a region

```bash
dologctl shm status /dologger_default.shm          # human-readable
dologctl shm status /dologger_default.shm --output json
dologctl shm clear /dologger_default.shm           # requires producer DEAD or --force
```

`dologctl shm status` and `clear` read the header through the core
`dologger_core::sink::shm::read_status` API — the single source of truth for
the header layout (see `core/include/dologger_shm.h` for the consumer ABI).

---

## Monitoring and Alerting

### Sysmon Event Stream

The System Monitor (`sysmon`) emits structured events to `stderr` by default. Each event is a single JSON line:

```json
(illustrative — the real sysmon line format is:
 {"sysmon_version":"1.0","error_code":0,"category":"engine","description":"...","timestamp_ms":...,"severity":1})
{"ts":"2026-08-12T14:30:00.123Z","level":"WARN","event":"PIPELINE_BACKLOG","pct":72,"buf_name":"main"}
```

**Table 3: Sysmon Event Types**

| Event                  | Severity | Meaning | Immediate Action |
|:-:|:-:|:-:|:-:|
| `PIPELINE_BACKLOG`     | WARN     | Ring buffer occupancy exceeds 50% | Check consumer thread health; consider increasing `ring_buffer_size` |
| `PIPELINE_DROP`        | WARN     | Record(s) dropped due to full buffer | Increase capacity or switch to `prod-performance` profile |
| `SHM_DROP`             | WARN     | Shared memory sink dropped records | Verify consumer process is alive and consuming |
| `SINK_CIRCUIT_OPEN`    | ERROR    | Sink circuit breaker tripped | Check downstream service health; circuit auto-resets after 30s |
| `SINK_CIRCUIT_CLOSED`  | INFO     | Sink circuit breaker reset | Downstream recovered |
| `EMERGENCY_BUFFER`     | WARN     | Emergency spill buffer activated | Ring buffer overflow; records spilling to disk |
| `EMERGENCY_RECOVERED`  | INFO     | Spill buffer drained back into pipeline | System recovered from overflow |
| `SANDBOX_VIOLATION`    | CRITICAL | Plugin attempted disallowed syscall | Plugin thread terminated; review plugin trust color |
| `SIGNATURE_FAILURE`    | CRITICAL | Ed25519 signature verification failed | Log record may have been tampered with; initiate incident response |
| `LSN_GAP_DETECTED`     | ERROR    | Gap found in LSN sequence | Records may be missing; run `dologctl verify-log` |
| `CONFIG_RELOAD`        | INFO     | Configuration reloaded | Verification — check that the expected changes took effect |
| `CONFIG_RELOAD_DENIED` | WARN     | Configuration reload rejected | Attempt to loosen a non-downgradable item |
| `LICENSE_POLICY_VIOLATION` | ERROR | Plugin rejected due to license incompatibility | Review plugin SPDX identifier |

### Control Plane Status Endpoint

```bash
# pseudocode/illustrative — the control plane is not started with the engine
# in v0.0.1; the response format below matches the /status handler in
# core/src/sys/control_plane.rs
# curl -s http://127.0.0.1:9090/status | jq .
```

```json
(illustrative — the /status handler's response is smaller:
 {"status":"ok","level":"INFO","profile":"prod-performance","plugins":0,"signature_enabled":false};
 the rich metrics body below is planned)
{
  "status": "ok",
  "uptime_seconds": 86412,
  "level": "INFO",
  "profile": "prod-performance",
  "plugins_loaded": 3,
  "plugins_failed": 0,
  "signature_enabled": false,
  "ring_buffer": {
    "capacity": 262144,
    "used": 8192,
    "pct_used": 3.1,
    "drops_total": 0,
    "emergency_spills": 0
  },
  "sinks": {
    "file": {"status": "healthy", "bytes_written": 1073741824},
    "kafka": {"status": "healthy", "messages_sent": 5000000}
  },
  "pipeline": {
    "records_processed": 10000000,
    "records_dropped": 0,
    "avg_latency_us": 82
  }
}
```

### Key Metrics and Alerting Thresholds

**Table 4: Operational Metrics**

| Metric                       | Baseline (P50) | Warning Threshold | Critical Threshold | Source |
|:-:|:-:|:-:|:-:|:-:|
| Record submission latency    | < 102 ns       | > 500 ns          | > 2 us              | `/status` |
| Ring buffer utilization      | < 70%          | > 80%             | > 90%               | `/status` |
| Drop rate                    | 0%             | > 0.01%           | > 0.1%              | `/status` |
| Sink write latency           | < 1 ms         | > 10 ms           | > 100 ms            | `/status` |
| Circuit breaker trips / hour | 0              | > 1               | > 3                 | sysmon `SINK_CIRCUIT_OPEN` count |
| Signature failures           | 0              | > 0               | > 0 (any)           | sysmon `SIGNATURE_FAILURE` |
| Sandbox violations           | 0              | > 0               | > 0 (any)           | sysmon `SANDBOX_VIOLATION` |
| LSN gaps                     | 0              | > 0               | > 0 (any)           | sysmon `LSN_GAP_DETECTED` |

### Prometheus Integration (planned)

```yaml
# prometheus.yml scrape config (illustrative — planned)
scrape_configs:
  - job_name: 'dologger'
    static_configs:
      - targets: ['localhost:9090']
    metrics_path: '/metrics'
```

### Log-Based Alerting

Ship sysmon events to your centralized logging platform (Elasticsearch, Loki, Splunk) and configure alerts:

```text
(illustrative alert rule sketch, not literal query syntax)
# Elasticsearch alert query example
event: "SIGNATURE_FAILURE" OR event: "SANDBOX_VIOLATION"
  → PagerDuty: critical
  → Slack: #incident-response

event: "SINK_CIRCUIT_OPEN"
  → PagerDuty: warning (escalate to critical after 5 minutes)

event: "PIPELINE_BACKLOG" AND pct > 90
  → Slack: #sre
```

---

## Control Plane Operations

### HTTP API Endpoints

**Table 5: Control Plane API (planned)** — none of these endpoints are started with the engine in v0.0.1.

| Method | Path       | Auth | Description |
|:-:|:-:|:-:|:-:|
| GET    | `/status`  | None | Engine status and metrics (see above) |
| GET    | `/health`  | None | Liveness check (planned — not implemented in v0.0.1) |
| POST   | `/level`   | None | Dynamically set the log level |
| POST   | `/reload`  | None | Trigger configuration reload |

### Changing Log Level at Runtime

```bash
# pseudocode/illustrative — the control plane is not started in v0.0.1
# Temporarily increase verbosity for debugging
# curl -X POST http://127.0.0.1:9090/level \
#   -H "Content-Type: application/json" \
#   -d '{"level": "DEBUG"}'

# Restore production level
# curl -X POST http://127.0.0.1:9090/level \
#   -H "Content-Type: application/json" \
#   -d '{"level": "INFO"}'

# Query current level
# curl -s http://127.0.0.1:9090/status | jq .level
```

### Triggering Configuration Reload

Hot reload is triggered automatically when `[watcher]` is enabled (see the
Hot Reload section above). A control-plane reload endpoint remains planned:

```bash
# planned — the control plane is not started in this version
# Reload without validation (applies changes if syntax is valid)
# curl -X POST http://127.0.0.1:9090/reload
```

### Security Considerations

- The control plane listens on `127.0.0.1:9090` by default (planned — the control plane is not started in v0.0.1) — only processes on the same host can reach it.
- mTLS + JWT authentication for remote access is planned.
- Production deployments should use host-level firewall rules to restrict access to the control plane port:
  ```bash
  # iptables: restrict to localhost only
  sudo iptables -A INPUT -p tcp --dport 9090 -s 127.0.0.1 -j ACCEPT
  sudo iptables -A INPUT -p tcp --dport 9090 -j DROP
  ```
- The `DO_LOG_CONFIG_LOCK=1` environment variable prevents fallback config search (the configured `DO_LOG_CONFIG_FILE` must exist).

---

## Log Lifecycle Management

### Rotation Policies

File Sink supports both size-based and time-based rotation:

```toml
# (illustrative — v0.0.1 FileSinkConfig has: path, max_size (bytes),
# fsync_on_write, durability_level, buffer_size; time-based rotation,
# compression, and file-count retention are planned)
[sinks.file]
type = "file"
path = "/var/log/dologger/app.log"
max_size = 104857600            # Rotate when file exceeds 100 MB
rotation_interval = "24h"       # Rotate at midnight regardless of size
max_rotated_files = 90          # Keep at most 90 rotated files
compression = "zstd"            # Compress rotated files (gzip | zstd | none)
```

Rotation does not block log submission. A new file is opened while the old file is closed and (optionally) compressed on a background thread.

### Retention Policies

```toml
# (illustrative — retention policy keys are planned, not parsed in v0.0.1)
[sinks.file]
retention_days = 90             # Delete files older than 90 days
retention_total_size = "10GB"   # Delete oldest files when total exceeds 10 GB
```

Retention is checked once per rotation. If both `retention_days` and `retention_total_size` are set, files are deleted when **either** condition is met.

### Cold-Hot Tiering

**Table 6: Storage Tier Strategy**

| Tier | Storage         | Retention | Format            | Access Pattern |
|:-:|:-:|:-:|:-:|:-:|
| Hot  | Local NVMe/SSD  | 0–7 days  | Uncompressed      | `tail -f`, `grep`, real-time dashboards |
| Warm | Local HDD        | 7–90 days | Zstd-compressed   | `dologctl query`, incident investigation |
| Cold | S3 / GCS / ABS  | 90+ days  | Parquet columnar  | Compliance audits, long-term analytics |

**Automated tiering (planned):**

```toml
# (planned — illustrative schema)
[sinks.file.tiering]
enabled = true
warm_storage = "/data/dologger/warm/"
cold_storage = "s3://my-audit-logs/cold/"
promote_to_warm_after = "7d"
archive_to_cold_after = "90d"
```

### WORM Audit Log Handling

WORM (Write-Once-Read-Many) audit logs are stored separately and handled with special care:

```bash
# List WORM segments
ls -la /var/lib/dologger/audit/
# -r-------- 1 root root 104857600 Aug 12 00:00 audit-000001.worm
# -r-------- 1 root root  52428800 Aug 12 12:00 audit-000002.worm

# Verify a single WORM file's chain (verify-log takes a file path)
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm

# Or report LSN continuity across all *.worm files in a directory
dologctl recovery-report /var/lib/dologger/audit/

# (planned — no `dologctl audit export` ships in v0.0.1)
# Export audit records to JSON for analysis
dologctl audit export \
    --path /var/lib/dologger/audit/ \
    --from "2026-08-01" \
    --to   "2026-08-12" \
    --format json \
    --output audit-august-2026.json
```

---

## Backup and Disaster Recovery

### WORM Audit Log Backup

```bash
# Verify integrity before backup
dologctl recovery-report /var/lib/dologger/audit/

# If verification passes, rsync to backup location
rsync -avz \
    /var/lib/dologger/audit/ \
    backup-server:/backups/dologger/$(hostname)/audit/

# (planned — the --latest-lsn-only flag and anchor publish do not ship in v0.0.1)
# Record the last verified LSN for external anchoring
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm --latest-lsn-only
# {"latest_lsn": 100042,"root_hash": "a3f8b2c1..."}

# Publish root hash to an external witness (S3 object metadata, blockchain anchor, etc.)
# planned: dologctl anchor publish --s3-bucket audit-anchors --root-hash "a3f8b2c1..."
```

### Emergency Buffer Recovery

When the ring buffer overflows, records are spilled to an emergency file on disk (in the `dologger/` subfolder of the system temp directory — see `core/src/buffer/emergency_buffer.rs`):

```text
dologger_emergency_<pid>_<spill_id>.buf
```

On recovery (when the ring buffer has free space):

1. The engine detects the emergency file at startup.
2. Records are read from the file and injected into the main pipeline.
3. The emergency file is deleted after successful replay.
4. A `EMERGENCY_RECOVERED` sysmon event is emitted.

**Manual recovery:**

```bash
# Check for abandoned emergency files (system temp directory's dologger/ subfolder)
ls -la /tmp/dologger/dologger_emergency_*.buf

# If the engine is running and the file persists, check engine status
# (pseudocode/illustrative — the control plane is not started in v0.0.1;
# the planned /status response has no ring_buffer object yet)
# curl http://127.0.0.1:9090/status

# If the engine crashed, the emergency file will be replayed on next startup
```

### Configuration Backup

```bash
# Backup the active configuration
cp /etc/dologger/default.toml /backups/dologger/config-$(date +%Y%m%d).toml

# (planned — no `config show` subcommand ships in v0.0.1)
# Backup with dologctl (includes merged effective config)
dologctl config show --effective > /backups/dologger/effective-$(date +%Y%m%d).toml
```

### Recovery Time Objectives

**Table 7: RTO/RPO Reference**

| Scenario                          | RPO                            | RTO          | Procedure |
|:-:|:-:|:-:|:-:|
| Disk failure (non-WORM)           | Last rotation (max 24h)        | Time to reprovision disk + restore from backup | Restore from backup server |
| Disk failure (WORM)               | Last fsync (0 records lost)    | Time to reprovision disk | WORM files fsync on every write |
| Process crash                     | Emergency buffer replay        | < 10 seconds | Engine auto-restarts; emergency buffer replayed |
| Accidental log deletion (non-WORM)| Last backup                    | Time to restore from backup | Restore from backup server |
| Accidental log deletion (WORM)    | N/A — files are read-only      | N/A           | WORM files cannot be deleted without OS-level intervention |

---

## Security Operations

### Non-Downgradable Items

The following 5 configuration items can only be **tightened** (moved toward greater security) across configuration layers; they can never be loosened:

**Table 8: Non-Downgradable Security Items**

| Item                | Loosening Means      | Security Impact if Loosened |
|:-:|:-:|:-:|
| `enable_signature`  | `true` → `false`     | Logs are no longer cryptographically verifiable. Non-repudiation is lost. |
| `escape_html`       | `true` → `false`     | Log injection (CRLF) attacks become possible. |
| `fsync_on_write`    | `true` → `false`     | Crashes may lose in-flight audit records; durability guarantee is voided. |
| `require_tls`       | `true` → `false`     | Network sinks accept unencrypted connections; man-in-the-middle attack surface. |
| `sign_ring2`        | `true` → `false`     | Verified extension fields lose cryptographic binding. |

Any attempt to loosen these items triggers a `CONFIG_RELOAD_DENIED` sysmon event and the change is rejected.

### Key Management

Ed25519 key pairs for log signing are managed by the `KeyProvider` plugin:

- **Default**: Built-in ephemeral key generator. Keys are generated once in-memory at startup and are **never written to disk**. Restarting the engine generates a new key, invalidating previous signatures.
- **Production**: Deploy an external `KeyProvider` backed by HSM (Hardware Security Module), AWS KMS, or HashiCorp Vault. This ensures key persistence across restarts and hardware-backed key protection.

```toml
# (illustrative — plugin config sections are not parsed in v0.0.1)
[plugins.hsm-key-provider]
type = "key_provider"
path = "/usr/lib/dologger/plugins/libhsm_keyprovider.so"

[plugins.hsm-key-provider.config]
pkcs11_module = "/usr/lib/softhsm/libsofthsm2.so"
slot_id = 0
key_label = "dologger-signing-key"
```

### Audit Trail Tamper Detection

The LSN (Log Sequence Number) + content_hash chain provides cryptographic tamper
evidence (pseudocode — illustrative):

```
Record(N):
  lsn          = N
  content_hash = SHA-256( canonical_serialization(Record(N)) )
  prev_hash    = SHA-256( Record(N-1).content_hash || Record(N-1).lsn )
  # sidecar audit.log.sig: sig(N) = Ed25519_Sign(TPM key, SHA-256(lsn || content_hash || prev_hash))

Record(N+1):
  lsn          = N+1
  content_hash = SHA-256( canonical_serialization(Record(N+1)) )
  prev_hash    = SHA-256( Record(N).content_hash || Record(N).lsn )
  # sidecar audit.log.sig: sig(N+1) = Ed25519_Sign(TPM key, SHA-256(lsn || content_hash || prev_hash))
```

If any record is modified, inserted, or deleted, the `content_hash` / `prev_hash`
chain breaks and verification fails.

**Verification command:**

```bash
# (--verbose is planned; v0.0.1 verify-log takes the file path positionally)
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm \
    --sidecar /var/lib/dologger/audit/audit-000001.sig

# (illustrative example output)
# [OK]     LSN 000001 — content_hash valid, signature valid, prev_hash=genesis
# [OK]     LSN 000002 — content_hash valid, signature valid, prev_hash matches
# [GAP]    LSN 000003 — missing (expected, found LSN 000004)
# [OK]     LSN 000004 — content_hash valid, signature valid, prev_hash matches
# [FAIL]   LSN 000005 — content_hash INVALID (record may be tampered)
# ...
# Summary: 9995 OK, 1 GAP, 1 FAIL — INTEGRITY CHECK FAILED
```

### Security Monitoring Checklist

- [ ] Sysmon events shipped to centralized logging platform
- [ ] `SIGNATURE_FAILURE` and `SANDBOX_VIOLATION` events trigger PagerDuty alerts
- [ ] `dologctl verify-log` runs daily via cron and reports failures
- [ ] Non-downgradable items audited weekly against production configuration
- [ ] Key rotation schedule established (manual today; automated rotation planned)
- [ ] Plugin signatures verified on every engine startup
- [ ] Control plane restricted to localhost via firewall
- [ ] TLS certificates for network sinks monitored for expiry

---

## Incident Response Procedures

### Incident: Log Loss Detected

**Symptoms:**
- `PIPELINE_DROP` events in sysmon
- Records missing from output files
- Gap in LSN sequence

**Response:**

1. **Triage**: `curl http://127.0.0.1:9090/status | jq .ring_buffer` (pseudocode/illustrative — the control plane is not started in v0.0.1)
2. **Check drops**: Look at `pct_used`, `drops_total`, `emergency_spills`
3. **Identify bottleneck**: Sink health status — is a sink in `circuit_open` state?
4. **Mitigation**:
   ```bash
   # (planned — no /sink/disable endpoint ships in v0.0.1)
   # If a sink is circuit-open and non-critical, disable it
   curl -X POST http://127.0.0.1:9090/sink/disable -d '{"sink": "kafka"}'
   ```
5. **Increase capacity**:
   ```bash
   # Set larger ring buffer via environment variable and restart
   export DO_LOG_BUF_SIZE=524288
   ```
6. **Recover**: Emergency buffer files will auto-replay. Verify with `dologctl verify-log`.

### Incident: Signature Verification Failure

**Symptoms:**
- `SIGNATURE_FAILURE` event in sysmon
- `dologctl verify-log` reports `FAIL` on one or more records

**Response:**

1. **Isolate**: Identify the affected LSN range.
   ```bash
   # (--verbose is planned)
   dologctl verify-log /var/lib/dologger/audit/audit-000001.worm 2>&1 | grep FAIL
   ```
2. **Assess**: Determine if this is a single-record corruption (disk error) or systematic tampering.
3. **Investigate**:
   - Check system logs for disk I/O errors around the affected timestamp.
   - Verify file permissions — was the WORM file writable by an unauthorized process?
   - Check for root-user activity on the host during the affected time window.
4. **Contain**: If tampering is suspected, isolate the host from the network and preserve a forensic image.
5. **Report**: File a security incident report. The affected records carry a cryptographic trail — preserve the WORM files as evidence.
6. **Remediate**: Rotate signing keys if compromise is confirmed.

### Incident: Sandbox Violation

**Symptoms:**
- `SANDBOX_VIOLATION` event in sysmon
- Plugin process terminated

**Response:**

1. **Identify**: The sysmon event contains the plugin name and syscall attempted (illustrative example).
   ```json
   {"event":"SANDBOX_VIOLATION","plugin":"untrusted-plugin","syscall":"fork","action":"KILL"}
   ```
2. **Isolate**: The violating plugin has already been terminated by the sandbox.
3. **Investigate**: Review the plugin's `manifest.toml` — does its `trust.color` match its behavior?
4. **Decision**:
   - If the plugin is malicious or compromised: remove it permanently.
   - If the plugin is legitimate but misclassified: upgrade its trust color (Red → Yellow, Yellow → Blue) only after code review and re-signing.
5. **Prevent**: Update the plugin vetting process.

### Incident: Performance Degradation

**Symptoms:**
- `PIPELINE_BACKLOG` frequency increasing
- Application latency increasing (blocking on `dologger_log` for AUDIT records)
- Ring buffer utilization trending upward

**Response:**

1. **Baseline**: Run `cargo bench` to confirm the engine itself is performing as expected.
2. **Profile**: Verify `performance_profile` — has it been changed to a lower-throughput profile?
   ```bash
   # (pseudocode/illustrative — the control plane is not started in v0.0.1)
   curl http://127.0.0.1:9090/status | jq .profile
   ```
3. **Check sinks**: Are sinks healthy? A slow downstream can cause backpressure.
4. **Check signatures**: Is `enable_signature` unexpectedly `true`? Signing adds ~17 us per record.
5. **Check disk**: Is the filesystem underlying the file sink experiencing high latency?
   ```bash
   iostat -x 1
   ```
6. **Mitigation**:
   - Temporarily reduce log level to `WARN` or `ERROR`.
   - Switch to `prod-performance` profile if not already.
   - Increase `ring_buffer_size`.
   - Add more Sink consumers for parallel writes.

### Post-Incident Review

After any incident, collect a diagnostic report:

```bash
# (`dologctl diag collect` is planned; gather the pieces manually today)
dologctl about --output json > post-incident-$(date +%Y%m%d-%H%M%S).json
dologctl config validate --strict
```

Review the collected data alongside the sysmon event timeline to identify root cause and preventive measures.
