//! File logger example — demonstrates FileSink with the DoLogger pipeline.
//!
//! Creates an engine with file sink, submits log records through the
//! ring buffer pipeline, writes to `dologger_output.log`.
//!
//! Usage:
//!   cargo run --example file_logger
//!   cat dologger_output.log

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
use dologger_core::sink::{DurabilityLevel, SinkRef};
use dologger_core::sink::{FileSink, FileSinkConfig};
use dologger_core::sys::io;
use dologger_core::sys::TimeSource;

fn main() {
    io::stdout_line("=== DoLogger File Logger Example ===");
    io::stdout_line("");

    let output_path = "dologger_output.log";
    let config = DologgerConfig::dev_profile();

    io::stdout_line(&format!("Output: {output_path}"));
    io::stdout_line(&format!(
        "Profile: {:?}, buffer_size: {}, batch: {}",
        config.performance_profile, config.ring_buffer_size, config.batch_size
    ));
    io::stdout_line("");

    let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
    let ring_buffer = Arc::new(RingBuffer::new(config.ring_buffer_size));

    // FileSink with fsync for AUDIT records
    let sink = Arc::new(SinkRef::new(FileSink::new(FileSinkConfig {
        path: output_path.into(),
        max_size: 10 * 1024 * 1024, // 10MB rotation
        fsync_on_write: true,
        durability_level: DurabilityLevel::Media,
        buffer_size: 65536,
    })));
    sink.open().expect("Failed to open file sink");

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

    const NUM_RECORDS: usize = 5000;
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
                .set(&format!("File log #{i}: persisted to disk"));
            record.thread_id = tid;
            record.process_id = pid;
            record.process_name.set("file_logger");
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

    // Wait for pipeline to finish writing
    io::stdout_line("Waiting for pipeline to complete fsync...");
    std::thread::sleep(std::time::Duration::from_millis(500));

    pipeline.shutdown();

    // Verify file was written
    match std::fs::metadata(output_path) {
        Ok(meta) => {
            io::stdout_line(&format!(
                "File written: {output_path} ({:.1} KB)",
                meta.len() as f64 / 1024.0
            ));
        }
        Err(e) => {
            io::stderr_line(&format!("ERROR: Output file not created: {e}"));
        }
    }

    let total_elapsed = start.elapsed();
    io::stdout_line("");
    io::stdout_line(&format!(
        "=== Complete: {NUM_RECORDS} records in {total_elapsed:?} ==="
    ));
}
