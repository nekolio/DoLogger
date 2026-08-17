//! Simple logger example — demonstrates the DoLogger internal API.
//!
//! Creates an engine with console sink, submits 10,000 log records
//! through the ring buffer pipeline.
//!
//! All diagnostic output uses `io::stdout_line`/`io::stderr_line`
//! (platform-native syscalls), NOT libc stdio macros.
//!
//! Usage:
//!   cargo run --example simple_logger

use std::sync::Arc;
use std::time::Instant;

use dologger_core::buffer::RecordPool;
use dologger_core::buffer::RingBuffer;
use dologger_core::config::DologgerConfig;
use dologger_core::pipeline::Pipeline;
use dologger_core::plugin::vtable::PluginDispatch;
use dologger_core::policy::{DropLevelPolicy, RateLimiter};
use dologger_core::record::{thread_id_u64, LogLevel};
use dologger_core::security::SignatureEngine;
use dologger_core::sink::{ConsoleSink, SinkRef};
use dologger_core::sys::io;
use dologger_core::sys::TimeSource;

fn main() {
    io::stdout_line("=== DoLogger Simple Logger Example ===");
    io::stdout_line("");

    let config = DologgerConfig::dev_profile();
    io::stdout_line(&format!(
        "Config: level={}, profile={:?}, buffer={}, batch={}",
        config.level, config.performance_profile, config.ring_buffer_size, config.batch_size
    ));
    io::stdout_line("");

    let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
    let ring_buffer = Arc::new(RingBuffer::new(config.ring_buffer_size));
    let sink = Arc::new(SinkRef::new(ConsoleSink::new()));
    sink.open().expect("Failed to open sink");

    let time_source = TimeSource::new();

    let mut pipeline = Pipeline::new(
        &config,
        Arc::clone(&ring_buffer),
        Arc::clone(&pool),
        Arc::clone(&sink),
        Arc::new(SignatureEngine::new()),
        Arc::new(RateLimiter::default()),
        Arc::new(DropLevelPolicy::new(LogLevel::Trace)),
        PluginDispatch::default(),
        None,
        None,
    )
    .expect("Failed to create pipeline");

    const NUM_RECORDS: usize = 10_000;
    let start = Instant::now();

    let tid = thread_id_u64();
    let pid = std::process::id();

    for i in 0..NUM_RECORDS {
        let record_ptr = pool.alloc().expect("Pool exhausted");

        unsafe {
            let record = &mut *record_ptr;
            record.id = time_source.next_id();
            record.timestamp = time_source.now_utc();
            record.level = match i % 7 {
                1 => LogLevel::Debug,
                2 => LogLevel::Info,
                3 => LogLevel::Warn,
                4 => LogLevel::Error,
                5 => LogLevel::Fatal,
                6 => LogLevel::Audit,
                _ => LogLevel::Trace,
            };
            record
                .message
                .set(&format!("Log message #{i}: Hello from DoLogger!"));
            record.thread_id = tid;
            record.process_id = pid;
            record.process_name.set("simple_logger");
            record.host_name.set("localhost");
            record.environment.set("dev");
        }

        if ring_buffer.try_push(record_ptr).is_err() {
            io::stderr_line(&format!("Ring buffer full at record {i}"));
            break;
        }
    }

    let submit_elapsed = start.elapsed();
    io::stdout_line(&format!(
        "Submitted {NUM_RECORDS} records in {submit_elapsed:?} \
         ({:.0} records/sec)",
        NUM_RECORDS as f64 / submit_elapsed.as_secs_f64()
    ));
    io::stdout_line("");

    // Give the pipeline time to drain and output the last batch
    io::stdout_line("Waiting for pipeline to drain...");
    io::stdout_line("");
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Graceful shutdown
    pipeline.shutdown();

    let total_elapsed = start.elapsed();
    io::stdout_line("");
    io::stdout_line(&format!(
        "=== Complete: {NUM_RECORDS} records in {total_elapsed:?} ==="
    ));
}
