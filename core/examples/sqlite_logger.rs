//! SQLite logger example — demonstrates SqliteSink.
//!
//! Compile with `--features sink-sqlite`:
//!   cargo run --features sink-sqlite --example sqlite_logger
//!   sqlite3 dologger.db "SELECT level, message FROM dologger_records LIMIT 5"

fn main() {
    if !cfg!(feature = "sink-sqlite") {
        eprintln!(
            "This example requires: cargo run --features sink-sqlite --example sqlite_logger"
        );
    }

    #[cfg(feature = "sink-sqlite")]
    {
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
        use dologger_core::sink::SinkRef;
        use dologger_core::sink::SqliteSink;
        use dologger_core::sys::io;
        use dologger_core::sys::TimeSource;

        io::stdout_line("=== DoLogger SQLite Sink Example ===");
        io::stdout_line("");

        let db_path = "dologger.db";
        let config = DologgerConfig::dev_profile();

        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_file(format!("{db_path}-wal"));
        let _ = std::fs::remove_file(format!("{db_path}-shm"));

        let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
        let ring_buffer = Arc::new(RingBuffer::new(config.ring_buffer_size));

        let mut sink = SinkRef::new(SqliteSink::with_path(db_path));
        sink.open().expect("Failed to open SQLite sink");

        io::stdout_line(&format!("Database: {db_path}"));
        io::stdout_line("");

        let time_source = TimeSource::new();

        let mut pipeline = Pipeline::new(
            &config,
            Arc::clone(&ring_buffer),
            Arc::clone(&pool),
            sink,
            Arc::new(SignatureEngine::new()),
            Arc::new(RateLimiter::default()),
            Arc::new(DropLevelPolicy::new(LogLevel::Trace)),
            PluginDispatch::default(), // no plugins loaded in this example
            None,
            None,
        )
        .expect("Failed to create pipeline");

        const NUM_RECORDS: usize = 1000;
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
                record.message.set(&format!("SQLite log #{i}"));
                record.thread_id = tid;
                record.process_id = pid;
                record.process_name.set("sqlite_logger");
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
            "Submitted {NUM_RECORDS} records in {submit_elapsed:?} ({:.0} rec/s)",
            NUM_RECORDS as f64 / submit_elapsed.as_secs_f64()
        ));

        std::thread::sleep(std::time::Duration::from_millis(300));
        pipeline.shutdown();

        match std::fs::metadata(db_path) {
            Ok(meta) => {
                io::stdout_line(&format!(
                    "Database: {db_path} ({:.1} KB)",
                    meta.len() as f64 / 1024.0
                ));
            }
            Err(e) => {
                io::stderr_line(&format!("ERROR: {e}"));
            }
        }

        let total_elapsed = start.elapsed();
        io::stdout_line("");
        io::stdout_line(&format!(
            "=== Complete: {NUM_RECORDS} records in {total_elapsed:?} ==="
        ));
        io::stdout_line("Query: sqlite3 dologger.db \"SELECT count(*) FROM dologger_records\"");
    }
}
