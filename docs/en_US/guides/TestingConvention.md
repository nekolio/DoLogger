# Testing Convention

> Live source of truth for **what** DoLogger tests, **where** they live, and
> **how** to write them. Applies to every code change: no feature, fix, or
> migration lands without the tests this document requires.
>
> The four-quadrant model (unit / integration / benchmark / security-stress)
> follows the design intent captured in older drafts, grounded here in the
> repository's physical layout.

## 1. Test taxonomy and physical layout

| Quadrant | Physical location | What goes there |
|:-:|:-:|:-:|
| **Unit** | `#[cfg(test)] mod tests` beside the code (`core/src/**`) | Single function / algorithm / data-structure behavior; error paths; invariants |
| **Integration** | [core/tests/](../../../core/tests/) (`{subject}.rs`, auto-discovered by Cargo) | Cross-module behavior: config → registry → sinks, plugin bundle, sandbox, security, fanout |
| **Integration (process-level)** | [tests/](../../../tests/) (`common/`, `smoke/`, `release-smoke/`) | C ABI smoke, platform smoke runners, release gates |
| **Benchmark** | [core/benches/](../../../core/benches/) (Criterion) | Latency, throughput, percentiles; per-sink delivery latency |
| **Fuzz** | [core/fuzz/](../../../core/fuzz/) (cargo-fuzz) | Untrusted input parsers: SIF frames, TOML config, ring buffer ops |
| **Perf (C ABI)** | [tests/perf/](../../../tests/perf/) (CMake) | Host-language / C ABI throughput harnesses |

## 2. What requires tests — no exceptions

1. **Every new public API** (`pub fn` / C ABI function / config key): happy path +
   every `Result::Err`/error-code path. If a code path can fail, a test must
   drive it to failure.
2. **Every new error code** (see [ErrorCodesReference](ErrorCodesReference.md)):
   the code is exercised by the failure path that emits it.
3. **Every config surface** (new TOML field, new `[shm]`/`[dologger]` key):
   valid config parses + carries through; missing fields fall back to defaults;
   invalid values are rejected with the right error code/warning; cross-platform
   path escaping (e.g. `\` in Windows paths) is covered.
4. **Every sink** (incl. `sink_shm` and remote sinks): lifecycle
   (open→write→flush→close), failure modes (ring full / connect lost /
   timeout), and cleanup (no leaked descriptors / shared-memory objects).
5. **Cross-platform guards**: any code guarded by `#[cfg(...)]` gets a test on
   each affect-ed platform (Linux in CI, Windows + macOS in the matrix where
   present). Platform-specific behavior must never be covered by one platform
   alone.
6. **Determinism**: anything labeled deterministic (hero art, release tooling,
   config loading) must have a two-run-comparison test.

## 3. How to write tests — repo style

- **Placement**: unit tests inline (`#[cfg(test)] mod tests`); integration
  tests as standalone files in `core/tests/`. Follow the existing style in
  [fanout_sinks.rs](../../../core/tests/fanout_sinks.rs): a doc comment stating
  the tested property, small focused tests, no shared global state.
- **Unique temp artifacts**: use a per-process atomic counter + `process::id()`
  when creating temp files/shm names (see `temp_path()` in fanout_sinks) so
  parallel tests never collide. Always clean up in the test body.
- **TOML-in-string**: build configs inline with `DologgerConfig::parse` rather
  than brittle file fixtures; escape `\` for Windows paths.
- **Error-code assertions**: assert on the symbolic constant, never a
  hard-coded literal.
- **Deterministic over flaky**: prefer bounded in-process tests over timing
  asserts; use `Ordering`-aware expectations for concurrent structures; if a
  test can flake by design (e.g. scale-dependent), gate it behind an env var
  rather than deleting it.
- **Rust best practices**: `#[should_panic]` only for documented invariants;
  use `proptest`/`arbitrary` for parser inputs (marked  will-not-flake);
  `loom` is a *future* tool for the lock-free structures — do not add it until
  a specific interleaving bug is being chased.

## 4. Acceptance gates (run before every commit / in CI)

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dologger-core
cargo test -p dologctl
cargo test (workspace, default members)
```

Benchmarks run locally against the release build (`cargo bench`); results land
in `benchmark-results.json` and release notes. Hard per-PR benchmark gates
(throughput −5%, P99 +10%, RSS +5% vs baseline) are documented targets; they
activate once a stable CI baseline runner exists — until then, record and
publish, don't gate.

## 5. Sink-specific test checklist (applies to `sink_shm` and remote sinks)

- **Unit**: config validation (each invalid value → correct code), header
  layout size asserts, ring-full detection, drop vs overwrite counters.
- **Integration (cross-process for shm)**: producer writes N SIF records; a
  separate consumer process maps the region read-only and verifies every
  decoded record matches; ring-full behavior with `drop_oldest`/`drop_newest`;
  producer crash → consumer detects `FLAG_PRODUCER_DEAD` and exits without
  dangling pointers; AUDIT-domain config is rejected.
- **Fuzz**: malformed SIF bytes written into the ring must not crash the
  consumer parser (`shm_parser` target).
- **Stress (deferred to CI capacity)**: multi-consumer concurrent attach under
  watermark semantics (see `sink_shm` design doc) with no data races; long-run
  leak check that the shared-memory object size stays constant.

## 6. Reviewing your own change

Before marking a change done, answer as a checklist:

- [ ] Do the new/updated error paths have tests that hit them?
- [ ] Do new config keys have parse / default / reject tests?
- [ ] Do new sinks cover lifecycle AND failure AND cleanup?
- [ ] Are platform-specific paths tested per platform?
- [ ] Does `cargo test` + `clippy -D warnings` pass?
- [ ] Does the change's docs (EN + zh, 1:1) stay consistent with behavior?

## Related

- [Error Codes Reference](ErrorCodesReference.md) — the codes tests must assert on
- [tests/README.md](../../../tests/README.md) — canonical category → physical location map
- `core/tests/` integration suites — style exemplars to imitate