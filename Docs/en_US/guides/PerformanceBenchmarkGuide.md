# DoLogger Performance Benchmark Guide

> 🌐 **语言 / Language**: [English](PerformanceBenchmarkGuide.md) | [中文：性能基准测试指南](../../zh_CN/guides/PerformanceBenchmarkGuide.md)

> **Version**: v0.1.0 | **Last Updated**: 2026-08-12 | **Target Audience**: Core Developers, Performance Engineers, Plugin Authors
>
> **Purpose**: This document describes how to run, interpret, and extend the DoLogger benchmark suite. It covers the benchmark harness, reference hardware, result interpretation (P50/P99/P99.9 percentiles and throughput), CI integration for regression detection, and conventions for adding new benchmarks.
>
> **Reading Path**: New contributors should read [Prerequisites](#prerequisites) and [Running Benchmarks](#running-benchmarks). Performance engineers should focus on [Interpreting Results](#interpreting-results) and [Reference Hardware](#reference-hardware). CI maintainers should read [CI Integration](#ci-integration).

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [The Benchmark Suite](#the-benchmark-suite)
3. [Running Benchmarks](#running-benchmarks)
4. [Reference Hardware](#reference-hardware)
5. [Interpreting Results](#interpreting-results)
6. [CI Integration](#ci-integration)
7. [Adding New Benchmarks](#adding-new-benchmarks)
8. [Benchmark Governance](#benchmark-governance)

---

## Prerequisites

### Environment Preparation

Benchmark results are highly sensitive to system state. Before running any benchmark, ensure the following conditions are met.

**Table 1: Benchmark Environment Requirements**

| Requirement | How to Verify | Impact if Not Met |
|:-:|:-:|:-:|
| CPU frequency locked (no turbo/speedstep) | `cpupower frequency-info` on Linux | Up to 30% variance between runs |
| No other CPU-intensive processes | `htop` / Task Manager — CPU idle > 95% | Noise in latency benchmarks, inflated P99 |
| No disk I/O contention | `iostat -x 1` — disk utilization < 5% | Variable throughput for file/WORM sink benchmarks |
| No network activity (for local benchmarks) | `iftop` / `nethogs` — near-zero traffic | Noise in latency measurements from interrupt handling |
| Adequate RAM (no swapping) | `free -h` — Swap used = 0B | Catastrophic latency spikes from page faults |
| Thermal throttling disabled | `sensors` — CPU temp stable, no throttling flags | Gradual throughput degradation over time |

### Linux-Specific Setup

```bash
# 1. Disable CPU frequency scaling
sudo cpupower frequency-set -g performance

# 2. Disable turbo boost (Intel)
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# 3. Disable turbo boost (AMD)
echo 0 | sudo tee /sys/devices/system/cpu/cpufreq/boost

# 4. Disable transparent huge pages (latency-sensitive benchmarks)
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/defrag

# 5. Set CPU affinity to isolate benchmark cores
sudo cset shield --cpu 2-15 --kthread=on

# 6. Increase max locked memory for ring buffer
sudo sysctl -w vm.max_map_count=262144
```

### macOS-Specific Setup

```bash
# Disable power management (requires disabling SIP for some settings)
sudo pmset -a disablesleep 1

# Disable Spotlight indexing on benchmark directories
sudo mdutil -i off /path/to/benchmark/output/
```

### Windows-Specific Setup

```powershell
# Set power plan to High Performance
powercfg /setactive 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c

# Disable Windows Defender real-time scanning on benchmark directories
Add-MpPreference -ExclusionPath "C:\benchmarks\"
```

### Toolchain Requirements

```bash
# Rust nightly or stable with LTO support
rustup show active-toolchain
# Expected: stable-x86_64-unknown-linux-gnu (or nightly)

# Verify criterion is available (benchmark framework)
cargo bench --help

# Install perf (Linux) for hardware counter profiling
sudo apt install linux-tools-common linux-tools-generic
```

---

## The Benchmark Suite

DoLogger provides four benchmark targets, each measuring a different dimension of performance.

**Table 2: Benchmark Target Overview**

| Benchmark | Crate / File | Measures | Primary Metric | Run Time (approx.) |
|:-:|:-:|:-:|:-:|:-:|
| `hot_path` | `benches/hot_path.rs` | End-to-end record submission latency | P50/P99/P99.9 ns | ~5 minutes |
| `latency` | `benches/latency.rs` | Per-pipeline-stage latency breakdown | P50 us per stage | ~8 minutes |
| `throughput` | `benches/throughput.rs` | Maximum sustained records/second | records/s | ~12 minutes |
| `latency_percentiles` | `benches/latency_percentiles.rs` | Full latency distribution (histogram) | P50/P90/P99/P99.9/P99.99 | ~15 minutes |

### `hot_path` — Submission Latency

Measures the time from `dologger_log()` call to return. This is the **most critical metric** for host applications — it represents the cost added to every log statement.

- **What it measures**: CAS push into the ring buffer + any cooperative helping
- **What it excludes**: Pipeline processing, formatting, sink I/O (these are asynchronous)
- **Units**: Nanoseconds
- **Statistical treatment**: Criterion.rs with 100 samples per iteration, 50 iterations

### `latency` — Pipeline Stage Breakdown

Measures the processing time of each pipeline stage independently:

| Stage | What is Timed |
|:-:|:-:|
| PreFilter | PolicyProvider evaluation |
| Filter | Filter plugin decision |
| FieldProvider | Field injection |
| Assembly | LSN assignment + Ed25519 sign + CRC32C |
| Processing | Processor plugin transformation |
| Formatting | Formatter serialization |
| Sink | IOSink write call |

### `throughput` — Maximum Sustainable Rate

Measures how many records per second the engine can process end-to-end under sustained load:

- **Configurations tested**: All four performance profiles (`dev`, `balanced`, `prod-performance`, `prod-audit`)
- **Sink variants**: Console, File (no fsync), File (fsync), WORM (sign + fsync)
- **Record sizes**: 64 B, 256 B, 1 KB, 4 KB messages
- **Producer thread counts**: 1, 2, 4, 8, 16

### `latency_percentiles` — Full Distribution

Generates a complete latency histogram with percentile breakdowns. Used to detect tail latency regressions and multi-modal distributions that indicate lock contention or GC pauses.

---

## Running Benchmarks

### Basic Usage

```bash
# Run a single benchmark suite
cargo bench --bench hot_path

# Run all benchmark suites
cargo bench

# Run a specific benchmark within a suite
cargo bench --bench hot_path -- single_record_submit

# Run with a specific performance profile
DO_LOG_PERF_PROFILE=prod-audit cargo bench --bench throughput
```

### Controlling Benchmark Parameters

```bash
# Override the sample size (faster, less statistical power)
cargo bench --bench hot_path -- --sample-size 20

# Set a fixed measurement time
cargo bench --bench latency -- --measurement-time 30

# Filter to a subset of benchmarks by name
cargo bench --bench throughput -- "file_sink/"

# Save baseline for later comparison
cargo bench --bench hot_path -- --save-baseline master

# Compare against a saved baseline
cargo bench --bench hot_path -- --baseline master
```

### Running with Profiling

```bash
# Linux perf — hardware counters
perf record --call-graph dwarf --freq=997 \
    cargo bench --bench hot_path -- single_record_submit
perf report
perf stat -e cycles,instructions,cache-references,cache-misses,branches,branch-misses \
    cargo bench --bench hot_path -- single_record_submit

# Flamegraph
perf record --call-graph dwarf --freq=997 \
    cargo bench --bench hot_path -- single_record_submit
perf script | stackcollapse-perf.pl | flamegraph.pl > hot_path_flamegraph.svg

# macOS Instruments
cargo instruments -t time --bench hot_path

# Valgrind / Callgrind (slow, highly accurate)
valgrind --tool=callgrind --dump-instr=yes \
    cargo bench --bench hot_path -- single_record_submit
kcachegrind callgrind.out.*
```

### Baseline Comparison Workflow

```bash
# Step 1: Establish a baseline on the current main branch
git checkout main
cargo bench --bench hot_path -- --save-baseline main

# Step 2: Run benchmarks on your feature branch
git checkout feature/my-change
cargo bench --bench hot_path -- --baseline main

# Step 3: Criterion reports regression/improvement automatically:
#  "Performance has improved by 3.2%"
#  "Performance has regressed by 7.1%"
```

---

## Reference Hardware

### Primary Reference Machine

All official benchmark numbers reported in DoLogger documentation are measured on this hardware configuration.

**Table 3: Primary Reference Hardware**

| Component | Specification |
|:-:|:-:|
| **CPU** | AMD Ryzen 9 7950X (16 cores / 32 threads, 4.5 GHz base, 5.7 GHz boost) |
| **RAM** | 64 GB DDR5-6000 (2 x 32 GB, dual-channel) |
| **Storage** | Samsung 990 Pro 2 TB NVMe (PCIe 4.0 x4) |
| **OS** | Ubuntu 24.04 LTS, Linux kernel 6.8 |
| **Rust** | 1.97.1 stable, `RUSTFLAGS="-C target-cpu=native -C lto=fat"` |
| **Compile flags** | `--release`, LTO enabled, `codegen-units=1` |

### Secondary Reference Machines

Benchmarks should also be validated on these configurations before release:

**Table 4: Secondary Reference Hardware**

| Platform | CPU | RAM | Storage | OS |
|:-:|:-:|:-:|:-:|:-:|
| Linux ARM | AWS Graviton3 (64 cores) | 128 GB DDR5 | EBS gp3 | Amazon Linux 2023 |
| macOS x86\_64 | Intel Core i9-13900K | 32 GB DDR5 | Apple SSD | macOS 14 Sonoma |
| macOS aarch64 | Apple M2 Max (12 cores) | 32 GB LPDDR5 | Apple SSD | macOS 14 Sonoma |
| Windows x86\_64 | AMD Ryzen 9 7950X | 64 GB DDR5 | Samsung 990 Pro | Windows 11 24H2 |

### Reporting Your Own Benchmark Results

When sharing benchmark results, include:

1. **Hardware**: CPU model, RAM speed/capacity, storage model, OS and kernel version
2. **Software**: Rust version, compiler flags (`RUSTFLAGS`), LTO setting
3. **Environment**: Frequency governor setting, turbo boost state, THP state
4. **DoLogger config**: Performance profile, `ring_buffer_size`, `batch_size`, `enable_signature`

Without this context, benchmark comparisons are unreliable.

---

## Interpreting Results

### Percentile Metrics Explained

**Table 5: Percentile Interpretation**

| Percentile | What It Tells You | Action if Elevated |
|:-:|:-:|:-:|
| **P50** (median) | Typical-case performance. Half of all operations complete faster than this. | High P50 suggests a systemic issue — hot-path algorithm, memory allocation, lock contention. |
| **P90** | Moderately degraded performance. 10% of operations are slower than this. | Elevated P90 suggests intermittent stalls — cache misses, brief lock holds, branch mispredictions. |
| **P99** | Tail latency. 1% of operations are slower than this. | High P99 indicates rare but impactful stalls — OS scheduling jitter, TLB misses, NUMA effects. |
| **P99.9** | Extreme tail. 0.1% of operations are slower than this. | High P99.9 points to very rare events — page faults, thermal throttling, GC pauses, kernel interrupts. |

### Throughput Interpretation

Throughput numbers should be read with these caveats:

- **Sustained vs burst**: The benchmark measures sustained throughput over 30+ seconds. Burst throughput (first 1-3 seconds) can be 2-3x higher.
- **Record size matters**: Larger records reduce throughput linearly (more memory to copy per record).
- **Sink type dominates**: The slowest enabled sink determines end-to-end throughput in fan-out mode.
- **Signing cost**: Ed25519 signing limits throughput to approximately 58,000 records/s regardless of sink.

**Table 6: Expected Performance Ranges (Reference Hardware)**

| Scenario | Throughput Range | P50 Latency | P99 Latency |
|:-:|:-:|:-:|:-:|
| Console Sink, signature off | 1.0M - 1.4M rec/s | 75 - 95 ns | 180 - 250 ns |
| File Sink, no fsync | 800K - 1.1M rec/s | 95 - 120 ns | 320 - 450 ns |
| File Sink, Ed25519 signing | 50K - 62K rec/s | 16 - 19 us | 20 - 25 us |
| WORM Sink, sign + fsync | 10K - 14K rec/s | 78 - 90 us | 125 - 160 us |
| Ring buffer CAS push (raw) | N/A (micro-benchmark) | 95 - 110 ns | 180 - 250 ns |

### Recognizing Anomalies

**Table 7: Common Benchmark Anomalies**

| Symptom | Likely Cause | Investigation |
|:-:|:-:|:-:|
| Bimodal latency distribution (two peaks) | Alternating between fast/slow paths — possible lock contention or NUMA remote access | Use `perf lock` to find contention; check NUMA node affinity |
| Gradual throughput degradation over time | Thermal throttling or memory leak | Monitor CPU temp and process RSS during benchmark |
| High variance (>20% between runs) | Turbo boost not disabled, background process interference | Verify prerequisites in [Environment Preparation](#environment-preparation) |
| P99.9 spike exceeding 100x P50 | Page faults from memory allocation on hot path | Check for heap allocation in VTable functions; use `perf record -e page-faults` |
| Throughput cliff at N threads | Ring buffer CAS contention (single cursor) | Known limitation — sharded ring buffer planned for M4; see [Architecture Reference](../ArchitectureReference.md#known-limitation) |

### Statistical Rigor

Criterion.rs (the benchmark framework) uses:

- **Warm-up**: Automatic warm-up phase before measurement begins
- **Outlier classification**: Tukey's fences (IQR method) to identify and report outliers
- **Confidence intervals**: 95% CI reported alongside every measurement
- **Regression detection**: t-test comparing baseline vs current, flags changes with p < 0.05

Treat changes within the 95% confidence interval as **noise**. Only changes that exceed the confidence interval AND the configured noise threshold (default: 2%) are reported as regressions or improvements.

---

## CI Integration

### Regression Detection Pipeline

The CI pipeline runs benchmarks on every pull request and compares against the `main` branch baseline.

**Table 8: CI Benchmark Jobs**

| Job | Trigger | Runtime | Failure Criteria |
|:-:|:-:|:-:|:-:|
| `bench-hot-path` | Every PR | ~5 min | > 5% regression on any metric |
| `bench-latency` | Every PR | ~8 min | > 5% regression on any stage |
| `bench-throughput` | Main branch only | ~12 min | > 5% regression on any sink |
| `bench-percentiles` | Main branch + weekly | ~15 min | > 10% regression on P99.9 |

### CI Configuration (GitHub Actions)

```yaml
# .github/workflows/benchmarks.yml
name: Benchmarks

on:
  pull_request:
    paths:
      - 'core/**'
      - 'plugins/**'
      - 'benches/**'

jobs:
  bench-hot-path:
    runs-on: [self-hosted, linux, x64, benchmark]
    steps:
      - uses: actions/checkout@v4
      - name: Setup benchmark environment
        run: |
          sudo cpupower frequency-set -g performance
          echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
      - name: Run hot_path benchmarks
        run: |
          cargo bench --bench hot_path -- --save-baseline pr
      - name: Compare against main baseline
        run: |
          git fetch origin main
          git checkout origin/main
          cargo bench --bench hot_path -- --save-baseline main
          git checkout -
          cargo bench --bench hot_path -- --baseline main --criterion-reports
      - name: Check for regressions
        run: |
          # Fail if any benchmark regressed by more than 5%
          python3 scripts/ci/check_benchmark_regression.py \
            --baseline main --threshold 5

  bench-latency:
    runs-on: [self-hosted, linux, x64, benchmark]
    steps:
      - uses: actions/checkout@v4
      - name: Run latency benchmarks
        run: cargo bench --bench latency
      - name: Check regressions
        run: python3 scripts/ci/check_benchmark_regression.py --threshold 5

  bench-throughput:
    runs-on: [self-hosted, linux, x64, benchmark]
    if: github.ref == 'refs/heads/main'
    steps:
      - uses: actions/checkout@v4
      - name: Run throughput benchmarks
        run: cargo bench --bench throughput
      - name: Store results as artifact
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: target/criterion/
```

### Regression Response

When CI detects a regression:

1. **Investigate**: The PR author examines the benchmark report to identify which metric regressed.
2. **Profile**: Run `perf` / flamegraph to isolate the cause.
3. **Fix or Accept**: Either fix the regression, or document and accept it:
   - Performance regressions for **security improvements** are accepted if the trade-off is documented in the PR description.
   - Performance regressions for **correctness fixes** are accepted with a `perf-regression-accepted` label.
   - All other regressions block merge.

### Self-Hosted Runner Requirements

CI benchmarks require dedicated, isolated hardware. The self-hosted runner must:

- Be a dedicated physical machine (not a VM, not shared)
- Have no other CI jobs running concurrently
- Pass the [environment preparation](#environment-preparation) checklist before every run
- Log CPU frequency, temperature, and memory pressure as CI artifacts alongside benchmark results

---

## Adding New Benchmarks

### File Structure

```
benches/
  hot_path.rs              ← Submission latency
  latency.rs               ← Pipeline stage breakdown
  throughput.rs            ← End-to-end throughput
  latency_percentiles.rs   ← Full distribution
  common/
    setup.rs               ← Shared harness: engine init, config, warm-up
    fixtures.rs            ← Pre-built record templates
    reporting.rs           ← Results formatting
```

### Benchmark Template

```rust
// benches/hot_path.rs — example structure

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use dologger_bench_common::{setup_engine, create_sample_record};

fn bench_single_record_submit(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_path");
    group.measurement_time(std::time::Duration::from_secs(10));
    group.sample_size(100);

    // Test across multiple ring buffer sizes
    for buf_size in [65536, 131072, 262144, 524288].iter() {
        let mut engine = setup_engine(*buf_size);
        let record = create_sample_record();

        group.bench_with_input(
            BenchmarkId::new("single_submit", buf_size),
            buf_size,
            |b, _| {
                b.iter(|| {
                    engine.log(black_box(&record))
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_single_record_submit);
criterion_main!(benches);
```

### Conventions

1. **Use `criterion`**: All benchmarks use the `criterion` crate (not libtest `#[bench]`). Criterion provides statistics, baseline comparison, and HTML reports.
2. **`black_box` all inputs**: Prevent the compiler from optimizing away benchmark code by wrapping inputs in `std::hint::black_box()` or `criterion::black_box()`.
3. **Warm-up phase**: Let criterion handle it. Do not manually warm up.
4. **One concern per benchmark**: Each `bench_with_input` group measures one operation. Do not combine submission latency with formatting time in the same measurement.
5. **Name benchmarks clearly**: Use descriptive `BenchmarkId` names — `"single_submit"`, `"batch_submit_256"`, `"file_sink_no_fsync"`.
6. **Record hardware context**: Every benchmark file includes a `// HARDWARE:` comment listing the expected reference configuration.
7. **Profile-sensitive benchmarks**: If a benchmark's behavior depends on the `performance_profile`, run it under all four profiles with `cfg` or environment variable detection.

### Benchmark Configuration Matrix

New benchmarks should test the following dimensions where applicable:

**Table 9: Benchmark Parameter Matrix**

| Parameter | Values |
|:-:|:-:|
| Performance profile | `dev`, `balanced`, `prod-performance`, `prod-audit` |
| Ring buffer size | 65536, 131072, 262144, 524288 |
| Batch size | 32, 128, 256, 512 |
| Record size | 64 B, 256 B, 1 KB, 4 KB |
| Producer threads | 1, 2, 4, 8, 16 |
| Ed25519 signing | On, Off |
| fsync | On, Off |

Not every benchmark needs every combination. Choose the dimensions relevant to the operation being measured.

### Example: Adding a Throughput Benchmark for a New Sink

```rust
// benches/throughput.rs

fn bench_throughput_new_sink(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput/new_sink");
    group.measurement_time(std::time::Duration::from_secs(30));

    for profile in ["prod-performance", "prod-audit"].iter() {
        let mut config = setup_config(profile);
        config.enable_sink("new_sink");

        let engine = setup_engine_with_config(&config);

        group.bench_with_input(
            BenchmarkId::new("sustained", profile),
            profile,
            |b, _| {
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for i in 0..iters {
                        engine.log(black_box(&RECORD_TEMPLATE));
                    }
                    start.elapsed()
                });
            },
        );
    }
    group.finish();
}
```

---

## Benchmark Governance

### Baseline Storage

Official baselines for each release are stored as Criterion data in the repository:

```
target/criterion/         ← Local only (not committed)
benches/baselines/         ← Committed baselines for CI comparison
  v1.0.0/
    hot_path.json
    latency.json
    throughput.json
    latency_percentiles.json
```

### Acceptable Performance Deviation

**Table 10: Performance Deviation Thresholds**

| Change Type | Hot Path P50 | Hot Path P99 | Throughput | Action |
|:-:|:-:|:-::|:-:|:-:|
| Within noise | < 2% | < 3% | < 2% | Accept — no action |
| Minor improvement | 2-10% | 3-10% | 2-10% | Accept — note in changelog |
| Minor regression | 2-5% | 3-5% | 2-5% | Review — document if intentional |
| Significant regression | > 5% | > 5% | > 5% | **Block merge** — investigate and fix |
| Critical regression | > 10% | > 10% | > 10% | **Block merge** — requires explicit justification |

### Reporting Benchmark Results

When publishing benchmark results (release notes, academic papers, marketing materials):

1. Always cite the reference hardware configuration.
2. Report **sustained** throughput, not burst.
3. Report P50, P99, and P99.9 together — never just one percentile.
4. Include the exact DoLogger configuration used.
5. Note whether Ed25519 signing was enabled (it dominates latency when on).
6. Include the benchmark command that produced the results.
7. Disclose whether results were produced on a dedicated benchmark machine or a shared system.
