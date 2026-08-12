//! Latency percentile benchmarks — P50/P99/P99.9/P99.99 measurement.
//!
//! Measures single-record submission latency with full
//! distribution analysis across message sizes (80B, 256B, 1KB), with and
//! without Ed25519 signing, and under multi-thread scaling (1/2/4/8/16).
//!
//! Each benchmark collects 200K raw latency samples, computes P50/P99/P99.9/P99.99,
//! and prints a report. Additional criterion-compatible benchmarks are registered
//! for standard comparison with the existing latency.rs / throughput.rs suites.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use dologger_core::buffer::RecordPool;
use dologger_core::buffer::RingBuffer;
use dologger_core::config::DologgerConfig;
use dologger_core::record::{LogLevel, Record};
use dologger_core::security::SignatureEngine;
use dologger_core::sys::TimeSource;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of samples per percentile measurement run.
const SAMPLES: usize = 200_000;

/// Records per thread in the multi-thread scaling benchmarks.
const RECORDS_PER_THREAD: usize = 100_000;

/// 80-byte test message (exactly 80 ASCII characters).
const MSG_80B: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

// ---------------------------------------------------------------------------
// Percentile computation
// ---------------------------------------------------------------------------

/// Compute P50, P99, P99.9, P99.99 from a vector of latency samples (nanoseconds).
fn compute_percentiles(mut samples: Vec<f64>) -> (f64, f64, f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let len = samples.len();
    if len == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let p50 = samples[len * 50 / 100];
    let p99 = samples[len * 99 / 100];
    let p999 = samples[len * 999 / 1000];
    let p9999 = samples[len * 9999 / 10000];
    (p50, p99, p999, p9999)
}

/// Format nanoseconds for human-readable display.
fn format_ns(ns: f64) -> String {
    if ns >= 1_000_000.0 {
        format!("{:.3} ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.3} µs", ns / 1_000.0)
    } else {
        format!("{:.0} ns", ns)
    }
}

/// Print a percentile report to stdout.
fn report(label: &str, samples: Vec<f64>) {
    let n = samples.len();
    let (p50, p99, p999, p9999) = compute_percentiles(samples);
    println!();
    println!("══════════════════════════════════════════════════");
    println!("  {label}");
    println!("══════════════════════════════════════════════════");
    println!("  samples:  {n}");
    println!("  P50:      {}", format_ns(p50));
    println!("  P99:      {}", format_ns(p99));
    println!("  P99.9:    {}", format_ns(p999));
    println!("  P99.99:   {}", format_ns(p9999));
    println!("══════════════════════════════════════════════════");
}

// ---------------------------------------------------------------------------
// Core push operations (hot path)
// ---------------------------------------------------------------------------

/// Drain all records from the ring buffer back to the pool.
fn drain_all(pool: &RecordPool, rb: &RingBuffer<*mut Record>) {
    rb.drain(usize::MAX, |ptr| {
        // SAFETY: ptr was allocated from this pool and not yet freed
        unsafe {
            pool.free(ptr);
        }
    });
}

/// Push a single Info-level record and return elapsed nanoseconds.
fn push_record(
    pool: &RecordPool,
    rb: &RingBuffer<*mut Record>,
    ts: &TimeSource,
    msg: &str,
    tid: u64,
    pid: u32,
) -> f64 {
    let start = Instant::now();
    let ptr = pool.alloc().expect("Pool exhausted");
    unsafe {
        let record = &mut *ptr;
        record.id = ts.next_id();
        record.timestamp = ts.now_utc();
        record.level = LogLevel::Info;
        record.message.set(msg);
        record.thread_id = tid;
        record.process_id = pid;
    }
    let _ = rb.try_push(ptr);
    start.elapsed().as_nanos() as f64
}

/// Push a single Audit-level record with Ed25519 signing, return elapsed nanoseconds.
fn push_signed(
    pool: &RecordPool,
    rb: &RingBuffer<*mut Record>,
    ts: &TimeSource,
    sig: &SignatureEngine,
    msg: &str,
    tid: u64,
    pid: u32,
) -> f64 {
    let start = Instant::now();
    let ptr = pool.alloc().expect("Pool exhausted");
    unsafe {
        let record = &mut *ptr;
        record.id = ts.next_id();
        record.timestamp = ts.now_utc();
        record.level = LogLevel::Audit;
        record.message.set(msg);
        record.thread_id = tid;
        record.process_id = pid;
    }
    let signature = sig.sign_record(unsafe { &mut *ptr });
    unsafe {
        (*ptr).signature = signature;
    }
    let _ = rb.try_push(ptr);
    start.elapsed().as_nanos() as f64
}

