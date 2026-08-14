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
{object}_{role}     →  object + an approved role suffix (see §5)
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
- No abbreviations in file names unless listed in §6.

### Shell scripts (`scripts/`, `tests/smoke/`, `peripheral/github/scripts/`)

Executable scripts use PowerShell-style *verb-noun* names:

```
{verb}-{object}.sh        # build-all.sh, setup-conan.sh, check-environment.sh
```

- `{verb}` comes from the approved list below; `{object}` is the target in
  lowercase, hyphen-separated when multi-word (`release-notes`).
- The `.sh` suffix is **mandatory** — every Bash script carries it. No
  extension-less, `dologger-`-prefixed names: the directory already names the
  project, and the file is invoked as `bash scripts/<name>.sh`.
- Full words only — `generate-release-notes.sh`, never `gen-release-notes.sh`.

Approved verbs (add new ones here with a review, as in §5):

| Verb | Meaning |
|:-:|:-:|
| `build` | Compiles artifacts |
| `setup` | Installs or detects prerequisites |
| `check` | Verifies an environment or output |
| `sync` | Mirrors content to a target |
| `generate` | Produces a document/body from git state |

### Library scripts (`scripts/lib/`, `tests/common/`)

Scripts that are `source`d by the executables above are helpers, not entry
points, and use bare descriptive names — no verb:

```
{object}.sh            # common.sh, lib.sh
```

- No verb-noun: the verb-noun rule exists so an executable's purpose is
  visible in its name; a sourced helper is always invoked as
  `source scripts/lib/common.sh` and has no single entry-point purpose to name.
- The `.sh` suffix still applies.

### Generated files

Machine-produced files are marked with a `{schema}_generated` suffix:

```
{name}_generated.rs    # dologger_sif_generated.rs (FlatBuffers codegen)
```

- The `_generated` suffix is a deliberate marker: the file is machine-written,
  not hand-maintained, and must not be edited by hand.
- Regeneration is wired into the build — `core/build.rs` regenerates
  `dologger_sif_generated.rs` when the generator (`flatc`) is available.
- The generated output is still committed to the repository so the crate
  builds without the toolchain; the committed copy is the fallback.

### Entrypoint exemptions

`main.rs`, `main.c`, `lib.rs` are fixed by the language toolchain, not by this
convention — a crate's entry point must be named exactly that no matter what
it exports. They are exempt from the leaf grammar; their identity comes from
the directory they sit in.

## 3. Test files

Tests follow the native convention of each test framework — the framework
already signals "this is a test", so a redundant `_test`/`_tests` suffix on
the file name is prohibited:

| Language / framework | Convention | Examples |
|:-:|:-:|:-:|
| Rust — integration (`tests/`, `core/tests/`) | bare `{subject}.rs` | `core/tests/security.rs`, `core/tests/plugin_sandbox.rs` |
| Rust — unit | inline `#[cfg(test)] mod tests` in `{subject}.rs` | no separate file |
| Rust — cargo-fuzz | `fuzz_{target}.rs` | `core/fuzz/fuzz_targets/fuzz_ring_buffer.rs` |
| Go | `{name}_test.go` beside `{name}.go` | `adapters/go/dologger_test.go` |
| Python | `test_{name}.py` (pytest) | `adapters/python/test_dologger.py` |
| Shell / PowerShell | verb-noun like any script | `tests/smoke/check-smoke.sh`, `tests/smoke/check-smoke.ps1` |

Rust integration suites in `core/tests/` are named `{subject}.rs` and
auto-discovered by Cargo (the crate declares no `[[test]]` entries):
`cargo test -p dologger-core --test <subject>`.

A helper module that a test *drives* (not itself discovered by the framework)
follows the leaf grammar like any other source — `tests/common/lib.sh`,
`tests/smoke/c_abi_smoke.py`.

## 4. Per-language rules

The leaf grammar applies everywhere; these rules refine it per language.

### C / C++

- Public C ABI headers carry the `dologger_` prefix. C has no module
  namespace, so the prefix *is* the namespace: `core/include/dologger_core.h`,
  `core/include/dologger_shm.h`. Internal `.c`/`.h` files inside a module use
  the bare leaf grammar.
