# Naming Convention

> Applies to every source file in this repository: Rust, C, C++, Go, Python,
> and the build/CI scripts. The goal is a single, checkable grammar so that a
> reader can predict a file's name from its responsibility — and its
> responsibility from its name.

## 1. The directory tree IS the namespace

The hierarchy and the module graph carry the layer/module information. A leaf
file is named after **the primary concept it exports** — never repeated with
its enclosing module's name:

```
core/src/buffer/object_pool.rs     # object_pool, inside the buffer module
core/src/sink/shared_memory.rs     # shared_memory, inside the sink module
```

Do NOT write `buffer_object_pool.rs` inside `buffer/` — the path already says
`buffer`. Repeating it triples the redundancy and lengthens every import.

## 2. Leaf grammar

A file name is either a bare object or an object with a role suffix:

```
{object}            →  snake_case noun naming the primary exported item
{object}_{role}     →  object + an approved role suffix (see §3)
```

Examples:

| Pattern | Examples |
|:-:|:-:|
| `{object}` | `record`, `audit`, `domain`, `phase`, `time` |
| `{object}_{role}` | `key_provider`, `secret_detector`, `ring_buffer`, `control_plane` |

Rules:

- snake_case, ASCII lowercase; no spaces, no CamelCase file names (types may
  be PascalCase, file names never).
- One primary concept per file. A file that defines `Foo` and a tiny helper
  struct is still `foo.rs`; a file that bundles two unrelated concepts should
  be split.
- Same-directory siblings follow one grammar: `sink/` uses bare nouns
  (`console`, `file`, `syslog`) for one-sink-per-file and `{object}` for
  infrastructure (`ring_buffer`).
- No abbreviations in file names unless listed in §4.

## 3. Approved role suffixes

A role suffix is the code-level analogue of PowerShell's *verb-noun* rule: the
"verb" is the file's role, drawn from a fixed list. New roles are added to the
list (with a review), never invented ad hoc.

| Role | Meaning |
|:-:|:-:|
| `manager` | Owns the lifecycle of one or more objects |
| `provider` | Supplies an object on demand (factory-like) |
| `dispatcher` | Routes work/records to handlers |
| `validator` | Enforces invariants before something is accepted |
| `loader` | Materialises objects from storage/config |
| `writer` | Persists output |
| `reader` | Reads input |
| `watcher` | Observes state and reacts |
| `scheduler` | Decides execution order/timing |
| `detector` | Recognises a condition (e.g. secret, anomaly) |
| `rotator` | Periodically replaces credentials/keys |
| `builder` | Constructs a complex object step by step |
| `parser` | Converts text/bytes into a structured form |
| `store` | Encapsulates a durable collection |
| `registry` | Maps keys to registered items |
| `service` | Long-running, externally-visible capability |
| `policy` | Encapsulates a decision rule |
| `facade` | Simplifies a subsystem behind one entry point |
| `adapter` | Adapts one interface to another |
| `layer` | Integration with a logging frontend |
| `encoder` / `decoder` | Serialization / deserialization |
| `reporter` | Emits metrics/telemetry |
| `handler` | Reacts to events or callbacks |
| `engine` | Drives a stateful computation loop |

Not every file needs a role. When no role fits, use the bare `{object}` form.

## 4. Allowed abbreviations

Abbreviations are forbidden unless they are (a) an industry-wide term, or (b)
part of a frozen public ABI. Each entry below documents why it is exempt.

| Name | Why it stays |
|:-:|:-:|
| `ffi` | Universal term; `dologger_core::ffi` is the plugin-facing API surface and is frozen |
| `io` | Universal term for input/output |
| `shm` | Matches POSIX `shm_open` and the frozen C ABI (`dologger_shm.h`, `dologger_shm_*`) |
| `crc32c` | The algorithm's canonical name (Castagnoli CRC-32C) |
| `sif` | Project term: **S**tandard **I**ntermediate **F**ormat |
| `perf` | CLI subcommand verb; matches the `perf` tool name |

Anything else — `diag` → `diagnostics`, `sysmon` → `system_monitor`, `otel` →
`open_telemetry` — must be spelled out.

## 5. Prohibited patterns

- **Module-name repetition** in a leaf (`sink/sink_file.rs`).
- **Ambiguous agent nouns**: bare `manager.rs`, `handler.rs`, `service.rs`
  with no object are not allowed at crate root — the object must be named.
- **Mixed vocabularies for one concept**: if a concept is named `record`
  (not `log`) and `sink` (not `output`), every file for that concept uses the
  same term. When two terms genuinely mean different things (`internal_log`
  vs `syslog`), keep them distinct and document the difference.
- **Feature/file name drift**: a file gated behind feature `foo` must be
  named `foo.rs` or its object must be spelled out (`open_telemetry.rs` gated
  by `sink-otel`).

## 6. Rename record

| Before | After | Reason |
|:-:|:-:|:-:|
| `core/src/sys/diag.rs` | `core/src/sys/diagnostics.rs` | abbreviation → full word |
| `core/src/sink/otel.rs` | `core/src/sink/open_telemetry.rs` | abbreviation → full word; also fixed the feature gate (`sink-webhook` → `sink-otel`) |
| `core/src/sys/sysmon.rs` | `core/src/sys/system_monitor.rs` | fused abbreviation → full word |

Backward-compatible re-exports are kept in `core/src/lib.rs` (`diag`, `sysmon`,
`sink_otel`) so existing `dologger_core` paths keep resolving.

## 7. Checking

`cargo fmt` enforces snake_case mechanically. Role-suffix and abbreviation
rules are reviewed in code review; a future `cargo xtask lint-naming` may
mechanically check the abbreviation list.
