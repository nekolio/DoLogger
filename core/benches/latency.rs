//! Latency benchmark — measures single-record submission latency (P50/P99).

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::sync::Arc;

use dologger_core::buffer::{RecordPool, RecordPtr, RingBuffer};
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
    let ring_buffer = Arc::new(RingBuffer::<RecordPtr>::new(config.ring_buffer_size));
    let time_source = TimeSource::new();
    let tid = 1u64;
    let pid = std::process::id();

    c.bench_function("single_record_submit", |b| {
        b.iter_batched(
            || {
                ring_buffer.drain(usize::MAX, |token| {
                    let ptr = token.into_raw();
                    // SAFETY: the token came from this pool and is consumed once.
                    unsafe {
                        pool.free(ptr);
                    }
                });
            },
            |_| {
                let record_ptr = pool.alloc().expect("Pool exhausted");
                unsafe {
                    let record = &mut *record_ptr;
                    let id = time_source.next_id();
                    record.set_id(id.hi, id.lo);
                    record.timestamp = time_source.now_nanos();
                    record.level = LogLevel::Info;
                    record.message.set("latency test");
                    record.thread_id = tid as u32;
                    record.process_id = pid;
                }
                let token = unsafe { RecordPtr::from_raw(record_ptr) };
                if let Err(token) = ring_buffer.try_push(token) {
                    // SAFETY: a rejected token remains owned by this producer.
                    unsafe {
                        pool.free(token.into_raw());
                    }
                }
            },
            BatchSize::PerIteration,
        )
    });

    let sig_engine = SignatureEngine::new();

    c.bench_function("single_record_submit_with_sign", |b| {
        b.iter_batched(
            || {
                ring_buffer.drain(usize::MAX, |token| {
                    let ptr = token.into_raw();
                    // SAFETY: the token came from this pool and is consumed once.
                    unsafe {
                        pool.free(ptr);
                    }
                });
            },
            |_| {
                let record_ptr = pool.alloc().expect("Pool exhausted");
                unsafe {
                    let record = &mut *record_ptr;
                    let id = time_source.next_id();
                    record.set_id(id.hi, id.lo);
                    record.timestamp = time_source.now_nanos();
                    record.level = LogLevel::Audit;
                    record.message.set("signed audit record");
                    record.thread_id = tid as u32;
                    record.process_id = pid;
                }
                let _sig = sig_engine.sign_record(unsafe { &mut *record_ptr }, &[0u8; 32]);
                let token = unsafe { RecordPtr::from_raw(record_ptr) };
                if let Err(token) = ring_buffer.try_push(token) {
                    // SAFETY: a rejected token remains owned by this producer.
                    unsafe {
                        pool.free(token.into_raw());
                    }
                }
            },
            BatchSize::PerIteration,
        )
    });

    ring_buffer.drain(usize::MAX, |token| {
        let ptr = token.into_raw();
        // SAFETY: the token came from this pool and is consumed once.
        unsafe {
            pool.free(ptr);
        }
    });
}

criterion_group!(benches, bench_single_record_latency);
criterion_main!(benches);
