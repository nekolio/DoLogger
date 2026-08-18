//! Performance benchmark command for `dologctl`.
//!
//! Quick local performance test that creates a minimal engine,
//! pushes N records, measures submit latency, and prints
//! P50/P99/P99.9 percentiles.  Uses the same percentile calculation
//! as the criterion benchmarks in `core/benches/latency_percentiles.rs`.

use std::time::Instant;

use dologger_core::buffer::RecordPool;
use dologger_core::buffer::RingBuffer;
use dologger_core::record::{thread_id_u64, LogLevel, RECORD_STRING_INLINE_MAX};
use dologger_core::sys::TimeSource;

use crate::output::{self, color, OutputFormat};
use crate::stdout;

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

fn cyan() -> &'static str {
    output::when_color(color::CYAN)
}
fn yellow() -> &'static str {
    output::when_color(color::YELLOW)
}
fn red() -> &'static str {
    output::when_color(color::RED)
}
fn bold() -> &'static str {
    output::when_color(color::BOLD)
}
fn dim() -> &'static str {
    output::when_color(color::DIM)
}
fn bright_green() -> &'static str {
    output::when_color(color::BRIGHT_GREEN)
}
fn bright_cyan() -> &'static str {
    output::when_color(color::BRIGHT_CYAN)
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum ring buffer size (64K — matches dev profile).
const MIN_RING_SIZE: usize = 65536;

/// How often to drain during sample collection to prevent pool exhaustion.
/// We drain when the ring buffer reaches this many records.
const DRAIN_AT: usize = 32768;

// ===========================================================================
// Public command entry point
// ===========================================================================

/// Run a local performance benchmark.
///
/// * `count`        — total number of records to push (default 100_000).
/// * `message_size` — length of each log message in bytes (default 80).
/// * `format`       — output format (text or json).
pub fn cmd_perf(count: usize, message_size: usize, format: OutputFormat) {
    if format == OutputFormat::Json {
        cmd_perf_json(count, message_size);
        return;
    }
    cmd_perf_text(count, message_size);
}

fn cmd_perf_text(count: usize, message_size: usize) {
    // Clamp message size to the inline capacity (heap fallback would skew
    // the benchmark toward allocation; keep the hot path representative).
    let msg_size = message_size.min(RECORD_STRING_INLINE_MAX);
    let message = "x".repeat(msg_size);

    // Compute ring buffer size as next power-of-two >= count, minimum 64K.
    let ring_size = count.next_power_of_two().max(MIN_RING_SIZE);

    let b = bold();
    let bc = bright_cyan();
    let d = dim();
    let c = cyan();
    let y = yellow();
    let r = red();
    let bg = bright_green();
    let reset = output::when_color(color::RESET);

    stdout!("{b}{bc}DoLogger Performance Benchmark{reset}");
    stdout!("{d}──────────────────────────────────{reset}");
    stdout!("  Records:       {count}");
    stdout!("  Message size:  {msg_size} bytes");
    stdout!(
        "  Ring buffer:   {ring_size} slots ({d}{size_kb} KiB{reset})",
        size_kb = ring_size * std::mem::size_of::<*mut dologger_core::record::Record>() / 1024
    );
    stdout!("");

    // --- Initialise ---
    let pool = RecordPool::new(ring_size);
    let rb = RingBuffer::new(ring_size);
    let ts = TimeSource::new();
    let tid = thread_id_u64();
    let pid = std::process::id();

    // --- Warm-up: 10K records to stabilise CPU frequency / caches ---
    stdout!("{d}Warming up (10 000 records)...{reset}");
    {
        let mut warm_samples = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            if i > 0 && i % DRAIN_AT == 0 {
                drain_all(&pool, &rb);
            }
            warm_samples.push(push_one(&pool, &rb, &ts, &message, tid, pid));
        }
        drain_all(&pool, &rb);
        let (p50, _, _) = compute_percentiles(warm_samples);
        stdout!("{d}  Warm-up P50: {}{reset}", format_ns(p50));
    }
    stdout!("");

    // --- Benchmark run ---
    stdout!("{d}Benchmarking ({count} records)...{reset}");

    let wall_start = Instant::now();
    let mut latencies_ns: Vec<f64> = Vec::with_capacity(count);

    for i in 0..count {
        if i > 0 && i % DRAIN_AT == 0 {
            drain_all(&pool, &rb);
        }

        latencies_ns.push(push_one(&pool, &rb, &ts, &message, tid, pid));
    }

    // Final drain
    drain_all(&pool, &rb);

    let wall_elapsed = wall_start.elapsed();
    let total_secs = wall_elapsed.as_secs_f64();
    let throughput = count as f64 / total_secs;

    // --- Percentile calculation ---
    let (p50, p99, p999) = compute_percentiles(latencies_ns.clone());

    // --- Report ---
    stdout!("");
    stdout!("{b}Results{reset}");
    stdout!("{d}───────{reset}");
    stdout!("  Total time:       {:.3} s", total_secs);
    stdout!("  Records pushed:   {count}");
    stdout!("  Throughput:       {b}{bg}{throughput:.0}{reset} rec/s");
    stdout!("");
    stdout!("{b}  Submit Latency (push → ring buffer){reset}");
    stdout!("  {d}─────────────────────────────────────{reset}");
    stdout!("    P50:     {c}{}{reset}", format_ns(p50));
    stdout!("    P99:     {y}{}{reset}", format_ns(p99));
    stdout!("    P99.9:   {r}{}{reset}", format_ns(p999));
    stdout!("");

    let min_ns = latencies_ns.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_ns = latencies_ns
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let avg_ns = latencies_ns.iter().sum::<f64>() / latencies_ns.len() as f64;

    stdout!(
        "  {d}Min: {}   Max: {}   Avg: {}{reset}",
        format_ns(min_ns),
        format_ns(max_ns),
        format_ns(avg_ns),
    );
}

