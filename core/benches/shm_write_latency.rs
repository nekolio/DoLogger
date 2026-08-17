//! sink_shm hot-path write latency benchmark.
//!
//! Measures the `ShmSink::write` latency for a single SIF record — the
//! zero-copy shared-memory ring write that external consumers read with no
//! copying. Uses `DropOldest` so every iteration performs a real slot write
//! (the true hot path) rather than the cheaper full-buffer drop branch.
//!
//! Success target: P99 < 1 µs for a 128-byte SIF record.

use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

use dologger_core::sink::{ShmFullPolicy, ShmSink, ShmSinkConfig};
use dologger_core::sys::Sysmon;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of raw latency samples per measurement run.
const SAMPLES: usize = 200_000;

/// Shared-memory path used for the benchmark. Dropped/cleaned up by the sink
/// on close (auto_cleanup=true).
const SHM_PATH: &str = "/dologger_bench_shm.shm";

/// A small but valid SIF frame (16-byte overhead + a FlatBuffer root table
/// offset). The absolute bytes do not matter for latency measurement — only
/// that the slot copy happens on the hot path.
fn sif_sample() -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    // Magic "SIF1"
    buf.extend_from_slice(b"SIF1");
    // SifHeader: version(1.0.0) | total_length | record_count
    buf.extend_from_slice(&0x0100_0000u32.to_le_bytes());
    buf.extend_from_slice(&(16u32 + 4).to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());
    // FlatBuffer payload: root table offset 4 (minimal, self-referential)
    buf.extend_from_slice(&4u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // vtable offset
    buf.extend_from_slice(&4u32.to_le_bytes()); // table size
    buf.extend_from_slice(&4u32.to_le_bytes()); // vtable size
                                                // Pad to 128 bytes to model a realistic record payload.
    buf.resize(128, 0);
    buf
}

// ---------------------------------------------------------------------------
// Percentile helpers
// ---------------------------------------------------------------------------

fn compute_percentiles(mut samples: Vec<f64>) -> (f64, f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let len = samples.len();
    if len == 0 {
        return (0.0, 0.0, 0.0);
    }
    let p50 = samples[len * 50 / 100];
    let p99 = samples[len * 99 / 100];
    let p999 = samples[len * 999 / 1000];
    (p50, p99, p999)
}

fn format_ns(ns: f64) -> String {
    if ns >= 1_000.0 {
        format!("{:.3} µs", ns / 1_000.0)
    } else {
        format!("{:.0} ns", ns)
    }
}

// ---------------------------------------------------------------------------
// Core measurement
// ---------------------------------------------------------------------------

/// Open a `DropOldest` shm sink and measure `write` latency over `count` samples.
fn collect_write_latency(sink: &ShmSink, sif: &[u8], count: usize) -> Vec<f64> {
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        let start = std::time::Instant::now();
        let _written = sink.write(sif);
        samples.push(start.elapsed().as_nanos() as f64);
    }
    samples
}

fn bench_shm_write_latency(c: &mut Criterion) {
    // Quiet diagnostic output.
    #[cfg(target_os = "windows")]
    dologger_core::sys::diagnostics::init("NUL");
    #[cfg(not(target_os = "windows"))]
    dologger_core::sys::diagnostics::init("/dev/null");

    let sysmon = Sysmon::start();

    // 4 MiB buffer, 64 KiB slots, DropOldest so the ring always accepts the
    // write (exercising the full slot-copy hot path).
    let config = ShmSinkConfig {
        path: SHM_PATH.into(),
        buffer_size_mb: 4,
        slot_size_kb: 64,
        full_policy: ShmFullPolicy::DropOldest,
        ..ShmSinkConfig::default()
    };
    let sink = Arc::new(ShmSink::new(config));
    sink.open(&sysmon).expect("Failed to open shm sink");

    let sif = sif_sample();

    println!();
    println!("══════════════════════════════════════════════════");
    println!("  sink_shm write latency — DropOldest, 128B SIF");
    println!("  target: P99 < 1 µs");
    println!("══════════════════════════════════════════════════");

    let samples = collect_write_latency(&sink, &sif, SAMPLES);
    let (p50, p99, p999) = compute_percentiles(samples);
    let status = if p99 < 1000.0 { "PASS" } else { "FAIL" };
    println!("  samples:  {SAMPLES}");
    println!("  P50:      {}", format_ns(p50));
    println!("  P99:      {}   [{status}]", format_ns(p99));
    println!("  P99.9:    {}", format_ns(p999));
    println!("══════════════════════════════════════════════════");

    // Criterion-compatible benchmark of the same hot path.
    c.bench_function("shm_write_latency_128B", |b| {
        b.iter_batched(
            || (), // no reset — the ring overwrites via DropOldest
            |_| {
                sink.write(&sif);
            },
            BatchSize::PerIteration,
        )
    });

    sink.close(&sysmon);
    println!();
    println!("=== shm write latency complete ===\n");
}

criterion_group!(benches, bench_shm_write_latency);
criterion_main!(benches);
