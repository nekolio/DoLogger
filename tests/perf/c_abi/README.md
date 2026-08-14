# DoLogger — C ABI Performance Harness

Home for the C-language performance harnesses that measure the DoLogger core
through its public C ABI (`dologger_core.h`). Rust Criterion benches covering
the in-process Rust API live separately in [`core/benches/`](../../../core/benches/).

## `c_abi_bench`

Log-throughput benchmark: drives `dologger_log()` from a compiled C host and
reports records/sec. Loads `libdologger_core` at runtime (dlopen /
LoadLibrary), so the same binary can be pointed at any release artifact
without a link step.

|:-:|:-:|
| What it measures | the C ABI submission fast path — `dologger_log()` push into the ring buffer (background threads sink asynchronously) |
| Source | [`c_abi_bench.c`](c_abi_bench.c) |
| Depends on | `core/include/dologger_core.h`, a built core shared library |

## Build

```bash
cmake -S tests/perf/c_abi -B build/tests/perf-c-abi
cmake --build build/tests/perf-c-abi
```

## Run

```bash
# default: 1,000,000 timed records after 100,000 warmup
./build/tests/perf-c-abi/c_abi_bench libdologger_core.so

# custom: 5,000,000 timed records after 500,000 warmup
./build/tests/perf-c-abi/c_abi_bench libdologger_core.so 5000000 500000
```

The library path points at the release artifact — e.g.
`release-artifacts/libdologger_core.so` (CI layout) or
`target/release/libdologger_core.so` (local build). The engine starts with
config auto-discovery (`dologger_init(NULL)`), so the pipeline is shaped by
the operator's `dologger.toml` as usual.

## Notes

- `dologger_log()` returns after enqueueing into the lock-free ring buffer;
  the reported rate is the submission fast path, not end-to-end persistence.
  Add I/O-bound sinks to your `dologger.toml` if you need the full pipeline
  cost.
- The record template is fixed (INFO, `bench` domain, source-location fields
  set) so the measurement stays a pure function of the call path.
