# DoLogger — Host Application Examples

Minimal, self-contained host apps in each supported language. Each one
demonstrates the **host-app layer** of the three-layer architecture: a real
program consuming `libdologger_core` through the public C ABI — nothing more.

| Example | Status |
|:-:|:-:|
| [c/](c/) | ✅ minimal C host (init → log → shutdown) |
| `rust/` | 🔜 ships with the adapter SDK |
| `python/` | 🔜 ships with the adapter SDK |
| `go/` | 🔜 ships with the adapter SDK |
| `zig/` | 🔜 added during the adapter CI-smoke milestone |

## Build the C example

```bash
# 1. Build the core library once (repo root):
cargo build --release

# 2. Build and run the example:
cmake -S examples/c -B examples/c/build
cmake --build examples/c/build
./examples/c/build/dologger-c-example
```

> **Note:** these root-level examples are independent host programs, distinct
> from `core/examples/*.rs`, which are `[[example]]` targets compiled *inside*
> the core crate for in-crate testing/benchmarks. Root `examples/` never links
> the Rust rlib — it goes through the C ABI like any external host would.
