# DoLogger Fuzz Testing

Coverage-guided fuzz testing for the DoLogger core engine using
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer backend).

## Quick Start

### 1. Install cargo-fuzz

```bash
cargo install cargo-fuzz
```

A nightly Rust toolchain is required (libFuzzer needs `-Z` compiler flags). The
workspace root `.rust-toolchain.toml` should already select nightly — verify
with:

```bash
rustup show
```

### 2. Build the fuzz targets (syntax check only, no fuzzing yet)

```bash
cd core/fuzz
cargo +nightly fuzz build
```

If there is no nightly in `rustup`, install it first:

```bash
rustup toolchain install nightly
```

### 3. Run a fuzz target

Each target accepts libFuzzer flags after `--`:

```bash
# Ring buffer (5 minutes)
cargo +nightly fuzz run fuzz_ring_buffer -- -max_total_time=300

# TOML config parser (5 minutes)
cargo +nightly fuzz run fuzz_toml_config -- -max_total_time=300

# Record / SIF format (5 minutes)
cargo +nightly fuzz run fuzz_sif_record -- -max_total_time=300
```

Longer runs for CI or nightly regression testing:

```bash
# 30 minutes each
cargo +nightly fuzz run fuzz_ring_buffer -- -max_total_time=1800 -jobs=4
cargo +nightly fuzz run fuzz_toml_config -- -max_total_time=1800 -jobs=4
cargo +nightly fuzz run fuzz_sif_record -- -max_total_time=1800 -jobs=4
```

### 4. Run the edge-case unit tests

Each fuzz target file also contains `#[cfg(test)] mod edge_case_tests` with
deterministic edge-case tests. Run them with:

```bash
cd core/fuzz
cargo test
```

### 5. Reproduce a crash

When a fuzz target finds a crash, it saves the reproducer to
`fuzz/artifacts/<target>/`. Re-run it with:

```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<crash-file>
```

## Fuzz Targets

### fuzz_ring_buffer

Exercises the lock-free MPSC ring buffer:

- Creation with random power-of-two capacity (1 .. 2^24)
- Sequential push/drain with random values
- Correctness invariants: no lost records, no duplicates, count consistency
- Edge cases: capacity 1, full buffer, empty buffer, partial batches

### fuzz_toml_config

Exercises the TOML configuration parser:

- Random byte sequences fed to `DologgerConfig::parse()`
- Parser must never panic on any input
- Valid TOML must produce a valid config
- Invalid TOML must return an error code (not panic)
- All config invariants verified after successful parse
- Edge cases: empty string, deeply nested tables, very long values,
  unknown profile strings, garbage binary

### fuzz_sif_record

Exercises the Record/SIF binary format and field access API:

- Record creation and field_get/set with random field names
- Ring permission checks (Ring 0-3) for all caller levels
- RecordString set/as_str round-trip with truncation handling
- CRC32C computation determinism, incremental update, empty input
- LogLevel parsing from arbitrary u8 values
- Ring 2 audit tag appending
- Ring 3 CRC32C auto-computation

## Directory Layout

```
core/fuzz/
  Cargo.toml               # cargo-fuzz manifest
  README.md                # This file
  fuzz_targets/
    fuzz_ring_buffer.rs    # Ring buffer fuzzer
    fuzz_toml_config.rs    # Config parser fuzzer
    fuzz_sif_record.rs     # Record / SIF format fuzzer
```

## CI Integration

Add to your CI pipeline:

```yaml
# GitHub Actions example
- name: Install cargo-fuzz
  run: cargo install cargo-fuzz

- name: Fuzz ring buffer (5 min)
  run: |
    cd core/fuzz
    cargo +nightly fuzz run fuzz_ring_buffer -- -max_total_time=300

- name: Fuzz TOML config (5 min)
  run: |
    cd core/fuzz
    cargo +nightly fuzz run fuzz_toml_config -- -max_total_time=300

- name: Fuzz Record SIF (5 min)
  run: |
    cd core/fuzz
    cargo +nightly fuzz run fuzz_sif_record -- -max_total_time=300
```

## Adding a New Fuzz Target

1. Create `core/fuzz/fuzz_targets/fuzz_<name>.rs` with a `#![no_main]` entry
   point using the `libfuzzer_sys::fuzz_target!` macro.
2. Add a `[[bin]]` section to `core/fuzz/Cargo.toml`.
3. Run `cargo +nightly fuzz build` to verify.
4. Add edge-case unit tests in a `#[cfg(test)]` module at the bottom of the
   target file.
