# DoLogger Architecture Reference

> **Version**: v0.2.0 | **Last Updated**: 2026-08-12 | **Target Audience**: Core Developers, Plugin Authors, Systems Engineers
>
> **Purpose**: Definitive reference for the DoLogger engine internals -- pipeline architecture, lock-free data structures, cryptographic audit chain, security model, sink fan-out, backpressure, and performance tuning. Assume familiarity with the [Integration Guide](IntegrationGuide.md).
>
> 🌐 **语言 / Language**: [English](ArchitectureReference.md) | [中文：架构参考手册](../zh_CN/ArchitectureReference.md)
>
> **Reading Path**: Start with the [Pipeline Architecture](#pipeline-architecture) diagram, then dive into your area of interest. Plugin developers should focus on [Plugin VTable Specification](#plugin-vtable-specification).

---

## Table of Contents

1. [Before You Start](#before-you-start)
2. [Pipeline Architecture](#pipeline-architecture)
3. [Ring Buffer Design and Lock-Free Guarantees](#ring-buffer-design-and-lock-free-guarantees)
4. [Audit Chain: Ed25519 + LSN + prev_hash](#audit-chain-ed25519--lsn--prev_hash)
5. [Security Model: Ring 0-3 Permissions and Three-Color Trust](#security-model-ring-0-3-permissions-and-three-color-trust)
6. [Sink Fan-Out and Fallback Chains](#sink-fan-out-and-fallback-chains)
7. [Backpressure System](#backpressure-system)
8. [Emergency Buffer and Recovery](#emergency-buffer-and-recovery)
9. [Thread Pool Architecture](#thread-pool-architecture)
10. [Plugin VTable Specification](#plugin-vtable-specification)
11. [SIF Binary Format Overview](#sif-binary-format-overview)
12. [Performance Benchmarks and Tuning](#performance-benchmarks-and-tuning)

---

## Before You Start

### Prerequisites

- Familiarity with lock-free concurrent programming (CAS, atomic ordering)
- Understanding of Rust's ownership model and FFI
- Knowledge of Ed25519 signatures and SHA-256 hash chains
- The [Integration Guide](IntegrationGuide.md) for application-level usage

### Key Terminology

| Term | Definition |
|:-:|:-:|
| **Record** | A single log entry flowing through the engine |
| **Ring buffer** | Lock-free MPSC queue for producer-to-consumer handoff |
| **Pipeline** | 7-stage processing chain: PreFilter -> Filter -> Field -> Assembly -> Process -> Format -> Sink |
| **Object pool** | Pre-allocated Record pool using a Treiber stack |
| **LSN** | Log Sequence Number -- monotonically increasing audit counter |
| **prev_hash** | SHA-256 hash linking each audit record to its predecessor |
| **WORM** | Write-Once-Read-Many -- immutable audit file storage |
| **VTable** | Virtual method table -- C ABI function pointer struct per plugin type |

---

## Pipeline Architecture

### System Overview

```
                          HOST APPLICATION
                               │
                    dologger_log() / dologger_logv()
                               │
                         102 ns P50 (CAS push)
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                     LOCK-FREE MPSC RING BUFFER                                │
│                                                                              │
│  ┌──────────────────────┐  ┌──────────────────────┐  ┌────────────────────┐  │
│  │   Normal Partition   │  │   AUDIT Partition    │  │  Cooperative       │  │
│  │       (90%)          │  │       (10%)          │  │  Helping           │  │
│  │  CAS-based enqueue   │  │  Dedicated, isolated │  │  Producer drains   │  │
│  │  Wait-free producers │  │  Never drops         │  │  at >90% full      │  │
│  └──────────┬───────────┘  └──────────┬───────────┘  └────────────────────┘  │
│             │                         │                                      │
└─────────────┬─────────────────────────┬──────────────────────────────────────┘
              │                         │
              ▼                         ▼
┌──────────────────────────┐  ┌───────────────────────────────────────────────┐
│  REGULAR PIPELINE         │  │  AUDIT PIPELINE (independent consumer)        │
│                           │  │                                               │
│  Stage 0: PreFilter       │  │  Ring Buffer → Direct Processing              │
│    PolicyProvider plugins │  │    (no plugin stages -- bypasses all)          │
│    (rate_limiter, level)  │  │                                               │
│         │                 │  │  Ed25519 Sign (mandatory)                     │
│         ▼                 │  │         │                                     │
│  Stage 1: Filter          │  │         ▼                                     │
│    Filter plugins         │  │  Dual-Write Sinks:                            │
│         │                 │  │    → WORM Sink  (LSN chain, prev_hash)        │
│         ▼                 │  │    → Security Sink (0600, plugin bypass)      │
│  Stage 2: FieldProvider   │  │                                               │
│    HostInfo + Field       │  └───────────────────────────────────────────────┘
│         │                 │
│         ▼                 │
│  Stage 3: Assembly        │
│    Core: LSN assign       │
│    + Ed25519 sign         │
│    + CRC32C Ring 3 check  │
│    + Secret detection     │
│         │                 │
│         ▼                 │
│  Stage 4: Processing      │
│    Processor plugins      │
│    (transform, redact)    │
│         │                 │
│         ▼                 │
│  Stage 5: Formatting      │
│    Formatter plugins      │
│    (JSON, text, SIF)      │
│         │                 │
│         ▼                 │
│  Stage 6: Sink Fan-Out    │
│    IOSink plugins         │
│    (parallel writes)      │
│         │                 │
│         ▼                 │
│  11 sink types available  │
└───────────────────────────┘
```

### Stage Details

| Stage | Index | Plugins | Can Drop? | Can Modify? | Core Operations |
|:-:|:-:|:-:|:-:|:-:|:-:|
| PreFilter | 0 | PolicyProvider | Yes | No | Rate limiting, level gating |
| Filter | 1 | Filter | Yes | No | Content-based filtering |
| FieldProvider | 2 | FieldProvider, HostInfoProvider | No | Ring 1 write | Host/container/cloud metadata injection |
| Assembly | 3 | Core only | No | Ring 0+1 write | LSN assign, Ed25519 sign, CRC32C verify, secret detection |
| Processing | 4 | Processor | Yes | Ring 2+3 write | Transform, redact, enrich |
| Formatting | 5 | Formatter | No | Read-only | Serialize to JSON/text/SIF |
| Sink | 6 | IOSink | No | Read-only | Write to external destinations |

### Record Lifecycle

```
Object Pool (Treiber stack)
       │
       ├─ alloc() ──▶ Record (pre-zeroed)
       │                 │
       │        Application fills Ring 1 fields
       │                 │
       │        dologger_log() → CAS push into ring buffer
       │                 │
       │        Consumer drains batch
       │                 │
       │        Pipeline stages 0-6 process
       │                 │
       │        Formatter serializes
       │                 │
       │        Sink writes to destination(s)
       │                 │
       └─ free() ◀──────┘
```

---

## Ring Buffer Design and Lock-Free Guarantees

### Architecture

```
Producer Threads (multiple)          Consumer Thread (single per domain)
       │                                         │
       │  CAS on producer_sequence               │
       ▼                                         ▼
┌──────────────────────────────────────────────────────────────┐
│  Slot 0 │ Slot 1 │ Slot 2 │ ... │ Slot N-1                  │
│  [data] │ [data] │ [data] │     │ [data]                    │
│  [seq]  │ [seq]  │ [seq]  │     │ [seq]                     │
└──────────────────────────────────────────────────────────────┘
       ▲                                         │
       │            Capacity = 2^k               │
       │            Mask = 2^k - 1               │
       │                                         │
       └───── index = sequence & mask ───────────┘
```

### Design Properties

| Property | Guarantee |
|:-:|:-:|
| **Producers** | Wait-free -- CAS slot claim, no mutex, no spin-loop |
| **Consumer** | Batch drain -- single thread per domain, never contends with producers |
| **Cache-line padding** | Each `RingSlot` is `#[repr(C, align(64))]` to prevent false sharing |
| **Power-of-two capacity** | Bitmask modulo (`index = seq & mask`) avoids division |
| **Sequence coordination** | Two atomic counters: `producer_sequence` and `consumer_sequence` |

### Enqueue Algorithm (Producer)

```
producer_push(record):
  loop:
    seq = producer_sequence.fetch_add(1, Relaxed)   // Claim next slot
    slot = &slots[seq & mask]
    while slot.sequence != seq:                      // Wait for slot to be free
      spin_loop()
    slot.data = record                                // Write
    slot.sequence.store(seq + 1, Release)            // Publish
    return OK
```

### Dequeue Algorithm (Consumer)

```
consumer_drain(batch_size):
  for i in 0..batch_size:
    consumer_seq = consumer_sequence.load(Relaxed)
    slot = &slots[consumer_seq & mask]
    if slot.sequence != consumer_seq + 1:             // Slot not ready yet
      break
    record = slot.data.take()
    slot.sequence.store(consumer_seq + capacity, Release)  // Free slot
    consumer_sequence.fetch_add(1, Release)
    process(record)
  return count
```

### Object Pool (Treiber Stack)

Records are pre-allocated in a `RecordPool` to avoid heap allocation on the hot path:

```
Allocation:
  CAS(pool.head, current_head, nodes[current_head].next)
  → return &mut nodes[current_head].record

Deallocation:
  loop:
    current_head = pool.head
    nodes[node].next = current_head
    if CAS(pool.head, current_head, node): break
```

### Concurrency Model Summary

| Component | Mechanism | Notes |
|:-:|:-:|:-:|
| Ring buffer (producer) | Lock-free CAS | Contention under >8 threads (single CAS cursor) |
| Ring buffer (consumer) | Single-threaded | One consumer per domain, no contention |
| Object pool | Lock-free Treiber stack | CAS on head pointer |
| Config store | `Arc<RwLock<Config>>` + CoW snapshot | Read-heavy, write-rare |
| Plugin registry | `Arc<RwLock<PluginRegistry>>` | Cold path only (load/unload) |
| Error state | Thread-local storage | `thread_local! { RefCell<DologgerError> }` |

### Known Limitation

The ring buffer uses a single CAS cursor for all producers. Under heavy multi-threaded submission (>8 concurrent producer threads), CAS contention can become a bottleneck. A sharded ring buffer with per-thread partitions is planned for M4.

---

## Audit Chain: Ed25519 + LSN + prev_hash

### Chain Structure

```
Record(1):
  lsn       = 1
  prev_hash = SHA-256(0x00...00)       // Genesis block -- all zeros
  signature = Ed25519_Sign(secret_key, Ring0+Ring1 fields)

Record(2):
  lsn       = 2
  prev_hash = SHA-256(Record(1).signature || Record(1).lsn)
  signature = Ed25519_Sign(secret_key, Ring0+Ring1 fields)

Record(3):
  lsn       = 3
  prev_hash = SHA-256(Record(2).signature || Record(2).lsn)
  signature = Ed25519_Sign(secret_key, Ring0+Ring1 fields)
```

### Verification Algorithm

```
verify_chain(records):
  for i = 0 to len(records)-1:
    1. Verify Ed25519 signature:
       if !pubkey.verify(records[i].signature, serialize(Ring0+Ring1)):
         return FAIL at i

    2. Verify prev_hash chain (if i > 0):
       expected = SHA-256(records[i-1].signature || records[i-1].lsn)
       if records[i].prev_hash != expected:
         return CHAIN_BREAK at i

    3. Verify LSN monotonicity:
       if records[i].lsn <= records[i-1].lsn:
         return LSN_ORDER_VIOLATION at i

    4. Detect gaps:
       if records[i].lsn > records[i-1].lsn + 1:
         mark GAP from (records[i-1].lsn+1) to (records[i].lsn-1)

  return OK with summary
```

### LSN Gap Handling

- **Reorder window (200 ms)**: Out-of-order records within 200 ms are filled in. No gap is marked.
- **Window exceeded**: A `GAP_MARKER` record is written to the WORM file, and a `LSN_GAP_DETECTED` sysmon event is emitted.
- **Non-AUDIT records**: Do not carry LSNs. Gaps are expected and non-malicious.

### Signature Coverage

| Fields | Integrity | Notes |
|:-:|:-:|:-:|
| Ring 0 | Ed25519 | Always signed |
| Ring 1 | Ed25519 | Always signed |
| Ring 2 | Ed25519 (optional) | Signed when `sign_ring2 = true` |
| Ring 3 | CRC32C only | Hardware-accelerated, not cryptographic |

### Cryptographic Performance

Measured on AMD Ryzen 9 7950X, single core, ed25519-dalek 2.0:

| Operation | Latency | Throughput |
|:-:|:-:|:-:|
| Ed25519 key generation | ~24 us | ~41,000 keys/s |
| Ed25519 signing | ~16.96 us | ~58,000 sigs/s |
| Ed25519 verification | ~48 us | ~20,800 verifs/s |
| SHA-256 (64 bytes) | ~120 ns | ~8.3M hashes/s |
| CRC32C (64 bytes) | ~3 ns | ~330M checks/s |

### External Anchoring (M4)

Periodic Merkle root hashes are published to immutable external storage (S3, blockchain) to provide long-term tamper resistance:

```
// Every N records, compute a Merkle root over the signature chain
let merkle_root = compute_merkle_root(records[l..r]);
send_to_external_anchor(merkle_root, lsn_range = [l, r]);
```

---

## Security Model: Ring 0-3 Permissions and Three-Color Trust

### Field Permission Rings

```
┌──────────────────────────────────────────────────────────────┐
│                          RING 3                              │
│  Untrusted Extensions  (ext.* namespace)                     │
│  Write: Any plugin (including Red)                           │
│  Read:  Any plugin                                           │
│  Integrity: CRC32C hardware checksum (~0.5 cycles/byte)      │
│  NOT covered by Ed25519 signature                            │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │                        RING 2                          │  │
│  │  Verified Extensions  (verified.* namespace)           │  │
│  │  Write: Blue + Yellow plugins only                     │  │
│  │  Read:  Any plugin                                     │  │
│  │  Integrity: Ed25519 (when sign_ring2=true)             │  │
│  │  Audit: Each write appends audit_tags entry            │  │
│  │                                                        │  │
│  │  ┌──────────────────────────────────────────────────┐  │  │
│  │  │                      RING 1                      │  │  │
│  │  │  System Trusted Fields                           │  │  │
│  │  │  Write: Core engine + HostInfoProvider           │  │  │
│  │  │  Read:  All plugins (read-only)                  │  │  │
│  │  │  Integrity: Ed25519 (always)                     │  │  │
│  │  │  Fields: level, message, host, process,          │  │  │
│  │  │          thread_id, environment                  │  │  │
│  │  │                                                  │  │  │
│  │  │  ┌────────────────────────────────────────────┐  │  │  │
│  │  │  │                RING 0                      │  │  │  │
│  │  │  │  Engine Core -- Immutable                  │  │  │  │
│  │  │  │  Write: Core engine ONLY                   │  │  │  │
│  │  │  │  Read:  Formatter + Sink (read-only)       │  │  │  │
│  │  │  │  Integrity: Ed25519 (always)               │  │  │  │
│  │  │  │  Fields: id, timestamp, signature,         │  │  │  │
│  │  │  │          origin_lsn                        │  │  │  │
│  │  │  └────────────────────────────────────────────┘  │  │  │
│  │  └──────────────────────────────────────────────────┘  │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

### Three-Color Plugin Trust Model

| Property | Blue (Full Trust) | Yellow (Partial Trust) | Red (Zero Trust) |
|:-:|:-:|:-:|:-:|
| Signing | Ed25519 required | Recommended | Not required |
| Sandbox | None | seccomp-bpf / AppContainer | Maximum isolation |
| File I/O | Full | Read + Write | Denied |
| Network | Full | Denied | Denied |
| Process spawn | Allowed | Denied | Denied |
| Field writes | Ring 2 (`verified.*`) | Ring 2 (`verified.*`) | Ring 3 (`ext.*`) |
| Field reads | Rings 0-3 | Rings 0-3 | Rings 0-3 |
| Dynamic load | Allowed | Allowed | Config-gated (`allow_red_plugins`) |

### seccomp-bpf Syscall Allowlist (Linux)

```
                    Blue        Yellow      Red
Memory              ALL         ALL         ALL
Threading           ALL         ALL         ALL
Time                ALL         ALL         ALL
Signal              ALL         ALL         DENIED
File I/O            ALL         ALL         DENIED
Network             ALL         DENIED      DENIED
Process             ALL         DENIED      DENIED
```

Violation action: `SECCOMP_RET_KILL_PROCESS` -- the plugin thread is terminated by the kernel with SIGSYS. A `SANDBOX_VIOLATION` sysmon event is emitted.

---

## Sink Fan-Out and Fallback Chains

### Fan-Out Architecture

```
Pipeline Output (formatted record)
           │
           ▼
    ┌──────────────────────┐
    │    Sink Dispatcher   │
    │  (parallel dispatch) │
    └──────┬───────┬───────┬──────┬──────┬──────┬──────┐
           │       │       │      │      │      │      │
           ▼       ▼       ▼      ▼      ▼      ▼      ▼
        Console  File   Callback Kafka Syslog Webhook SQLite ...
```

Each enabled sink receives a copy of every formatted record. Dispatch is parallel across the `io_pool` thread pool.

### Built-in Sinks (11 Total)

| Sink | Type | TLS | Use Case |
|:-:|:-:|:-:|:-:|
| Console | `sink_console` | N/A | Development, debugging |
| File | `sink_file` | N/A | Local file output with rotation |
| Callback | `sink_callback` | N/A | In-process custom processing |
| Kafka | `sink_kafka` | TLS + SASL | Centralized log aggregation |
| Syslog | `sink_syslog` | TLS (RFC 5425) | Traditional syslog infrastructure |
| Webhook | `sink_webhook` | HTTPS | REST API log ingestion |
| SQLite | `sink_sqlite` | N/A | Local structured log storage |
| WORM | `sink_worm` | N/A | Immutable audit log storage |
| Security File | `sink_security` | N/A | Isolated audit output (0600, plugin bypass) |
| Shared Memory | `sink_shm` | N/A | Sidecar inter-process communication |
| OpenTelemetry | `sink_otel` | HTTPS | OTLP/HTTP observability pipelines |

### Fallback Chains

When a primary sink fails, a fallback chain provides degraded-mode output:

```toml
[sinks.file]
type = "sink_file"
enabled = true
path = "/var/log/dologger/app.log"
fallback = "emergency_file"

[sinks.kafka]
type = "sink_kafka"
enabled = true
brokers = ["kafka1:9092"]
fallback = "file"            # If Kafka is down, write to file instead
```

```
Primary Sink (Kafka)
       │
       │ write failure
       ▼
Fallback Sink (File)
       │
       │ write failure
       ▼
Emergency Sink (Console stderr)
```

### Circuit Breaker Per Remote Sink

Each remote sink (Kafka, Syslog, Webhook) has an independent circuit breaker:

```
CLOSED ──(failures >= threshold)──▶ OPEN
  ▲                                    │
  │                              (timeout_ms)
  │                                    ▼
  └────(probe success)──── HALF_OPEN ◀─┘
       (probe failure)─────────────▶ OPEN
```

| Parameter | Default | AUDIT Override |
|:-:|:-:|:-:|
| `failure_threshold` | 5 consecutive failures | >= 3 |
| `timeout_ms` | 30,000 (30 seconds) | >= 60,000 |
| `half_open_max_requests` | 3 probes | 3 probes |

---

## Backpressure System

### Drop Strategies

When the ring buffer is full and the configured `block_timeout_ms` expires, records are dropped according to the strategy:

| Strategy | Behavior | Availability Impact |
|:-:|:-:|:-:|
| `drop_newest` | Discard the newly submitted record | Low -- producers never block |
| `oldest` | Discard the oldest unprocessed record | Low -- maintains freshness |
| `below_warn` | Drop only records below WARN level | Medium -- WARN+ always preserved |
| `below_error` | Drop only records below ERROR level | High -- ERROR+ always preserved |
| `never` | Block indefinitely (AUDIT domain only) | May stall the host |

### Backpressure Thresholds

```
Ring Buffer Occupancy:

   0% ──────────── normal operation ────────────▶ 50%
                                                     │
                                                     ▼
   50% ───── PIPELINE_BACKLOG (WARN sysmon) ────▶ 90%
                                                     │
                                                     ▼
   90% ──── Cooperative helping activates ───────▶ 95%
    (producer threads help drain inline)              │
                                                     ▼
   95% ─── Emergency buffer activates ───────────▶ 100%
    (spill to mmap file on disk)                      │
                                                     ▼
   100% ── Drop strategy applied ────────────────▶
    (drop_newest / oldest / below_warn / never)
```

### Cooperative Helping

At 90% occupancy, producer threads help drain the ring buffer inline before pushing their own record. This trades a small increase in submission latency for the prevention of buffer overflow:

```
if occupancy >= 90%:
  producer drains a small batch (16 records)
  producer then pushes its own record
```

### Performance Profile Binding

| Profile | `block_timeout_ms` | `drop_strategy` | AUDIT Behavior |
|:-:|:-:|:-:|:-:|
| `dev` | 100 | `drop_newest` | AUDIT blocks indefinitely |
| `prod-performance` | 3000 | `below_warn` | AUDIT blocks indefinitely |
| `prod-audit` | 3000 | `below_warn` | AUDIT blocks indefinitely |
| `balanced` | 2000 | `oldest` | AUDIT blocks indefinitely |

The AUDIT iron law overrides all profiles: AUDIT records never drop.

---

## Emergency Buffer and Recovery

### Activation

- **Trigger**: Ring buffer occupancy >= 95% for >5 seconds
- **Threshold managed by**: `BackpressureController`
- **Storage**: Anonymous memory-mapped file in system temp directory
- **Format**: Length-prefixed framed records (8-byte length prefix + raw record bytes)
- **AUDIT encryption**: AES-256-GCM with per-session key

### Emergency Buffer Data Flow

```
Normal Path:                   Emergency Path:
                               (ring buffer >95%)
dologger_log()                 dologger_log()
     │                              │
     ▼                              ▼
ring_buffer.try_push()         ring_buffer.try_push()
     │                              │
     │ OK                           │ ERR (full)
     │                              ▼
     │                         emergency_buffer.push()
     │                              │
     │                              ▼
     │                         mmap file on disk
     │                         (AES-256-GCM if AUDIT)
```

### Recovery

```
Engine Startup:
  1. Check for emergency buffer file: dologger_emergency_<pid>_<ts>.buf
  2. If found:
     a. Read all spilled records
     b. LSN-based deduplication (skip records with already-seen LSNs)
     c. Replay into the main pipeline
     d. Delete the emergency file
  3. Emit EMERGENCY_RECOVERED sysmon event
```

### Emergency Buffer Limits

| Parameter | Default |
|:-:|:-:|
| Max file size | 512 MB |
| Max records | 1,000,000 |

If these limits are exceeded, the emergency buffer itself drops records and a `EMERGENCY_BUFFER_OVERFLOW` sysmon event is emitted.

---

## Thread Pool Architecture

### Pool Layout

```
┌─────────────────────────────────────────────────────────────┐
│                     THREAD POOLS                            │
│                                                             │
│  ┌──────────────────┐  ┌─────────────────┐  ┌─────────────┐ │
│  │    cpu_pool      │  │    io_pool      │  │ sysmon_pool │ │
│  │                  │  │                 │  │             │ │
│  │ Threads: N       │  │ Threads: N/2    │  │ Threads: 1  │ │
│  │ Priority: Normal │  │ Priority: Normal│  │ Priority:Low│ │
│  │                  │  │                 │  │             │ │
│  │ Pipeline stages: │  │ Sink writes:    │  │ Sysmon flush│ │
│  │  Filter          │  │  File           │  │ Diagnostics │ │
│  │  FieldProvider   │  │  Kafka          │  │             │ │
│  │  Assembly        │  │  Syslog         │  │             │ │
│  │  Processing      │  │  Webhook        │  │             │ │
│  │  Formatting      │  │  OTel           │  │             │ │
│  │                  │  │                 │  │             │ │
│  └──────────────────┘  └─────────────────┘  └─────────────┘ │
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  AUDIT Consumer Thread (dedicated, never shared)        ││
│  │  Name: dologger-audit-pipeline                          ││
│  │  Priority: Normal                                       ││
│  │  Work: Read→Sign→Dual-write(WORM+Security)→Pool return  ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Config Watcher Thread (1 thread)                       ││
│  │  Name: dologger-config-watcher                          ││
│  │  Work: Poll config file every 1s (500ms debounce)       ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

### Thread Naming Convention

All threads follow the naming pattern `dologger-<pool>-<id>`:

```
dologger-cpu_pool-0
dologger-cpu_pool-1
dologger-io_pool-0
dologger-sysmon_pool-0
dologger-audit-pipeline
dologger-config-watcher
```

### Scheduler

The pipeline scheduler uses a work-stealing thread pool (`crossbeam_channel`):

- CPU pool: `num_cpus` threads for CPU-bound pipeline stages
- IO pool: `num_cpus / 2` threads for IO-bound sink writes
- Sysmon pool: 1 thread for diagnostic flushing (low priority)

---

## Plugin VTable Specification

### The 10 Plugin Types

| # | Type | Phase | VTable Functions |
|:-:|:-:|:-:|:-:|
| 1 | `Filter` | Filter (1) | `filter`, `filter_batch` |
| 2 | `PolicyProvider` | PreFilter (0) | `policy_evaluate`, `policy_update` |
| 3 | `FieldProvider` | Field (2) | `provide_fields`, `provide_fields_batch` |
| 4 | `HostInfoProvider` | Field (2) | `provide_host_info` (Ring 1 restricted) |
| 5 | `Processor` | Process (4) | `process`, `process_batch` |
| 6 | `Formatter` | Format (5) | `format`, `flush` |
| 7 | `IOSink` | Sink (6) | `open`, `write`, `flush`, `close`, `health` |
| 8 | `ConfigProvider` | Config (load-time) | `load_config`, `watch_config` |
| 9 | `KeyProvider` | Key (load-time) | `sign`, `public_key`, `rotate` |
| 10 | `SyscallBroker` | Syscall (proxy) | `broker_dispatch` |

### VTable Pattern

All VTable functions follow this contract:

```c
// Required: Provide function pointer, or NULL if unsupported
// Return: DO_LOG_OK on success, error code on failure
// DO_LOG_ERR_FATAL causes the plugin to be unloaded

typedef dologger_error_t (*vtable_fn_t)(/* parameters */);
```

### Plugin Lifecycle

```
engine_start()
  │
  └─ for each plugin in config:
       ├─ dlopen(plugin_path)                     // Load shared library
       ├─ dlsym("plugin_query")                   // → PluginInfo
       │   ├─ Validate ABI version
       │   ├─ Validate type
       │   └─ Validate license SPDX
       ├─ dlsym("dologger_vtable")                // → VTable struct
       │   └─ Validate required function pointers
       ├─ (Blue only) Verify Ed25519 signature
       ├─ Apply sandbox policy (seccomp/AppContainer)
       └─ plugin_init(config)                     // → Allocate state
           │
           │  ... runtime: VTable functions called ...
           │
           ▼
engine_shutdown()
  │
  └─ for each plugin in REVERSE load order:
       ├─ plugin_shutdown()                       // → Free state
       └─ dlclose()
```

### Required C ABI Exports

Every plugin MUST export:

```c
const dologger_plugin_info_t *plugin_query(void);
dologger_error_t plugin_init(const dologger_plugin_config_t *config);
dologger_error_t plugin_shutdown(void);
const <type>_vtable_t dologger_vtable;   // Type-specific VTable
```

Every plugin MAY export:

```c
dologger_error_t plugin_state_serialize(dologger_state_buf_t *out);
dologger_error_t plugin_state_deserialize(const dologger_state_buf_t *in);
```

---

## SIF Binary Format Overview

### Format

SIF (Structured Interchange Format) is the binary log record format used for WORM storage and inter-process communication.

### Record Layout (simplified)

```
┌──────────────────────────────────────────────────────────────┐
│ Offset │ Size  │ Field             │ Description             │
├────────┼───────┼───────────────────┼─────────────────────────┤
│   0    │   4   │ magic             │ b"SIF1" (0x53494631)    │
│   4    │   4   │ version           │ Format version (1)      │
│   8    │   4   │ length            │ Total record length     │
│  12    │   8   │ lsn               │ Log Sequence Number     │
│  20    │   8   │ timestamp_hi      │ Timestamp high 64 bits  │
│  28    │   8   │ timestamp_lo      │ Timestamp low 64 bits   │
│  36    │   1   │ level             │ Log level (0-6)         │
│  37    │   1   │ flags             │ Bit flags               │
│  38    │   2   │ reserved          │ Reserved (padding)      │
│  40    │   8   │ thread_id         │ Thread ID               │
│  48    │   8   │ process_id        │ Process ID              │
│  56    │   8   │ origin_lsn        │ Origin LSN (distributed)│
│  64    │  64   │ signature         │ Ed25519 signature       │
│ 128    │  32   │ prev_hash         │ SHA-256 chain hash      │
│ 160    │  ...  │ message           │ Length-prefixed UTF-8   │
│  ...   │  ...  │ source_file       │ Length-prefixed UTF-8   │
│  ...   │  ...  │ host_name         │ Length-prefixed UTF-8   │
│  ...   │   4   │ crc32c            │ CRC32C of entire record │
└──────────────────────────────────────────────────────────────┘
```

### Future Direction

The current SIF format is a simplified binary frame. M4 will introduce a full FlatBuffers-based SIF with schema evolution support for forward/backward compatibility.

---

## Performance Benchmarks and Tuning

### Hardware Reference

| Component | Specification |
|:-:|:-:|
| CPU | AMD Ryzen 9 7950X (16C/32T) |
| RAM | DDR5-6000 |
| Storage | Samsung 990 Pro NVMe |
| OS | Linux 6.x |
| Rust | 1.97.1, release + LTO |

### Benchmark Results

| Benchmark | Measurement |
|:-:|:-:|
| Single record submit (CAS push) | 102 ns P50 |
| Ring buffer push (1K records) | 121 us |
| Batch push (256 records) | 19.2 us |
| Console Sink, no signing | 1,200,000 rec/s |
| File Sink, no signing | 950,000 rec/s |
| File Sink, Ed25519 signing | 58,000 rec/s |
| WORM Sink, sign + fsync | 12,000 rec/s |
| CRC32C (64 bytes) | ~3 ns (SSE 4.2 hardware) |

### Tuning Parameters

| Parameter | Default | Tuning Guidance |
|:-:|:-:|:-:|
| `ring_buffer_size` | 262144 | Increase for bursty workloads. Must be power of two. |
| `batch_size` | 256 | 128-512. Larger = higher throughput, higher latency. |
| `enable_signature` | false | Adds ~17 us per record. Only for AUDIT/compliance. |
| `fsync_on_write` | false | Forces media durability. I/O latency bound. |
| `ring_buffer_coop_helping` | true | Prevents overflow at cost of ~1 us on hot path. |

### OS-Level Tuning

```bash
# Pin pipeline threads to isolated CPUs
sudo cset shield --cpu 2-3 --kthread=on

# Increase max locked memory for ring buffer (huge pages)
sudo sysctl -w vm.max_map_count=262144

# Disable transparent huge pages when measuring latency
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled
```

### Running Benchmarks

```bash
# Run the built-in benchmark suite
cargo bench --bench hot_path

# Latency benchmarks
cargo bench --bench latency

# Throughput benchmarks
cargo bench --bench throughput

# Latency percentile distribution
cargo bench --bench latency_percentiles
```