- A `.c` and `.h` pair that belongs together shares a base name
  (`filter.c` + `filter.h`).
- A single-file example plugin is named after its dominant concept, never its
  enclosing directory:
  `plugins/examples/filter/c/example_filter/filter.c`,
  `plugins/examples/formatter/cpp/example_formatter/formatter.cpp`.
- The exported `dologger_*` symbols are frozen ABI; the prefix never renames.

### Go

- One package unit = `{package}.go` plus its test `{package}_test.go`.
- Example: `adapters/go/dologger.go` + `adapters/go/dologger_test.go`.

### Python

- Modules use snake_case — `adapters/python/dologger.py`.
- Tests use the pytest-native `test_{name}.py` prefix —
  `adapters/python/test_dologger.py`.

## 5. Approved role suffixes

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

## 6. Allowed abbreviations

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

## 7. Prohibited patterns

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

## 8. Rename record

| Before | After | Reason |
|:-:|:-:|:-:|
| `core/src/sys/diag.rs` | `core/src/sys/diagnostics.rs` | abbreviation → full word |
| `core/src/sink/otel.rs` | `core/src/sink/open_telemetry.rs` | abbreviation → full word; also fixed the feature gate (`sink-webhook` → `sink-otel`) |
| `core/src/sys/sysmon.rs` | `core/src/sys/system_monitor.rs` | fused abbreviation → full word |
| `core/tests/security_tests.rs` | `core/tests/security.rs` | `_tests` suffix redundant — the tests directory already signals "test" (§3) |
| `adapters/rust/src/slog_drain.rs` | `adapters/rust/src/slog_adapter.rs` | `drain` is not an approved role; the module adapts slog's `Drain` to DoLogger — `adapter` is; public API `dologger_sdk::slog_drain` → `slog_adapter` |
| `adapters/rust/src/write_sink.rs` | `adapters/rust/src/sink_writer.rs` | verb-first `write_sink` → `{object}_{role}` = `sink_writer`; `writer` is the approved role; public API `dologger_sdk::write_sink` → `sink_writer` |
| `plugins/examples/filter/c/example_filter/example_filter.c` | `.../example_filter/filter.c` | §1 rule — don't repeat the enclosing dir; named by dominant concept |
| `plugins/examples/formatter/cpp/example_formatter/example_formatter.cpp` | `.../example_formatter/formatter.cpp` | same as above |
| `peripheral/tools/hero-svg/hero_gen.py` | `peripheral/tools/hero-svg/hero_generator.py` | abbreviation `gen` → full word |
| `tests/release-smoke/cabi_smoke.py` | `tests/smoke/c_abi_smoke.py` | fused abbreviation `cabi` → `c_abi`; relocated under `tests/smoke/` |
| `tests/release-smoke/smoke-test.sh` / `smoke-test.ps1` | `tests/smoke/check-smoke.sh` / `check-smoke.ps1` | noun-verb → approved verb `check`; relocated under `tests/smoke/` |
| `scripts/check-env.sh` | `scripts/check-environment.sh` | abbreviation `env` → full word |
| `scripts/setup-dev.sh` | `scripts/setup-development.sh` | abbreviation `dev` → full word |
| `plugins/official/fmt_json/` | `plugins/official/formatter_json/` | abbreviation `fmt` → full word `formatter`; aligns with `plugin_type = "formatter"` |
| `plugins/official/fmt_text/` | `plugins/official/formatter_text/` | same as above |

Backward-compatible re-exports are kept in `core/src/lib.rs` (`diag`, `sysmon`,
`sink_otel`) so existing `dologger_core` paths keep resolving. The frozen C
ABI (`dologger_*` symbols and `dologger_core.h` / `dologger_shm.h`) never
renames.

## 9. Checking

`cargo fmt` enforces snake_case mechanically. Role-suffix and abbreviation
rules are reviewed in code review; a future `cargo xtask lint-naming` may
mechanically check the abbreviation list.
