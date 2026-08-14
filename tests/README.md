# DoLogger — Test Suites

Test taxonomy mapped to the canonical categories in the design doc (§3.3:
`tests/{common,integration,fuzz,performance,security}`). Some categories live
under `core/` because they are Cargo test/bench/fuzz targets that must sit
inside the crate they exercise — the table records where each canonical
category physically lives.

| Canonical category | Physical location | What it covers |
|:-:|:-:|:-:|
| **common** | [common/](common/) | Shared test utilities (`lib.sh`), mock plugins |
| **integration / system** | [smoke/](smoke/) | C ABI smoke (`c_abi_smoke.py`) + platform smoke runners (`check-smoke.sh`/`.ps1`) |
| **integration (in-crate)** | [../core/tests/](../core/tests/) | plugin bundle, plugin security, plugin sandbox, security, fanout sinks |
| **fuzz** | [../core/fuzz/](../core/fuzz/) | cargo-fuzz targets (ring buffer, SIF, TOML config) |
| **performance (Rust)** | [../core/benches/](../core/benches/) | Criterion benches (latency, throughput, percentiles) |
| **performance (C ABI)** | [perf/c_abi/](perf/c_abi/) | C log-throughput harness (`c_abi_bench`) driving `libdologger_core` |

Rust integration suites in [`core/tests/`](../core/tests/) are named
`{subject}.rs` and auto-discovered by Cargo (the core crate declares no
`[[test]]` entries) — run any of them with
`cargo test -p dologger-core --test <subject>`. The shared-memory and C ABI
perf suites under [`perf/`](perf/) are CMake-built and pointed at built
release artifacts, mirroring the smoke suite's artifact-driving model.