fn cmd_perf_json(count: usize, message_size: usize) {
    let msg_size = message_size.min(RECORD_STRING_INLINE_MAX);
    let message = "x".repeat(msg_size);
    let ring_size = count.next_power_of_two().max(MIN_RING_SIZE);

    let pool = RecordPool::new(ring_size);
    let rb = RingBuffer::new(ring_size);
    let ts = TimeSource::new();
    let tid = thread_id_u64();
    let pid = std::process::id();

    // Warm-up (silent in JSON mode)
    {
        let mut warm_samples = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            if i > 0 && i % DRAIN_AT == 0 {
                drain_all(&pool, &rb);
            }
            warm_samples.push(push_one(&pool, &rb, &ts, &message, tid, pid));
        }
        drain_all(&pool, &rb);
    }

    // Benchmark run
    let wall_start = Instant::now();
    let mut latencies_ns: Vec<f64> = Vec::with_capacity(count);
    for i in 0..count {
        if i > 0 && i % DRAIN_AT == 0 {
            drain_all(&pool, &rb);
        }
        latencies_ns.push(push_one(&pool, &rb, &ts, &message, tid, pid));
    }
    drain_all(&pool, &rb);

    let wall_elapsed = wall_start.elapsed();
    let total_secs = wall_elapsed.as_secs_f64();
    let throughput = count as f64 / total_secs;
    let (p50, p99, p999) = compute_percentiles(latencies_ns.clone());
    let min_ns = latencies_ns.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_ns = latencies_ns
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let avg_ns = latencies_ns.iter().sum::<f64>() / latencies_ns.len() as f64;

    let obj = serde_json::json!({
        "count": count,
        "message_size_bytes": msg_size,
        "ring_buffer_slots": ring_size,
        "wall_time_secs": total_secs,
        "throughput_rec_per_sec": throughput,
        "latency_ns": {
            "p50": p50,
            "p99": p99,
            "p999": p999,
            "min": min_ns,
            "max": max_ns,
            "avg": avg_ns
        }
    });
    output::stdout_line(&obj.to_string());
}

// ===========================================================================
// Internal helpers
// ===========================================================================

/// Push a single Info-level record into the ring buffer and return the
/// elapsed wall-clock time in nanoseconds.
fn push_one(
    pool: &RecordPool,
    rb: &RingBuffer<*mut dologger_core::record::Record>,
    ts: &TimeSource,
    msg: &str,
    tid: u64,
    pid: u32,
) -> f64 {
    let start = Instant::now();

    let ptr = pool.alloc().expect("Pool exhausted during benchmark");
    unsafe {
        let record = &mut *ptr;
        record.id = ts.next_id();
        record.timestamp = ts.now_utc();
        record.level = LogLevel::Info;
        record.message.set(msg);
        record.thread_id = tid;
        record.process_id = pid;
        record.process_name.set("dologctl-perf");
        record.host_name.set("localhost");
        record.environment.set("bench");
    }

    let _ = rb.try_push(ptr);
    start.elapsed().as_nanos() as f64
}

/// Drain every record from the ring buffer and return them to the pool.
fn drain_all(pool: &RecordPool, rb: &RingBuffer<*mut dologger_core::record::Record>) {
    rb.drain(usize::MAX, |ptr| {
        // SAFETY: ptr was allocated from this pool and has not been freed
        unsafe {
            pool.free(ptr);
        }
    });
}

// ===========================================================================
// Percentile calculation
// ===========================================================================

/// Compute P50, P99, P99.9 from a vector of latency samples (nanoseconds).
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

// ===========================================================================
// Formatting
// ===========================================================================

/// Format a nanosecond value for human-readable display.
pub(crate) fn format_ns(ns: f64) -> String {
    if ns >= 1_000_000.0 {
        format!("{:.3} ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.1} µs", ns / 1_000.0)
    } else {
        format!("{:.0} ns", ns)
    }
}
