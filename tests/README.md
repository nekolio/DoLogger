# DoLogger — Test Suites

Test taxonomy mapped to the canonical categories in the design doc (§3.3:
`tests/{common,integration,fuzz,performance,security}`). Some categories live
under `core/` because they are Cargo test/bench/fuzz targets that must sit
inside the crate they exercise — the table records where each canonical
category physically lives.

| Canonical category | Physical location | What it covers |
|:-:|:-:|:-:|
| **common** | [common/](common/) | Shared test utilities, mock plugins |
| **integration / system** | [release-smoke/](release-smoke/) | C ABI smoke (`cabi_smoke.py`) + platform smoke runners |
| **integration (in-crate)** | [../core/tests/](../core/tests/) | plugin bundle, plugin security, security tests |
| **fuzz** | [../core/fuzz/](../core/fuzz/) | cargo-fuzz targets (ring buffer, SIF, TOML) |
| **performance** | [../core/benches/](../core/benches/) | Criterion benches (latency, throughput, percentiles) |
| **security** | [security/](security/) | sandbox isolation / BPF seccomp / policy tests |

> `tests/security/sandbox_escape/` documents how to wire its in-process tests
> into the core crate — see its own README for the copy/symlink instructions.