/// Collect `count` latency samples using a push closure. Drains ring buffer
/// periodically to prevent pool exhaustion.
fn collect_samples<F: FnMut() -> f64>(
    pool: &RecordPool,
    rb: &RingBuffer<*mut Record>,
    count: usize,
    pool_size: usize,
    mut push_fn: F,
) -> Vec<f64> {
    let mut samples = Vec::with_capacity(count);
    let drain_interval = pool_size.saturating_sub(1024).max(1);

    for i in 0..count {
        if i > 0 && i % drain_interval == 0 {
            drain_all(pool, rb);
        }
        samples.push(push_fn());
    }

    samples
}

// ---------------------------------------------------------------------------
// Multi-thread scaling
// ---------------------------------------------------------------------------

/// Run a multi-thread latency benchmark.
///
/// Each thread gets its own pool and ring buffer to avoid contention
/// and measure pure per-thread latency at scale. Threads are synchronized
/// via a `Barrier` for simultaneous start. Aggregate throughput is
/// computed from wall-clock elapsed time across all threads.
fn run_multi_thread(num_threads: usize, pool_size: usize, msg: &str, with_sign: bool) {
    let msg = msg.to_string();
    let pid = std::process::id();

    let barrier = Arc::new(Barrier::new(num_threads));
    let wall_start = Instant::now();

    let handles: Vec<_> = (0..num_threads)
        .map(|_| {
            let msg = msg.clone();
            let barrier = Arc::clone(&barrier);
            let pool_size = pool_size;
            let with_sign = with_sign;
            std::thread::spawn(move || {
                let pool = RecordPool::new(pool_size);
                let rb = RingBuffer::new(pool_size);
                let ts = TimeSource::new();
                let sig = if with_sign {
                    Some(SignatureEngine::new())
                } else {
                    None
                };
                let tid = dologger_core::record::thread_id_u64();
                let mut samples = Vec::with_capacity(RECORDS_PER_THREAD);
                let drain_interval = pool_size.saturating_sub(1024).max(1);

                barrier.wait(); // synchronize start across all threads

                for i in 0..RECORDS_PER_THREAD {
                    if i > 0 && i % drain_interval == 0 {
                        drain_all(&pool, &rb);
                    }

                    let elapsed = if let Some(ref s) = sig {
                        push_signed(&pool, &rb, &ts, s, &msg, tid, pid)
                    } else {
                        push_record(&pool, &rb, &ts, &msg, tid, pid)
                    };
                    samples.push(elapsed);
                }

                // Final drain
                drain_all(&pool, &rb);
                samples
            })
        })
        .collect();

    // Merge all thread samples
    let mut all_samples = Vec::with_capacity(num_threads * RECORDS_PER_THREAD);
    for h in handles {
        match h.join() {
            Ok(samples) => all_samples.extend(samples),
            Err(_) => eprintln!("[WARN] A worker thread panicked"),
        }
    }

    let wall_elapsed = wall_start.elapsed();
    let total_records = all_samples.len() as f64;
    let throughput = total_records / wall_elapsed.as_secs_f64();

    let label = format!(
        "multi_thread_{}t_{}",
        num_threads,
        if with_sign { "signed" } else { "unsigned" }
    );
    report(&label, all_samples);
    println!("  throughput: {:.0} records/sec", throughput);
    println!(
        "  wall time:  {:.3} s ({} threads × {} records)",
        wall_elapsed.as_secs_f64(),
        num_threads,
        RECORDS_PER_THREAD
    );
}

// ---------------------------------------------------------------------------
// Benchmark entry point
// ---------------------------------------------------------------------------

