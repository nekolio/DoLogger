# Error Codes Reference

> **Authoritative source of truth for DoLogger error codes.** The constants live
> in [`core/src/error.rs`](../../../core/src/error.rs) and are mirrored in the
> C ABI enum `dologger_error_code_t` in
> [`core/include/dologger_core.h`](../../../core/include/dologger_core.h).
> Do not trust error values found anywhere else (including older design docs) —
> this table, `error.rs`, and the C header are the only live sources.

## Design

Every error code is a negative `i32`; `0` (`DO_LOG_OK`) means success. The
magnitude is organized as a nibble scheme where the **high byte names the phase
in which the failure surfaced**, following the journey of a record:

```
caller → config → plugin → record → ingest → sign → sandbox → sink → remote → quota → compliance → clock → SIF
0x01     0x02      0x03      0x04     0x05    0x06     0x07        0x08     0x09     0x0A      0x0B        0x0C    0x0D
```

The ordering principle (why, not just what):

1. **Semantic / scenario order** — codes are grouped by the stage of the
   execution flow, so an operator reading a value can tell *where* to look.
2. **Community convention** — negative error, `0` for success, category band +
   sequential code within a band (POSIX `errno`, Linux kernel `-EXYZ`,
   Win32 `HRESULT` facility/code), stable once assigned, new codes appended at
   the end of a band.
3. **Names** — `DO_LOG_ERR_<SUBSYSTEM>_<CONDITION>` in `UPPER_SNAKE_CASE`;
   the condition names the failure, not the recovery.

**Plugin-defined codes** use the high-bit range `-0x80000000` and below
(`0x80000000`–`0xFFFFFFFF` unsigned). The core passes them through without
interpretation, only wrapping them in a `DologgerDomainEvent` for sysmon.

## 0x01xx — General / API

Caller-boundary checks: argument validation and engine lifecycle. These are the
first errors any host sees.

| Value | Name | Description |
|:-:|:-:|:-:|
| `0` | `DO_LOG_OK` | Success (no error) |
| `-0x0101` | `DO_LOG_ERR_INVALID_ARG` | Invalid argument passed to an API |
| `-0x0102` | `DO_LOG_ERR_NOT_SUPPORTED` | Operation not supported on this platform / build |
| `-0x0103` | `DO_LOG_ERR_NOT_INITIALIZED` | Core engine not initialized |
| `-0x0104` | `DO_LOG_ERR_ALREADY_INITIALIZED` | Core engine already initialized (double init) |
| `-0x0105` | `DO_LOG_ERR_OUT_OF_MEMORY` | Memory allocation failure |
| `-0x0106` | `DO_LOG_ERR_BUFFER_TOO_SMALL` | Caller-provided buffer too small for the result |
| `-0x0107` | `DO_LOG_ERR_TIMEOUT` | Operation timed out |
| `-0x0108` | `DO_LOG_ERR_INTERNAL` | Generic internal error |
| `-0x0109` | `DO_LOG_ERR_INIT_FAILED` | Engine initialization failed with an internal fatal error |

## 0x02xx — Configuration

