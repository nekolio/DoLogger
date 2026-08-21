//! Throughput benchmark — measures records/sec through the ring buffer pipeline.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::sync::Arc;

use dologger_core::buffer::{RecordPool, RecordPtr, RingBuffer};
use dologger_core::config::DologgerConfig;
use dologger_core::record::LogLevel;
use dologger_core::sys::TimeSource;

fn bench_ring_buffer_push(c: &mut Criterion) {
    let config = DologgerConfig::dev_profile();
    let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
    let ring_buffer = Arc::new(RingBuffer::<RecordPtr>::new(config.ring_buffer_size));
    let time_source = TimeSource::new();
    let tid = 1u64;
    let pid = std::process::id();

    c.bench_function("ring_buffer_push_1k", |b| {
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
                for i in 0..1000 {
                    let record_ptr = pool.alloc().expect("Pool exhausted");
                    unsafe {
                        let record = &mut *record_ptr;
                        let id = time_source.next_id();
                        record.set_id(id.hi, id.lo);
                        record.timestamp = time_source.now_nanos();
                        record.level = LogLevel::Info;
                        record.message.set(&format!("bench message #{i}"));
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
                }
            },
            BatchSize::PerIteration,
        )
    });

    c.bench_function("ring_buffer_push_batch_256", |b| {
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
                let batch: Vec<RecordPtr> = (0..256)
                    .map(|_| {
                        let record_ptr = pool.alloc().expect("Pool exhausted");
                        unsafe {
                            let record = &mut *record_ptr;
                            let id = time_source.next_id();
                            record.set_id(id.hi, id.lo);
                            record.timestamp = time_source.now_nanos();
                            record.level = LogLevel::Info;
                            record.message.set("batch message");
                            record.thread_id = tid as u32;
                            record.process_id = pid;
                        }
                        unsafe { RecordPtr::from_raw(record_ptr) }
                    })
                    .collect();

                for token in batch {
                    if let Err(token) = ring_buffer.try_push(token) {
                        // SAFETY: a rejected token remains owned by this producer.
                        unsafe {
                            pool.free(token.into_raw());
                        }
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

criterion_group!(benches, bench_ring_buffer_push);
criterion_main!(benches);