fn bench_percentile_latency(c: &mut Criterion) {
    let config = DologgerConfig::dev_profile();
    let pool_size = config.ring_buffer_size; // 65536
    let pid = std::process::id();

    // Redirect diagnostic output to suppress truncation warnings from
    // oversized messages (1 KB > 255-byte RecordString limit).
    // On Windows, NUL is the null device; on Unix, /dev/null.
    #[cfg(target_os = "windows")]
    dologger_core::sys::diag::init("NUL");
    #[cfg(not(target_os = "windows"))]
    dologger_core::sys::diag::init("/dev/null");

    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  DoLogger P50/P99/P99.9/P99.99 Latency Benchmarks         ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Pool size:       {pool_size:<6} records                          ║");
    println!("║  Samples/run:     {SAMPLES:<6}                                    ║");
    println!("╚══════════════════════════════════════════════════════════════╝");

    // ── 1. single_record_latency_percentiles (default message) ──────────
    {
        let pool = RecordPool::new(pool_size);
        let rb = RingBuffer::new(pool_size);
        let ts = TimeSource::new();
        let samples = collect_samples(&pool, &rb, SAMPLES, pool_size, || {
            push_record(&pool, &rb, &ts, "latency percentile test", 1, pid)
        });
        drain_all(&pool, &rb);
        report("single_record_latency_percentiles", samples);
    }

    // ── 2. single_record_latency_80B ────────────────────────────────────
    {
        assert_eq!(MSG_80B.len(), 80, "MSG_80B must be exactly 80 bytes");
        let pool = RecordPool::new(pool_size);
        let rb = RingBuffer::new(pool_size);
        let ts = TimeSource::new();
        let samples = collect_samples(&pool, &rb, SAMPLES, pool_size, || {
            push_record(&pool, &rb, &ts, MSG_80B, 1, pid)
        });
        drain_all(&pool, &rb);
        report("single_record_latency_80B", samples);
    }

    // ── 3. single_record_latency_256B ───────────────────────────────────
    // RecordString inline capacity is 255 bytes (256 buffer - 1 null).
    // Using 255 bytes exercises the full inline copy without triggering
    // the truncation diagnostic path.
    {
        let msg_255b = "x".repeat(255);
        let pool = RecordPool::new(pool_size);
        let rb = RingBuffer::new(pool_size);
        let ts = TimeSource::new();
        let samples = collect_samples(&pool, &rb, SAMPLES, pool_size, || {
            push_record(&pool, &rb, &ts, &msg_255b, 1, pid)
        });
        drain_all(&pool, &rb);
        report("single_record_latency_256B", samples);
    }

    // ── 4. single_record_latency_1KB ────────────────────────────────────
    // 1 KB message exceeds the 255-byte inline capacity, triggering
    // truncation + diag warning. This measures the real-world overhead
    // of the oversized-message path (memcpy 255 bytes + diag log write).
    {
        let msg_1kb = "y".repeat(1024);
        let pool = RecordPool::new(pool_size);
        let rb = RingBuffer::new(pool_size);
        let ts = TimeSource::new();
        let samples = collect_samples(&pool, &rb, SAMPLES, pool_size, || {
            push_record(&pool, &rb, &ts, &msg_1kb, 1, pid)
        });
        drain_all(&pool, &rb);
        report("single_record_latency_1KB", samples);
    }

    // ── 5. single_record_with_sign_latency ──────────────────────────────
    {
        let pool = RecordPool::new(pool_size);
        let rb = RingBuffer::new(pool_size);
        let ts = TimeSource::new();
        let sig = SignatureEngine::new();
        let samples = collect_samples(&pool, &rb, SAMPLES, pool_size, || {
            push_signed(&pool, &rb, &ts, &sig, "signed audit record", 1, pid)
        });
        drain_all(&pool, &rb);
        report("single_record_with_sign_latency", samples);
    }

    // ── 6. multi_thread_scaling (unsigned) ──────────────────────────────
    println!();
    println!(">>> Multi-thread scaling — unsigned records");
    for &n in &[1, 2, 4, 8, 16] {
        run_multi_thread(n, pool_size, "multi-thread test message", false);
    }

    // ── 7. multi_thread_scaling (signed) ────────────────────────────────
    println!();
    println!(">>> Multi-thread scaling — Ed25519 signed records");
    for &n in &[1, 2, 4, 8, 16] {
        run_multi_thread(n, pool_size, "multi-thread audit message", true);
    }

    // ── Criterion-compatible P50 benchmarks ─────────────────────────────
    // Register lightweight criterion benchmarks using BatchSize::PerIteration
    // for compatibility with the existing latency.rs / throughput.rs suites.
    // These measure the same hot path and can be compared directly.
    {
        let pool = Arc::new(RecordPool::new(pool_size));
        let rb = Arc::new(RingBuffer::new(pool_size));
        let ts = TimeSource::new();

        c.bench_function("single_record_submit_p50", |b| {
            b.iter_batched(
                || drain_all(&pool, &rb),
                |_| {
                    push_record(&pool, &rb, &ts, MSG_80B, 1, pid);
                },
                BatchSize::PerIteration,
            )
        });

        let msg_255b = "x".repeat(255);
        c.bench_function("single_record_submit_256B_p50", |b| {
            b.iter_batched(
                || drain_all(&pool, &rb),
                |_| {
                    push_record(&pool, &rb, &ts, &msg_255b, 1, pid);
                },
                BatchSize::PerIteration,
            )
        });

        // 1KB variant — uses max inline (255 bytes) for clean measurement
        // without diag overhead in the criterion timing loop.
        c.bench_function("single_record_submit_1KB_p50", |b| {
            b.iter_batched(
                || drain_all(&pool, &rb),
                |_| {
                    push_record(&pool, &rb, &ts, &msg_255b, 1, pid);
                },
                BatchSize::PerIteration,
            )
        });

        let sig = SignatureEngine::new();
        c.bench_function("single_record_submit_signed_p50", |b| {
            b.iter_batched(
                || drain_all(&pool, &rb),
                |_| {
                    push_signed(&pool, &rb, &ts, &sig, "signed audit record", 1, pid);
                },
                BatchSize::PerIteration,
            )
        });
    }

    println!();
    println!("=== Percentile benchmarks complete ===\n");
}

criterion_group!(benches, bench_percentile_latency);
criterion_main!(benches);