Config file load, parse, validate, merge (domain inheritance), and hot reload.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0201` | `DO_LOG_ERR_CONFIG_NOT_FOUND` | Config file not found |
| `-0x0202` | `DO_LOG_ERR_CONFIG_PERMISSION` | Config file permission denied |
| `-0x0203` | `DO_LOG_ERR_CONFIG_PARSE` | Config parse (TOML syntax) error |
| `-0x0204` | `DO_LOG_ERR_CONFIG_VALIDATION` | Config semantic validation failed |
| `-0x0205` | `DO_LOG_ERR_CONFIG_MERGE` | Config merge conflict (domain inheritance) |
| `-0x0206` | `DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED` | Hot reload failed; previous config stays in effect |
| `-0x0207` | `DO_LOG_ERR_CONFIG_HASH_MISMATCH` | Hot reload config hash mismatch (file changed mid-check) |
| `-0x0208` | `DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID` | New config submitted for hot reload failed validation |
| `-0x0209` | `DO_LOG_ERR_CONFIG_RESTART_REQUIRED` | Reload applied other fields but protected encoding changes require restart |

## 0x03xx — Plugin

Plugin registry and runtime: load, manifest, ABI, dependencies, state, and
cross-plugin calls.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0301` | `DO_LOG_ERR_PLUGIN_NOT_FOUND` | Plugin not found in any search path |
| `-0x0302` | `DO_LOG_ERR_PLUGIN_LOAD_FAILED` | Dynamic-library load failed (link, missing symbol, platform mismatch) |
| `-0x0303` | `DO_LOG_ERR_PLUGIN_MANIFEST_INVALID` | Plugin manifest validation failed |
| `-0x0304` | `DO_LOG_ERR_PLUGIN_VERSION_MISMATCH` | Plugin version incompatible with the core ABI |
| `-0x0305` | `DO_LOG_ERR_PLUGIN_ABI` | Plugin ABI incompatible with the core |
| `-0x0306` | `DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING` | Plugin dependency not satisfied |
| `-0x0307` | `DO_LOG_ERR_PLUGIN_LOCK_MISMATCH` | Plugin lock file mismatch (deterministic loading) |
| `-0x0308` | `DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID` | Plugin signature verification failed |
| `-0x0309` | `DO_LOG_ERR_MISSING_CAPABILITY` | Plugin depends on a capability no provider offers |
| `-0x030A` | `DO_LOG_ERR_CIRCULAR_DEPENDENCY` | Circular dependency detected in the plugin graph |
| `-0x030B` | `DO_LOG_ERR_TOKEN_EXCEEDED_DEPTH` | Cross-plugin call capability token chain depth exceeded |
| `-0x030C` | `DO_LOG_ERR_CALL_DEADLOCK` | Cross-plugin call detected a deadlock (cyclic wait) |
| `-0x030D` | `DO_LOG_ERR_STATE_FORMAT_UNSUPPORTED` | Plugin state format version not supported |
| `-0x030E` | `DO_LOG_ERR_STATE_ROLLBACK_REJECTED` | Plugin state migration rejected a rollback (epoch anti-rollback) |
| `-0x030F` | `DO_LOG_ERR_STATE_MIGRATE_FAILED` | Plugin state serialize/deserialize migration failed during reload |

## 0x04xx — Record / Field

Record invariants and field access.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0401` | `DO_LOG_ERR_RECORD_INVALID` | Record is in an invalid state |
| `-0x0402` | `DO_LOG_ERR_FIELD_NOT_FOUND` | Field not found in record |
| `-0x0403` | `DO_LOG_ERR_FIELD_PERMISSION_DENIED` | Field access denied (Ring permission violation) |
| `-0x0404` | `DO_LOG_ERR_FIELD_TYPE_MISMATCH` | Field type mismatch |
| `-0x0405` | `DO_LOG_ERR_FIELD_DEPENDENCY_NOT_MET` | Plugin-required field not provided by an earlier pipeline stage |
| `-0x0406` | `DO_LOG_ERR_RECORD_INVALID_ENCODING` | Legacy text ABI input is not valid UTF-8 |

## 0x05xx — Buffer / Pipeline

Ingest, backpressure, and pipeline stages.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0501` | `DO_LOG_ERR_BUFFER_FULL` | Ring buffer full and the configured strategy forbids drop/block-free |
| `-0x0502` | `DO_LOG_ERR_PIPELINE_STAGE` | Pipeline stage error |
| `-0x0503` | `DO_LOG_ERR_AUDIT_QUEUE_FULL` | Audit-domain queue full with a no-drop policy |

## 0x06xx — Signature / Audit chain

Key service, signing, verification, LSN chain, and audit-domain policy.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0601` | `DO_LOG_ERR_SIGN_FAILED` | Signature generation failed (Assembly stage) |
| `-0x0602` | `DO_LOG_ERR_VERIFY_FAILED` | Signature verification failed (possible tampering) |
| `-0x0603` | `DO_LOG_ERR_LSN_CHAIN_BROKEN` | LSN chain broken (tampering detected) |
| `-0x0604` | `DO_LOG_ERR_LSN_GAP_DETECTED` | LSN gap detected (reorder window exceeded) |
| `-0x0605` | `DO_LOG_ERR_KEY_NOT_AVAILABLE` | Required key not available for signing |
| `-0x0606` | `DO_LOG_ERR_KEY_PROVIDER_FAILED` | KeyProvider plugin open/read/sign operation failed |
| `-0x0607` | `DO_LOG_ERR_AUDIT_DROP_FORBIDDEN` | AUDIT domain configured with a drop strategy |
| `-0x0608` | `DO_LOG_ERR_AUDIT_CALLBACK_ONLY` | AUDIT domain configured with only a callback sink |
| `-0x0609` | `DO_LOG_ERR_AUDIT_NO_PERSISTENT_SINK` | AUDIT domain has no persistent primary sink |

## 0x07xx — Security / Sandbox

Plugin execution protection.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0701` | `DO_LOG_ERR_SANDBOX_INIT_FAILED` | Sandbox initialization failed |
| `-0x0702` | `DO_LOG_ERR_SANDBOX_VIOLATION` | Sandbox policy violation (forbidden syscall blocked) |
| `-0x0703` | `DO_LOG_ERR_UNTRUSTED_PLUGIN` | Attempted to load an unsigned (Red) plugin in production mode |

