//! Throughput benchmark — measures records/sec through the ring buffer pipeline.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::sync::Arc;

use dologger_core::buffer::RecordPool;
use dologger_core::buffer::RingBuffer;
use dologger_core::config::DologgerConfig;
use dologger_core::record::{LogLevel, Record};
use dologger_core::sys::TimeSource;

fn bench_ring_buffer_push(c: &mut Criterion) {
    let config = DologgerConfig::dev_profile();
    let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
    let ring_buffer = Arc::new(RingBuffer::new(config.ring_buffer_size));
    let time_source = TimeSource::new();
    let tid = 1u64;
    let pid = std::process::id();

    // 1,000 records per batch
    c.bench_function("ring_buffer_push_1k", |b| {
        b.iter_batched(
            || {
                // Setup: drain to free pool slots
                ring_buffer.drain(usize::MAX, |ptr| {
                    // SAFETY: ptr was allocated from this pool and not yet freed
                    unsafe {
                        pool.free(ptr);
                    }
                });
            },
            |_| {
                for i in 0..1000 {
                    let record_ptr = pool.alloc().expect("Pool exhausted");
                    unsafe {
                        let record = &mut *record_ptr;
                        record.id = time_source.next_id();
                        record.timestamp = time_source.now_utc();
                        record.level = LogLevel::Info;
                        record.message.set(&format!("bench message #{i}"));
                        record.thread_id = tid;
                        record.process_id = pid;
                    }
                    let _ = ring_buffer.try_push(record_ptr);
                }
            },
            BatchSize::PerIteration,
        )
    });

    // 256-record batch pre-allocation
    c.bench_function("ring_buffer_push_batch_256", |b| {
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
                let batch: Vec<*mut Record> = (0..256)
                    .map(|_| {
                        let record_ptr = pool.alloc().expect("Pool exhausted");
                        unsafe {
                            let record = &mut *record_ptr;
                            record.id = time_source.next_id();
                            record.timestamp = time_source.now_utc();
                            record.level = LogLevel::Info;
                            record.message.set("batch message");
                            record.thread_id = tid;
                            record.process_id = pid;
                        }
                        record_ptr
                    })
                    .collect();

                for ptr in batch {
                    let _ = ring_buffer.try_push(ptr);
                }
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

criterion_group!(benches, bench_ring_buffer_push);
criterion_main!(benches);
