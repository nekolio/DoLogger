//! Latency benchmark — measures single-record submission latency (P50/P99).

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::sync::Arc;

use dologger_core::buffer::RecordPool;
use dologger_core::buffer::RingBuffer;
use dologger_core::config::DologgerConfig;
use dologger_core::record::LogLevel;
use dologger_core::security::SignatureEngine;
use dologger_core::sys::TimeSource;

/// Bench the hot path: alloc → fill → CAS push → drain (single record round-trip).
///
/// Uses `iter_batched` so Criterion drains the ring buffer between batches,
/// preventing pool exhaustion across thousands of iterations.
fn bench_single_record_latency(c: &mut Criterion) {
    let config = DologgerConfig::dev_profile();
    let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
    let ring_buffer = Arc::new(RingBuffer::new(config.ring_buffer_size));
    let time_source = TimeSource::new();
    let tid = 1u64;
    let pid = std::process::id();

    c.bench_function("single_record_submit", |b| {
        b.iter_batched(
            || {
                // Setup: ensure pool has free slots
                ring_buffer.drain(usize::MAX, |ptr| {
                    // SAFETY: ptr was allocated from this pool and not yet freed
                    unsafe {
                        pool.free(ptr);
                    }
                });
            },
            |_| {
                let record_ptr = pool.alloc().expect("Pool exhausted");
                unsafe {
                    let record = &mut *record_ptr;
                    record.id = time_source.next_id();
                    record.timestamp = time_source.now_utc();
                    record.level = LogLevel::Info;
                    record.message.set("latency test");
                    record.thread_id = tid;
                    record.process_id = pid;
                }
                let _ = ring_buffer.try_push(record_ptr);
            },
            BatchSize::PerIteration,
        )
    });

    // With Ed25519 signing (AUDIT path)
    let sig_engine = SignatureEngine::new();

    c.bench_function("single_record_submit_with_sign", |b| {
        b.iter_batched(
            || {
                ring_buffer.drain(usize::MAX, |ptr| {
                    // SAFETY: ptr was allocated from this pool and not yet freed
                    unsafe {
                        pool.free(ptr);
                    }
                });
            },
            |_| {
                let record_ptr = pool.alloc().expect("Pool exhausted");
                unsafe {
                    let record = &mut *record_ptr;
                    record.id = time_source.next_id();
                    record.timestamp = time_source.now_utc();
                    record.level = LogLevel::Audit;
                    record.message.set("signed audit record");
                    record.thread_id = tid;
                    record.process_id = pid;
                }
                let sig = sig_engine.sign_record(unsafe { &mut *record_ptr });
                unsafe {
                    (*record_ptr).signature = sig;
                }
                let _ = ring_buffer.try_push(record_ptr);
            },
            BatchSize::PerIteration,
        )
    });

    // Final drain
    ring_buffer.drain(usize::MAX, |ptr| {
        // SAFETY: ptr was allocated from this pool and not yet freed
        unsafe {
            pool.free(ptr);
        }
    });
}

criterion_group!(benches, bench_single_record_latency);
criterion_main!(benches);
