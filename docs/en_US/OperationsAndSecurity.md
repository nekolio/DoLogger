# DoLogger Operations & Security Guide

> **Version**: v0.0.1 | **Last Updated**: 2026-08-12 | **Target Audience**: SRE, Operations Engineers, Security Engineers, Compliance Officers
>
> **Purpose**: Production deployment, monitoring, key management, audit verification, incident response, and compliance configuration for DoLogger. This is the operations manual for running DoLogger in production environments.
>
> 🌐 **语言 / Language**: [English](OperationsAndSecurity.md) | [中文：运维与安全指南](../zh_CN/OperationsAndSecurity.md)
>
> **Reading Path**: SREs should start with [Deployment Modes](#deployment-modes) and [Monitoring](#monitoring). Security engineers should focus on [Key Management](#key-management) and [Audit Verification](#audit-verification). For the underlying architecture, see the [Architecture Reference](ArchitectureReference.md).

---

## Table of Contents

1. [Before You Start](#before-you-start)
2. [Deployment Modes](#deployment-modes)
3. [Monitoring](#monitoring)
4. [Key Management](#key-management)
5. [Audit Verification](#audit-verification)
6. [Incident Response Runbooks](#incident-response-runbooks)
7. [Compliance Configuration](#compliance-configuration)
8. [Sandbox Configuration Per Trust Level](#sandbox-configuration-per-trust-level)
9. [Performance Regression Detection](#performance-regression-detection)

---

## Before You Start

### Prerequisites

- DoLogger engine built and installed (see [Quick Start Guide](QuickStart.md))
- The `dologctl` CLI tool available on your PATH
- Root or sudo access for system-wide installation
- Understanding of the [Architecture Reference](ArchitectureReference.md) for internals
- For compliance deployments: access to the `compliance/` template directory

### Filesystem Layout

**Linux:**

(illustrative layout):

```
/etc/dologger/
  default.toml                       # System-wide configuration
  conf.d/                            # Drop-in fragments (merged alphabetically)
    10-sinks.toml
    20-plugins.toml

/usr/lib/dologger/
  plugins/                           # System plugin directory
  libdologger_core.so                # Core engine shared library

/var/log/dologger/                   # Log output
  app.log                            # Current file
  app.2026-08-12.log.zst             # Rotated and compressed

/var/lib/dologger/
  audit/                             # WORM audit logs
    audit-000001.worm
    audit-000002.worm
  state/                             # Engine state (LSN cursor, etc.)

/dev/shm/
  dologger_<name>.shm                # Shared memory (sidecar mode)

/run/dologger/
  dologger.pid                       # PID file (daemon mode)
  control.sock                       # Unix domain socket (daemon mode)
```

**Windows:**

(illustrative layout):

```
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

---

## Deployment Modes

### Mode Comparison

| Mode | Description | Latency | Isolation | Use Case |
|:-:|:-:|:-:|:-:|:-:|
| **Embedded** | `libdologger_core` linked directly into host process | Lowest (102 ns P50) | Shared address space | Single-process services, Rust/C applications |
| **Sidecar** | Independent process, logs received via `sink_shm` shared memory | Low (~1 us) | Process isolation | Polyglot microservices, fault isolation |
| **Daemon** | System-level service, local socket or shared memory | Moderate | Process isolation | Legacy applications, system-wide collection |

### Embedded Deployment

```bash
# Build the engine
cargo build --release

# Link into your application
cc -o myapp myapp.c -ldologger_core -L./target/release

# Run with project-local config
DO_LOG_CONFIG_FILE=./dologger.toml ./myapp
```

### Sidecar Deployment

```bash
# Start the sidecar process (illustrative — the long-running sidecar mode lands
# not implemented yet; today dologctl run supports --dry-run and --trace only)
dologctl run --config /etc/dologger/sidecar.toml --mode sidecar &

# Configure host applications to use sink_shm
```

Sidecar configuration (field names follow `ShmSinkConfig` in `core/src/sink/shm.rs`):

```toml
[dologger]
performance_profile = "prod-performance"

[sinks.shm]
type = "sink_shm"
path = "dologger_app"
input_format = "sif"
buffer_size_mb = 100        # 100 MB
slot_size_kb = 256
full_policy = "drop_oldest" # What to do when SHM is full
```

### Daemon Deployment

Install as a system service:

**Linux (systemd):**

(illustrative unit file — engine daemon mode is not implemented yet; `dologctl run` currently supports `--dry-run` and `--trace` only):

```ini
# /etc/systemd/system/dologger.service
[Unit]
Description=DoLogger Logging Engine
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/dologctl run --config /etc/dologger/default.toml
Restart=on-failure
RestartSec=5
User=dologger
Group=dologger
LimitNOFILE=65536
LimitMEMLOCK=268435456
CPUAffinity=2-3

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable dologger
sudo systemctl start dologger
sudo systemctl status dologger
```

---

## Monitoring

### Sysmon Event Stream

The System Monitor emits structured JSON events to `stderr` (or configurable output):

```json
{"sysmon_version":"1.0","error_code":0,"category":"engine","description":"Engine initialized: ring_size=65536, coop_helping=false","timestamp_ms":1786561656221,"severity":1}
```

### Sysmon Event Types

| Event | Level | Meaning | Action Required |
|:-:|:-:|:-:|:-:|
| `PIPELINE_BACKLOG` | WARN | Ring buffer >50% full | Monitor trend; consider increasing `ring_buffer_size` |
| `PIPELINE_DROP` | WARN | Records dropped (buffer full) | Investigate sink health; increase capacity |
| `SHM_DROP` | WARN | Shared memory sink dropped records | Verify consumer process is alive |
| `SINK_CIRCUIT_OPEN` | ERROR | Remote sink unavailable | Check downstream service; auto-resets after 30s |
| `SINK_CIRCUIT_CLOSED` | INFO | Remote sink recovered | Confirm in monitoring dashboard |
| `EMERGENCY_BUFFER` | WARN | Spill buffer activated (>=95% full) | Ring buffer overflow; records on disk |
| `EMERGENCY_RECOVERED` | INFO | Spill buffer drained | System recovered |
| `SANDBOX_VIOLATION` | CRITICAL | Plugin attempted disallowed syscall | Plugin terminated; investigate immediately |
| `SIGNATURE_FAILURE` | CRITICAL | Ed25519 verification failed | Possible log tampering; initiate incident response |
| `LSN_GAP_DETECTED` | ERROR | Missing records in audit chain | Run `dologctl verify-log` |
| `CONFIG_RELOAD` | INFO | Configuration reloaded | Verify expected changes took effect |
| `CONFIG_RELOAD_DENIED` | WARN | Reload rejected (non-downgradable item) | Check for security policy violation |
| `LICENSE_POLICY_VIOLATION` | ERROR | Plugin rejected (incompatible license) | Review plugin SPDX |

### Control Plane API

The control plane provides a lightweight HTTP API for runtime management — planned: none of these endpoints are started with the engine in v0.0.1.

| Method | Path | Auth | Description |
|:-:|:-:|:-:|:-:|
| GET | `/status` | None | Engine status and metrics |
| GET | `/health` | None | Liveness check (200 = alive) *(planned)* |
| POST | `/level` | None | Set log level dynamically |
| POST | `/reload` | None | Trigger configuration reload |

### Health Check

```bash
# pseudocode/illustrative — the control plane is not started in v0.0.1;
# the current implementation (core/src/sys/control_plane.rs) only has
# GET /status, POST /level, POST /reload — no /health endpoint
# curl -s http://127.0.0.1:9090/health
# HTTP 200 OK
```

### Status Endpoint

```bash
# pseudocode/illustrative — the control plane (GET /status) is not started
# with the engine in v0.0.1
# curl -s http://127.0.0.1:9090/status | jq .
```

```json
{"status":"ok","level":"INFO","profile":"prod-performance","plugins":0,"signature_enabled":false}
```

The response is deliberately minimal today; richer metrics (uptime, ring buffer statistics, per-sink health, pipeline counters) are planned.

### Alerting Thresholds

| Metric | Warning | Critical | Alert Channel |
|:-:|:-:|:-:|:-:|
| Ring buffer utilization | > 80% | > 90% | Slack #sre |
| Drop rate | > 0.01% | > 0.1% | PagerDuty warning |
| Sink write latency | > 10 ms | > 100 ms | Slack #sre |
| Circuit breaker trips/hour | > 1 | > 3 | PagerDuty warning |
| Signature failures | > 0 (any) | > 0 (any) | **PagerDuty critical** |
| Sandbox violations | > 0 (any) | > 0 (any) | **PagerDuty critical** |
| LSN gaps | > 0 (any) | > 0 (any) | **PagerDuty critical** |

### Dynamic Log Level Adjustment

```bash
# pseudocode/illustrative — the control plane (POST /level) is not started
# with the engine in v0.0.1
# curl -X POST http://127.0.0.1:9090/level \
#   -H "Content-Type: application/json" \
#   -d '{"level": "DEBUG"}'

# Restore production level
# curl -X POST http://127.0.0.1:9090/level \
#   -H "Content-Type: application/json" \
#   -d '{"level": "INFO"}'

# Lock the level (disable runtime changes) — this env var really works
export DO_LOG_CONFIG_LOCK=1
```

### Hot Reload

```bash
# Edit the config file
vim /etc/dologger/default.toml

# pseudocode/illustrative — the control plane (POST /reload) is not started
# with the engine in v0.0.1
# curl -X POST http://127.0.0.1:9090/reload

# Dry-run first (planned — the reload endpoint ignores the request body today)
# curl -X POST http://127.0.0.1:9090/reload \
#   -H "Content-Type: application/json" \
#   -d '{"dry_run": true}'
```

### Control Plane Security

- Binds to `127.0.0.1:9090` by default (localhost only; planned — the control plane is not started in v0.0.1)
- mTLS + JWT authentication for remote access is planned
- Production: use host firewall to restrict access

```bash
# iptables: restrict control plane to localhost
sudo iptables -A INPUT -p tcp --dport 9090 -s 127.0.0.1 -j ACCEPT
sudo iptables -A INPUT -p tcp --dport 9090 -j DROP
```

---

## Key Management

DoLogger has two independent signing domains, each with its own key:

| Key Domain | Purpose | Key Material | Managed By |
|:-:|:-:|:-:|:-:|
| Log record signing | Per-record Ed25519 signatures on audit domains | Ephemeral pair generated in memory at startup | Built-in `DefaultKeyProvider` (planned — no `KeyProvider` plugin ships in v0.0.1) |
| Plugin signing | Ed25519 signatures over official plugin bundles (Blue trust) | Private seed stored **only** as the `DOLOGGER_PLUGIN_SIGNING_KEY` GitHub Actions secret; public anchors + CRL committed | `dologctl plugin` commands + committed `plugins/official/trust-anchors/` |

### Plugin Signing Keys (v0.0.1 — live)

The official plugin bundle is signed with the project's Ed25519 seed. The private
key never enters the repository:

- The **seed** is a raw 64-hex Ed25519 value stored exclusively in the
  `DOLOGGER_PLUGIN_SIGNING_KEY` GitHub Actions secret (encrypted at rest by
  GitHub's infrastructure). No `.key`, `.seed`, or `.enc` file is committed — see
  `.gitignore` and the `leak-hygiene` workflow.
- The **public half** lives in `plugins/official/trust-anchors/active.pub` (one
  64-hex public key per line) with a revocation list `revoked.txt` (CRL of key
  fingerprints). These files are public and committed.
- The loader (in `dologger-core`) verifies a plugin's `.sig` against **any**
  active, non-revoked anchor → **Blue** trust. A signature matching only a
  *revoked* anchor is **rejected** (`SignatureInvalid`), and this holds even in
  dev mode — revocation is real.

#### CLI commands

| Command | Purpose |
|:-:|:-:|
| `dologctl plugin keygen <path>` | Generate a new Ed25519 seed (0600 perms); prints the public key |
| `dologctl plugin sign <lib> [--key <seed> \| --wrapped-key <enc>] [--require-2fa]` | Sign a library, writing `<lib>.sig`; seed from a file, a wrapped key (prompts for passphrase), or `DO_LOG_PLUGIN_SIGNING_KEY`; optional TOTP 2FA gate |
| `dologctl plugin verify [--trust-store <dir>]` | Verify plugins; a `--trust-store` is authoritative over the env anchor |
| `dologctl plugin list --trust-store <dir>` | List plugins with the trust store applied |
| `dologctl plugin wrap-key <seed> <out>` / `unwrap-key <enc> <out>` | AES-256-GCM wrap/unwrap a seed under an SSH-style passphrase (local key hygiene) |
| `dologctl plugin totp [secret] [--uri]` | Show the current TOTP code or an `otpauth://` provisioning URI for signing 2FA |

#### Local key hygiene (`wrap-key`)

`signing.key` on your machine is plaintext. `dologctl plugin wrap-key` encrypts it
with AES-256-GCM under a passphrase (from `DO_LOG_PLUGIN_KEY_PASSPHRASE` or an
interactive prompt). The wrapped file begins with the `DOLOGKEY1` magic and is
recovered with `unwrap-key`. Never commit either form.

#### Signing 2FA

Set `DO_LOG_PLUGIN_TOTP_SECRET` (base32) so every `dologctl plugin sign` requires a
TOTP code from your authenticator app. `dologctl plugin totp --uri` prints an
`otpauth://` URI to provision the app; `dologctl plugin totp` shows the current
code. `--require-2fa` forces the gate even when the env var is absent.

#### Scheduled rotation runbook

1. `dologctl plugin keygen new-signing.key` — generate a fresh key.
2. Add the new public key to `plugins/official/trust-anchors/active.pub` and commit
   (both keys now active — old signatures keep verifying).
3. Replace the `DOLOGGER_PLUGIN_SIGNING_KEY` secret with the new seed
   (Settings → Secrets and variables → Actions).
4. After a grace window, append the **old** key's fingerprint to
   `plugins/official/trust-anchors/revoked.txt` with reason `superseded` and commit —
   old-key signatures now fail verification.

#### Emergency revocation runbook (compromise)

Loss stays bounded: a leaked key can only vouch for plugins released between the
compromise and the revocation.

1. Append the compromised fingerprint to `revoked.txt` with reason `compromised`
   and commit. The loader rejects its signature **immediately** — even in dev mode,
   even if the key is still listed in `active.pub` (the CRL wins).
2. Rotate the secret: `keygen` → update `DOLOGGER_PLUGIN_SIGNING_KEY` → add the new
   public key to `active.pub` → commit.
3. Already-shipped artifacts signed by the revoked key fail verification for any
   loader with the updated store.

#### Workflow-compromise defense

The release workflow signs bundles with the raw secret, so a compromised workflow
is the one place a leak could happen. Mitigations:

- **SHA-pinned actions** — every `uses:` is pinned to an immutable commit SHA; a
  tagged action cannot be silently retargeted.
- **Least privilege** — top-level `permissions: contents: read`; only the
  `create-release` job grants `contents: write`.
- **Sign-step isolation** — the seed is written to a 0600 file inside the signing
  step only, under `trap 'rm -f ...' EXIT` so early-exit failures wipe it; the
  secret is scoped to that step's `env:`.
- **Trusted triggers** — the workflow runs on `tags: ['v*']` only; no
  `pull_request_target`.
- **Leak-hygiene job** — `.github/workflows/leak-hygiene.yml` scans every push/PR
  for private-key blocks and 64-hex seeds in key-named files.

Future hard guarantee (roadmap): **OIDC → cloud KMS**. GitHub's OIDC token would
let the workflow obtain the signing key on demand from a cloud KMS (e.g. AWS KMS
signing or Azure Key Vault) with short-lived, per-run grants, eliminating any
long-lived secret from CI. This needs a cloud account and breaks offline signing,
so it is intentionally not wired up in v0.0.1.

### Log Record Signing Keys (TPM-backed, phase 1)

The engine signs audit records with a **TPM-provisioned** Ed25519 key
(author ruling 2026-08-18): the private key is created inside the TPM,
**non-exportable**, and all signing happens in hardware.

| Platform | Backend | Status |
|:-:|:-:|:-:|
| Windows | CNG (TPM-based key, zero new dependencies) | Phase 1 |
| Linux | `tpm2-tss` | Phase 1 |
| macOS | Secure Enclave (equivalent hardware boundary) | Phase 1 |

Policy: `enable_signature = true` without an available TPM **refuses startup
with an explicit error** — no silent downgrade to a software key. The
`KeyProvider` plugin interface is the existing extension hole for external
HSM/KMS backends. Phase 2+ (PCR measurement, attestation, monotonic
rollback counter) is stubbed for a post-v1.0 review.

The default signing granularity is **per-record** (audit favors security over
throughput); block-level Merkle signing (`audit_block_size > 1`) is an
optional optimization. A block size is only promoted to a documented default
after an authoritative Criterion sweep on the real TPM backend.

#### Key rotation lifecycle

(illustrative diagram):

```mermaid
flowchart TD
    P1["Phase 1: Initiate Rotation<br/>New TPM key pair created<br/>Old key enters grace period"] --> P2["Phase 2: Grace Period (default 7 days)<br/>Both keys active simultaneously<br/>Old key signs in-flight records<br/>New key signs newly submitted records<br/>Verifier accepts records signed by EITHER key"]
    P2 --> P3["Phase 3: Rotation Complete<br/>Old key revoked (added to CRL)<br/>All new records signed with new key<br/>Old-key records still verifiable with old public key"]
    P3 --> P4["Phase 4: Emergency Revocation (optional)<br/>Key fingerprint added to CRL immediately<br/>All records signed by revoked key fail verification"]
```

#### Certificate revocation list (CRL) — record signing

```rust
// (matches core/src/security/key_rotation.rs — the record-signing CRL design)
pub struct CrlEntry {
    pub fingerprint: KeyFingerprint,   // SHA-256 of the revoked key ([u8; 32])
    pub revoked_at: u64,               // Unix timestamp (seconds)
    pub reason: CrlReason,
}

pub enum CrlReason {
    Compromised,   // key leaked (emergency — sysmon CRITICAL)
    Superseded,    // replaced by a newer key after rotation
    Deactivated,   // disabled by an administrator (not compromised)
}
```

The plugin-signing revocation list (`plugins/official/trust-anchors/revoked.txt`)
shares the same `CrlReason` vocabulary (`compromised`, `superseded`, `deactivated`)
but is enforced by the plugin loader, which is live in v0.0.1.


---

## Audit Verification

### `dologctl verify-log`

Verify the integrity of a WORM audit log (takes a single SIF/WORM file path — no `--path`/`--verbose` options). The 64B Ed25519 signatures live in the companion sidecar `audit.log.sig`; pass it with `--sidecar` (in per-record mode) so non-repudiation can be checked:

```bash
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm \
    --sidecar /var/lib/dologger/audit/audit-000001.sig
```

Output (illustrative — the actual v0.0.1 output is a "Verification Results" summary; see the [dologctl Command Reference](guides/DologctlCommandReference.md)):

```
Log File Verification
  File: /var/lib/dologger/audit/audit-000001.worm
  Records parsed: 4
Verification Results
  Total records:     4
  Content hashes:    4 valid, 0 mismatched (100.0% ok)
  Chain links:       3 valid, 0 broken (100.0% ok)
  Signatures:        4 valid, 0 invalid (100.0% ok)
  LSN continuity:    PASS - no gaps detected
VERIFICATION PASSED - all checks OK
```

When a problem is found, per-record details are printed on stderr, e.g. `CHAIN BROKEN LSN 3 -> 4: ...`, `LSN GAP Expected 3, found 5 (missing 2)`, or `TAMPERED LSN 5 - signature invalid`, and the command exits non-zero with `VERIFICATION FAILED`.

### What It Verifies

| Check | What It Means |
|:-:|:-:|
| content_hash | The record content matches its recomputed SHA-256 (memory/runtime tamper evidence) |
| Ed25519 signature (sidecar) | The record was authored by the TPM-held key (non-repudiation) |
| prev_hash chain | The record is in its original position in the sequence |
| LSN monotonicity | Records are in correct chronological order |
| Gap detection | Missing records are identified and reported |

### `dologctl verify-anchor`

Verify external anchoring hashes (planned):

```bash
# Takes the anchor JSON file path + --pubkey; v0.0.1 has no
# --anchor-file/--worm-path options
dologctl verify-anchor anchors/2026-08.json --pubkey "$(cat pubkey.hex)"

# Compares locally computed Merkle roots with
# externally published anchor hashes
```

### Automated Verification

Set up a daily cron job:

```bash
# /etc/cron.daily/dologger-audit-verify
#!/bin/bash
# `-o json` is the global output-format flag; verify-log takes a single file path
REPORT=$(dologctl verify-log /var/lib/dologger/audit/audit-000001.worm \
         --sidecar /var/lib/dologger/audit/audit-000001.sig -o json)
if echo "$REPORT" | jq -e '.status == "failed"' > /dev/null; then
    echo "AUDIT INTEGRITY FAILURE: $REPORT" | \
        mail -s "CRITICAL: DoLogger audit chain broken" security@example.com
fi
```

(Note: `verify-log` JSON output contains `status: "passed"/"failed"`, `total_records`, `broken_chain_links`, `lsn_gaps`, and `signatures` fields.)

### WORM File Handling

| Operation | Command |
|:-:|:-:|
| List WORM segments | `ls -la /var/lib/dologger/audit/` |
| Verify chain | `dologctl verify-log /var/lib/dologger/audit/audit-000001.worm --sidecar /var/lib/dologger/audit/audit-000001.sig` |
| Export audit records | *(pseudocode — `dologctl audit export` is a planned feature)* |
| Check latest LSN | `dologctl verify-log /var/lib/dologger/audit/audit-000001.worm -o json` |

The companion signature file follows the same WORM lifecycle as its audit
file and **must be archived together** — without it, non-repudiation cannot
be verified offline.

### Tamper Detection

The LSN + content_hash chain provides self-verifying tamper evidence
(author ruling 2026-08-18: the primary battlefield is **memory/runtime**
tampering; disk tampering is additionally covered by WORM semantics):

- **Record modification**: The content_hash will not match the recomputed value, and the Ed25519 signature will not verify -- the record content changed since signing.
- **Record deletion**: The prev_hash of the next record will not match the expected value -- the chain is broken.
- **Record insertion**: The prev_hash will not match, and the LSN will not be monotonic.
- **Record reordering**: Both prev_hash and LSN checks will fail.
- **Forgery (re-signing)**: Impossible without the TPM-held key -- an attacker cannot mint valid signatures even with full memory access.

---

## Incident Response Runbooks

### Incident: Audit Signature Failure

**Severity**: CRITICAL

**Symptoms**:
- `SIGNATURE_FAILURE` sysmon event
- `dologctl verify-log` reports `FAIL` on one or more records

**Response Procedure**:

1. **Identify affected records:**
   ```bash
   dologctl verify-log /var/lib/dologger/audit/audit-000001.worm 2>&1 | grep FAIL
   ```

2. **Assess scope:**
   - Single-record failure: likely disk corruption or bit flip
   - Multiple sequential failures: possible tampering
   - All records failing: key mismatch or key compromise

3. **Investigate root cause:**
   - Check system logs for disk I/O errors around affected timestamps
   - Verify file permissions: was the WORM file writable?
   - Check for root/sudo activity matching the affected time window

4. **Contain (if tampering suspected):**
   - Isolate the host from the network
   - Preserve forensic image of the affected files
   - If the plugin signing key may be compromised, rotate it now: see the [Emergency revocation runbook](#emergency-revocation-runbook-compromise) (append the fingerprint to `plugins/official/trust-anchors/revoked.txt`, replace the `DOLOGGER_PLUGIN_SIGNING_KEY` secret). The record-signing `dologctl key rotate` command remains planned.

5. **Report:**
   - File a security incident report
   - Preserve WORM files as forensic evidence
   - The cryptographic chain provides tamper evidence for investigation

### Incident: Sandbox Violation

**Severity**: CRITICAL

**Symptoms**:
- `SANDBOX_VIOLATION` sysmon event
- Plugin thread terminated with SIGSYS

**Response Procedure**:

1. **Identify the violating plugin:**

   (illustrative example — sandbox enforcement is not implemented yet; real sysmon events use the `sysmon_version`/`error_code`/`category`/`description`/`timestamp_ms`/`severity` shape shown in the Monitoring section):

   ```json
   {"event":"SANDBOX_VIOLATION","plugin":"untrusted-plugin","syscall":"fork","action":"KILL","tid":12345}
   ```

2. **Assess:**
   - Is this a known plugin behaving unexpectedly? (misclassification)
   - Is this an unknown plugin? (possible compromise)

3. **Decision tree:**
   - Misclassified Yellow/Blue plugin: upgrade trust color after code review and re-signing
   - Malicious or compromised plugin: remove immediately, rotate all keys
   - Unknown plugin: quarantine binary for analysis

4. **Prevent recurrence:**
   - Audit all installed plugins: `dologctl plugin list`
   - Review plugin vetting process
   - Consider disabling Red plugins entirely (`allow_red_plugins = false`)

### Incident: Log Loss

**Severity**: HIGH

**Symptoms**:
- `PIPELINE_DROP` or `EMERGENCY_BUFFER` events
- LSN gaps in audit chain
- Records missing from output files

**Response Procedure**:

1. **Triage:**
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .ring_buffer
   # Check pct_used, drops_total, emergency_spills
   ```

2. **Identify bottleneck:**
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .sinks
   ```

3. **Mitigate:**
   ```bash
   # pseudocode/illustrative — the /sink/disable endpoint is planned; the
   # v0.0.1 control plane only has /status, /level, /reload
   # curl -X POST http://127.0.0.1:9090/sink/disable -d '{"sink": "kafka"}'
   ```

4. **Increase capacity:**
   ```bash
   # Double ring buffer size (requires restart)
   sed -i 's/ring_buffer_size = 262144/ring_buffer_size = 524288/' dologger.toml
   sudo systemctl restart dologger
   ```

5. **Recover:**
   - Emergency buffer files auto-replay on recovery
   - Verify integrity post-recovery: `dologctl verify-log /var/lib/dologger/audit/audit-000001.worm`

### Incident: Performance Degradation

**Severity**: MEDIUM

**Symptoms**:
- Application latency increasing (blocking on AUDIT records)
- Ring buffer utilization trending upward over hours
- `PIPELINE_BACKLOG` event frequency increasing

**Response Procedure**:

1. **Check current profile:**
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .profile
   ```

2. **Check sink health:**
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .sinks
   ```

3. **Check if signing is unexpectedly enabled:**
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .signature_enabled
   # Ed25519 signing adds ~17 us per record
   ```

4. **Check disk I/O:**
   ```bash
   iostat -x 1
   # High await times indicate storage bottleneck
   ```

5. **Mitigation:**
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl -X POST http://127.0.0.1:9090/level -d '{"level": "ERROR"}'
   ```

### Post-Incident Diagnostic Collection

After any incident, capture a diagnostic snapshot:

```bash
# (illustrative — the diag command is a planned CLI feature, not yet available)
dologctl diag collect --output post-incident-$(date +%Y%m%d-%H%M%S).tar.gz
```

This archive contains:
- `dologger_internal.log` (full diagnostic log)
- Active configuration (sensitive values redacted)
- Plugin load manifest with versions
- Ring buffer statistics snapshot
- OS resource limits (`ulimit -a` equivalent)

---

## Compliance Configuration

### Available Templates

| Template | File | Activates | Framework |
|:-:|:-:|:-:|:-:|
| GDPR | `compliance/gdpr.toml` | All 6 non-downgradable items | EU General Data Protection Regulation |
| HIPAA | `compliance/hipaa.toml` | All 6 non-downgradable items | US Health Insurance Portability and Accountability Act |
| PCI DSS | `compliance/pci-dss.toml` | All 6 non-downgradable items | Payment Card Industry Data Security Standard |

### Compliance Template Contents

Every compliance template activates all six non-downgradable security items:

| Item | Value | Rationale |
|:-:|:-:|:-:|
| `enable_signature` | `true` | Non-repudiation -- cryptographically verifiable log records |
| `escape_html` | `true` | Log injection prevention -- CRLF and ANSI escape neutralization |
| `[sinks.audit]` (worm sink) | `durability = "media_with_fua"` | Immutability -- log records cannot be deleted or modified |
| `fsync_on_write` | `true` | Durability -- records committed to media before acknowledgment |
| `require_tls` | `true` | Transport security -- all network sinks use TLS 1.2+ |
| `sign_ring2` | `true` | Verified extension integrity -- plugin-provided fields are cryptographically bound |

### Applying a Compliance Template

```bash
# Validate the compliance template itself
dologctl config validate --config compliance/gdpr.toml --strict

# Validate your production config (merged with the template by hand, or use
# the template as a starting point)
dologctl config validate --config /etc/dologger/gdpr-production.toml --strict

# (illustrative — a dedicated merge command is planned, not yet available)
dologctl config merge \
    --base /etc/dologger/default.toml \
    --overlay compliance/gdpr.toml \
    --output /etc/dologger/gdpr-production.toml
```

### GDPR Configuration Summary

(illustrative summary of the values in `compliance/gdpr.toml`):

```
performance_profile = "prod-audit"
level               = "AUDIT"
enable_signature    = true    (non-downgradable)
[sinks.audit]                 # WORM immutability (non-downgradable)
type                = "worm"
durability          = "media_with_fua"
sign_ring2          = true    (non-downgradable)
escape_html         = true    (non-downgradable)
fsync_on_write      = true    (non-downgradable)
require_tls         = true    (non-downgradable)
shutdown_policy     = "graceful"
shutdown_timeout_ms = 10000
```

| GDPR Article | DoLogger Feature |
|:-:|:-:|
| Art. 5(1)(f) | Ed25519 signatures + CRC32C integrity check |
| Art. 15 | Ring 2 field signing (user.id, session.id) for data subject access records |
| Art. 30 | WORM audit log as records of processing activities |
| Art. 32 | Encryption in transit (TLS), integrity protection (signatures), resilience (ring buffer + emergency spill) |
| Art. 33-34 | Signed audit trail as evidence for breach notifications |
| Art. 35 | Compliance templates as technical basis for DPIA |
| Art. 58 | Verifiable audit chains for supervisory authority inspections |

### HIPAA Configuration Summary

| HIPAA Rule | DoLogger Feature |
|:-:|:-:|
| 164.312(b) Audit controls | WORM + Ed25519 + LSN chain for ePHI access records |
| 164.312(c)(2) Integrity | Ed25519 cryptographic mechanism to verify ePHI audit integrity |
| 164.312(e)(1) Transmission | TLS 1.2+ enforced for all network sinks |

### PCI DSS Configuration Summary

| PCI DSS Requirement | DoLogger Feature |
|:-:|:-:|
| 10.2 Automated audit trails | LSN chain + WORM immutable audit trail |
| 10.5 Secure audit trails | Ed25519 signatures (10.5.1-10.5.2), WORM immutability (10.5.5) |
| 4.1 Strong cryptography | TLS 1.2+ required for all network sinks |

### Legal Disclaimer

**These compliance templates are technical starting points only.** They do NOT guarantee regulatory compliance. You MUST consult your legal counsel and perform a full assessment before deploying to production. The templates:
- Set all security-relevant configuration to their most restrictive values
- Cannot be loosened by lower-priority configuration layers (non-downgradable)
- Must be validated with: `dologctl config validate --config compliance/<framework>.toml --strict`

---

## Sandbox Configuration Per Trust Level

### Trust Level Comparison

| Capability | Blue | Yellow | Red |
|:-:|:-:|:-:|:-:|
| Memory access | Full | Full | Full |
| File I/O | Full read/write | Read + Write | **Denied** |
| Network | Full | **Denied** | **Denied** |
| Process creation | Allowed | **Denied** | **Denied** |
| Signal handling | Allowed | Allowed | **Denied** |
| Field writes | Ring 2 (`verified.*`) | Ring 2 (`verified.*`) | Ring 3 (`ext.*`) |

### Linux Sandbox (seccomp-bpf)

(illustrative allowlist sketch — sandbox enforcement is not implemented yet):

```
Yellow plugin syscall allowlist:
  Memory:     mmap, munmap, mprotect, brk, madvise
  Threading:  futex, clone, set_robust_list
  Time:       clock_gettime, gettimeofday, nanosleep
  Signal:     rt_sigaction, rt_sigreturn, tgkill
  SystemInfo: uname, getpid, gettid, getrandom
  File I/O:   open, openat, read, write, close, lseek, fstat, fsync
  Network:    (none)
  Process:    (none)

Red plugin syscall allowlist:
  Memory:     mmap, munmap, mprotect, brk
  Threading:  futex, clone
  Time:       clock_gettime, gettimeofday
  SystemInfo: uname, getpid, getrandom
  Signal:     (none)
  File I/O:   (none)
  Network:    (none)
  Process:    (none)
```

Violation: `SECCOMP_RET_KILL_PROCESS` -- thread terminated. Emits `SANDBOX_VIOLATION` sysmon event.

### Windows Sandbox (AppContainer)

- **Yellow**: LowBox token with `WIN://NO_NETWORK` and `WIN://NO_PROCESS_CREATION` capability SIDs withheld
- **Red**: Full AppContainer isolation, only `WIN://LOWBOX` base capability

### macOS Sandbox (App Sandbox)

Sandbox profiles applied via `sandbox_init(3)` with seatbelt/SBPL rules per trust tier.

### Enabling Red Plugins

Red plugins are disabled by default. Enable with (illustrative — red-plugin sandbox enforcement is not implemented yet):

```toml
[dologger]
allow_red_plugins = true
```

This should only be done in development environments. Production should never enable Red plugins.

### Sandbox Violation Audit

Monitor sandbox violations in real time:

```bash
# Watch sysmon events for sandbox violations (illustrative — sandbox
# enforcement is not implemented yet; real events use the "category" field)
tail -f dologger_internal.log | jq 'select(.category == "SANDBOX_VIOLATION")'
```

---

## Performance Regression Detection

### Baseline Benchmarks

Establish a baseline on your production hardware (the v0.0.1 repository ships `latency`, `throughput`, and `latency_percentiles` benchmarks):

```bash
# Run all benchmarks and save the results
cargo bench --bench latency -- --save-baseline prod-baseline
cargo bench --bench throughput -- --save-baseline prod-baseline
cargo bench --bench latency_percentiles -- --save-baseline prod-baseline
```

### Regression Detection

After a configuration change or engine update, compare against the baseline:

```bash
cargo bench --bench latency -- --baseline prod-baseline
```

A regression is flagged when:
- Hot path latency increases by >20% from baseline
- Throughput decreases by >20% from baseline
- P99 latency increases by >50% from baseline

### Runtime Performance Monitoring

```bash
# pseudocode/illustrative — the control plane is not started in v0.0.1
# watch -n 5 'curl -s http://127.0.0.1:9090/status | jq .pipeline'

# Key metrics today: status, level, profile, plugins, signature_enabled.
# Richer metrics (pipeline counters, ring buffer usage) are planned.
```

### Performance Regression Response

If performance degrades after a change:

1. **Compare profiles**: Has `performance_profile` been changed?
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .profile
   ```

2. **Check signing overhead**: Is Ed25519 signing unexpectedly enabled?
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .signature_enabled
   ```

3. **Check sink health**: A slow downstream causes backpressure.
   ```bash
   # pseudocode/illustrative — the control plane is not started in v0.0.1
   # curl http://127.0.0.1:9090/status | jq .sinks
   ```

4. **Check disk I/O**: File/WORM sinks are I/O bound.
   ```bash
   iostat -x 1
   ```

5. **Rollback if needed**: Restore previous configuration and restart.

### Performance Profile Override

Individual profile values can be overridden without changing profiles:

```toml
[dologger]
performance_profile = "prod-performance"
ring_buffer_size = 524288            # Override default 262144
batch_size = 512                     # Override default 256
```

Non-downgradable items cannot be relaxed via overrides.

### Performance Baseline Reference

| Profile | Expected P50 Latency | Expected Throughput | Max Ring Buffer Usage |
|:-:|:-:|:-:|:-:|
| `dev` | < 200 ns | > 500K rec/s | < 90% |
| `balanced` | < 150 ns | > 1M rec/s | < 70% |
| `prod-performance` | < 120 ns | > 5M rec/s | < 50% |
| `prod-audit` | < 20 us | > 50K rec/s | < 50% |

These are targets, not guarantees. Actual performance depends on hardware, record size, sink configuration, and plugin overhead.

---

## Complete Specification

For the authoritative design document covering every architecture decision, API, and security property, see the [Architecture Reference](ArchitectureReference.md).

For detailed deployment, monitoring, and recovery procedures: [Operations Manual](guides/OperationsManual.md).

For the complete threat model and cryptographic design: [Security Whitepaper](guides/SecurityWhitepaper.md).