## 0x08xx — Sink / IO

Local and shared-memory output: file, WORM, callback, and `sink_shm`.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0801` | `DO_LOG_ERR_SINK_WRITE_FAILED` | Sink write failed (full or partial write) |
| `-0x0802` | `DO_LOG_ERR_SINK_CONNECTION_FAILED` | Sink failed to connect its target (file, network, broker) |
| `-0x0803` | `DO_LOG_ERR_SINK_CONNECTION_LOST` | Sink connection lost after establishment |
| `-0x0804` | `DO_LOG_ERR_SINK_FORMAT_INVALID` | Sink output format configuration invalid or unsupported |
| `-0x0805` | `DO_LOG_ERR_SINK_CONFIG_INVALID` | Sink configuration rejected (e.g. `sink_shm` `full_policy = "block"`) |
| `-0x0806` | `DO_LOG_ERR_SINK_NO_FALLBACK` | Sink does not support a fallback chain (e.g. `sink_shm`) |
| `-0x0807` | `DO_LOG_ERR_CALLBACK_TIMEOUT` | Callback sink host invocation timed out |
| `-0x0808` | `DO_LOG_ERR_WORM_WRITE_FAILED` | WORM write failed (disk full, permission) |
| `-0x0809` | `DO_LOG_ERR_SHM_INIT_FAILED` | Shared-memory object create/map failed (permission, space) |
| `-0x080A` | `DO_LOG_ERR_SHM_RING_FULL` | Shared-memory ring buffer full (surfaced only with a block policy) |
| `-0x080B` | `DO_LOG_ERR_AUDIT_SHM_FORBIDDEN` | `sink_shm` configured for an AUDIT domain — forbidden |

## 0x09xx — Network / Remote

Remote sinks (Kafka / Syslog / Webhook): connection, TLS, SASL, circuit breaker.

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0901` | `DO_LOG_ERR_CIRCUIT_OPEN` | Remote-sink circuit breaker is OPEN; writes rejected |
| `-0x0902` | `DO_LOG_ERR_TLS_FAILED` | TLS handshake / certificate failure |
| `-0x0903` | `DO_LOG_ERR_SASL_FAILED` | SASL authentication failure |
| `-0x0904` | `DO_LOG_ERR_REMOTE_TIMEOUT` | Remote sink operation timed out (produce, batch ack) |

## 0x0Axx — Resource / Quota

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0A01` | `DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED` | Plugin memory usage exceeded its configured quota |
| `-0x0A02` | `DO_LOG_ERR_QUOTA_CPU_EXCEEDED` | Plugin CPU usage exceeded its configured quota |
| `-0x0A03` | `DO_LOG_ERR_RECURSION_DEPTH_EXCEEDED` | Logging self-reference recursion depth exceeded |

## 0x0Bxx — Compliance

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0B01` | `DO_LOG_ERR_COMPLIANCE_VIOLATION` | Compliance violation (template vs manual config, or a non-downgradable item relaxed) |
| `-0x0B02` | `DO_LOG_ERR_AUDIT_DURABILITY_INSUFFICIENT` | AUDIT domain sink durability below the required MEDIA level |

## 0x0Cxx — Clock / Time safety

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0C01` | `DO_LOG_ERR_TIME_BACKWARD` | Monotonic clock jumped backward; AUDIT domain frozen |

## 0x0Dxx — SIF / Serialization

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0D01` | `DO_LOG_ERR_SIF_INVALID` | SIF frame malformed, over limit, or failed structural verification |

## 0x0Exx — Internal / Fatal

| Value | Name | Description |
|:-:|:-:|:-:|
| `-0x0E01` | `DO_LOG_ERR_FATAL` | Engine-fatal condition (plugin unloaded; sink triggers `SINK_CIRCUIT_OPEN`) |

## 0x0Fxx — Reserved

Reserved for future core expansion. Plugin-defined codes must use the high-bit
range `-0x80000000` and below; core codes never enter that space.

## Related

- Rust constants + category rationale: `core/src/error.rs`
- C ABI enum: `core/include/dologger_core.h`
- Where codes surface: `DologgerError` / `DologgerDomainEvent` (see [Host Integration Guide](HostIntegrationGuide.md))
- Testing expectations: [Testing Convention](TestingConvention.md)
