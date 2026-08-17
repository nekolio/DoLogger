//! Pipeline scheduler for log record processing.
//!
//! # Implementation
//!
//! A background consumer thread drains the ring buffer and routes
//! each record through the full multi-stage pipeline:
//!
//! PreFilter → Filter → FieldProvider → Assembly → Processing → Formatting → Sink
//!
//! When an `io_pool` is provided, sink writes are dispatched to
//! I/O worker threads via a crossbeam channel, separating CPU-bound
//! processing from I/O-bound writes.
//!
//! Statistics are collected per stage and reported to sysmon/diag periodically.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::buffer::RecordPool;
use crate::buffer::RingBuffer;
use crate::config::DologgerConfig;
use crate::error::DO_LOG_ERR_BUFFER_TOO_SMALL;
use crate::pipeline::{report_stats, run_pipeline, PipelineContext};
use crate::plugin::vtable::{OutputBuffer, PluginDispatch};
use crate::policy::{DropLevelPolicy, RateLimiter};
use crate::record::Record;
use crate::security::SignatureEngine;
use crate::sink::ShmSink;
use crate::sink::SinkRef;
use crate::sys::ThreadPool;

/// Channel messages for I/O worker dispatch.
enum SinkMsg {
    /// Write a formatted record to the sink.
    Write(String),
    /// Flush the sink.
    Flush,
    /// Close the sink.
    Close,
}

/// Pipeline handle controlling the background consumer thread.
pub struct Pipeline {
    consumer_thread: Option<thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    /// Signalled by the I/O worker when it has completed flush+close.
    /// Only set when `io_pool` is used; used to synchronise shutdown.
    sink_done: Option<Arc<AtomicBool>>,
}

impl Pipeline {
    /// Create a new pipeline with the full multi-stage processing chain.
    ///
    /// * `io_pool` — optional I/O thread pool for sink writes.
    ///   When `None`, sink writes happen inline on the consumer thread.
    ///   When `Some`, writes are sent through a bounded crossbeam channel
    ///   to a worker running on the pool, decoupling CPU from I/O.
    /// * `shm_sink` — optional shared-memory sink. When present, each
    ///   accepted record is additionally serialised to SIF and written to the
    ///   ring buffer on the consumer thread (parallel to the configured sink).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &DologgerConfig,
        ring_buffer: Arc<RingBuffer<*mut Record>>,
        pool: Arc<RecordPool>,
        mut sink: SinkRef,
        signature_engine: Arc<SignatureEngine>,
        rate_limiter: Arc<RateLimiter>,
        drop_level_policy: Arc<DropLevelPolicy>,
        dispatch: PluginDispatch,
        io_pool: Option<Arc<ThreadPool>>,
        shm_sink: Option<Arc<ShmSink>>,
    ) -> Result<Self, String> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown);
        let batch_size = config.batch_size;
        let enable_signature = config.enable_signature;

        // When io_pool is Some, set up a channel-based dispatch.
        // The sink is moved into a worker running on the io_pool; the
        // consumer thread sends formatted records through the channel.
        // When io_pool is None, the sink stays inline.
        let (sink_tx, sink_done, mut consumer_sink) = match io_pool {
            Some(ref io_pool) => {
                let (tx, rx) = crossbeam_channel::bounded::<SinkMsg>(256);
                let done = Arc::new(AtomicBool::new(false));
                let done_clone = Arc::clone(&done);

                io_pool.execute(move || {
                    while let Ok(msg) = rx.recv() {
                        match msg {
                            SinkMsg::Write(data) => {
                                if let Err(e) = sink.write(&data) {
                                    crate::sys::diagnostics::error(
                                        "pipeline",
                                        &format!("Sink write error: {e}"),
                                    );
                                }
                            }
                            SinkMsg::Flush => {
                                let _ = sink.flush();
                            }
                            SinkMsg::Close => {
                                let _ = sink.close();
                            }
                        }
                    }
                    // Channel closed — final cleanup
                    let _ = sink.flush();
                    let _ = sink.close();
                    done_clone.store(true, Ordering::Release);
                });

                (Some(tx), Some(done), None)
            }
            None => (None, None, Some(sink)),
        };

        let consumer_thread = thread::Builder::new()
            .name("dologger-pipeline".into())
            .spawn(move || {
                let mut ctx = ConsumerCtx {
                    ring_buffer,
                    pool,
                    shutdown: shutdown_flag,
                    batch_size,
                    sink: consumer_sink.as_mut(),
                    signature_engine: &signature_engine,
                    rate_limiter: &rate_limiter,
                    drop_level_policy: &drop_level_policy,
                    enable_signature,
                    dispatch: &dispatch,
                    io_pool,
                    sink_tx,
                    shm_sink: shm_sink.as_ref(),
                };
                consumer_loop(&mut ctx);
            })
            .map_err(|e| format!("Failed to spawn pipeline consumer thread: {e}"))?;

        Ok(Self {
            consumer_thread: Some(consumer_thread),
            shutdown,
            sink_done,
        })
    }

    /// Initiate graceful shutdown, draining all in-flight records.
    ///
    /// When `io_pool` is active, this also waits for the I/O worker
    /// to finish flushing and closing the sink after the channel is
    /// disconnected.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.consumer_thread.take() {
            let _ = handle.join();
        }

        // If io_pool was used, wait for the sink worker to finish.
        if let Some(ref done) = self.sink_done {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !done.load(Ordering::Acquire) {
                if std::time::Instant::now() > deadline {
                    crate::sys::diagnostics::error(
                        "pipeline",
                        "Sink I/O worker did not finish within the shutdown deadline",
                    );
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// Parameters for the consumer loop.
///
/// When `sink_tx` is `Some` (io_pool enabled), writes go through
/// the channel. When `sink_tx` is `None`, `sink` provides inline
/// access.
struct ConsumerCtx<'a> {
    ring_buffer: Arc<RingBuffer<*mut Record>>,
    pool: Arc<RecordPool>,
    shutdown: Arc<AtomicBool>,
    batch_size: usize,
    sink: Option<&'a mut SinkRef>,
    signature_engine: &'a SignatureEngine,
    rate_limiter: &'a RateLimiter,
    drop_level_policy: &'a DropLevelPolicy,
    enable_signature: bool,
    /// Resolved plugin dispatch (formatter + field-provider vtables, M6). A
    /// reference held by the consumer thread (the `PluginDispatch` itself lives
    /// in `Pipeline::new`'s closure), loaned to each `PipelineContext`. Kept a
    /// `&'a` reference — like `signature_engine`/`rate_limiter` — so copying it
    /// into a `PipelineContext` does not re-borrow `self` and block the
    /// `&mut self` sink writes in `dispatch_write`.
    dispatch: &'a PluginDispatch,
    /// The channel sender MUST be dropped before `io_pool` so the sink worker
    /// unblocks (channel disconnects) before the thread pool attempts to join
    /// its worker threads in `ThreadPool::drop`.
    sink_tx: Option<crossbeam_channel::Sender<SinkMsg>>,
    /// I/O thread pool; stored for ordered drop (must outlive channel sender)
    #[allow(dead_code)]
    io_pool: Option<Arc<ThreadPool>>,
    /// Optional shared-memory sink. Written per accepted record (SIF) on the
    /// consumer thread, parallel to the configured sink.
    shm_sink: Option<&'a Arc<ShmSink>>,
}

impl ConsumerCtx<'_> {
    /// Write a formatted record — dispatches through the channel when
    /// io_pool is active, otherwise writes inline.
    fn dispatch_write(&mut self, formatted: String) {
        if let Some(ref tx) = self.sink_tx {
            // Channel-based dispatch — send blocks if the I/O
            // worker is behind, providing natural backpressure.
            if tx.send(SinkMsg::Write(formatted)).is_err() {
                crate::sys::diagnostics::error(
                    "pipeline",
                    "Sink channel disconnected — write dropped",
                );
            }
        } else if let Some(ref mut sink) = self.sink {
            if let Err(e) = sink.write(&formatted) {
                crate::sys::diagnostics::error("pipeline", &format!("Sink write error: {e}"));
            }
        }
    }

    /// Flush the sink.
    fn dispatch_flush(&mut self) {
        if let Some(ref tx) = self.sink_tx {
            let _ = tx.send(SinkMsg::Flush);
        } else if let Some(ref mut sink) = self.sink {
            let _ = sink.flush();
        }
    }

    /// Close the sink.
    fn dispatch_close(&mut self) {
        if let Some(ref tx) = self.sink_tx {
            let _ = tx.send(SinkMsg::Close);
        } else if let Some(ref mut sink) = self.sink {
            let _ = sink.close();
        }
    }
}

/// Format a record for the sink, dispatching to the first loaded formatter
/// plugin when present, else falling back to the built-in plain-text format.
///
/// When a formatter is loaded, it writes into an engine-owned growable
/// [`OutputBuffer`]; on `DO_LOG_ERR_BUFFER_TOO_SMALL` the buffer is grown and
/// retried. A plugin error or an unreasonable size requirement falls back to
/// the built-in format so a misbehaving formatter can never lose a record.
fn format_record(record: &Record, dispatch: &PluginDispatch) -> String {
    let Some(fmt) = dispatch.formatters.first() else {
        return SinkRef::format_record(record);
    };
    let record_ptr = record as *const Record as *const std::ffi::c_void;
    let mut cap: usize = 256;
    loop {
        let mut backing = vec![0u8; cap];
        let mut ob = OutputBuffer {
            data: backing.as_mut_ptr(),
            len: 0,
            capacity: cap,
        };
        // SAFETY: fmt.format is a plugin-provided C-ABI fn; `record` is a live
        // Record handle and `ob` is a valid engine-owned buffer for the call.
        let rc = unsafe { (fmt.format)(record_ptr, &mut ob, fmt.config) };
        if rc == 0 {
            backing.truncate(ob.len);
            return String::from_utf8_lossy(&backing).into_owned();
        }
        if rc == DO_LOG_ERR_BUFFER_TOO_SMALL && cap < (1 << 20) {
            cap *= 2;
            continue;
        }
        // Plugin error or unreachable size cap — fall back to built-in format.
        return SinkRef::format_record(record);
    }
}

/// Main consumer loop — drains ring buffer, runs multi-stage pipeline.
fn consumer_loop(c: &mut ConsumerCtx<'_>) {
    let empty_sleep = Duration::from_micros(100);
    let stats_interval = Duration::from_secs(5);
    let mut last_stats_report = std::time::Instant::now();
    let mut total_processed: u64 = 0;
    let mut total_dropped: u64 = 0;

    while !c.shutdown.load(Ordering::Acquire) {
        let mut batch_processed = 0u64;
        let mut batch_dropped = 0u64;

        let mut pctx = PipelineContext::new(
            c.signature_engine,
            c.rate_limiter,
            c.drop_level_policy,
            c.enable_signature,
            c.dispatch,
        );

        // Collect formatted records for batch dispatch
        let mut pending_writes: Vec<String> = Vec::with_capacity(c.batch_size);

        let drained = c.ring_buffer.drain(c.batch_size, |record_ptr| {
            // SAFETY: record_ptr has exclusive ownership during drain.
            let record = unsafe { &mut *record_ptr };

            if run_pipeline(record, &mut pctx) {
                pending_writes.push(format_record(record, c.dispatch));
                batch_processed += 1;
            } else {
                batch_dropped += 1;
            }

            // Mirror the accepted record into the shared-memory sink (if any)
            // while it is still owned by the consumer. SIF serialisation is
            // cheap and the shm write is non-blocking / lossy by design.
            if let Some(shm) = c.shm_sink {
                let sif = crate::sif::encode_record(record);
                shm.write(&sif);
            }

            // SAFETY: record_ptr was obtained from this pool via alloc() and
            // has exclusive ownership during the drain callback. It has not
            // been freed yet.
            unsafe {
                c.pool.free(record as *const Record);
            }
        });

        // Dispatch all formatted records after drain (avoids borrowing issues)
        for formatted in pending_writes {
            c.dispatch_write(formatted);
        }

        total_processed += batch_processed;
        total_dropped += batch_dropped;

        if drained == 0 {
            thread::sleep(empty_sleep);
        }

        if last_stats_report.elapsed() >= stats_interval {
            report_stats(&pctx, c.batch_size);
            if total_processed > 0 || total_dropped > 0 {
                crate::sys::diagnostics::info(
                    "pipeline",
                    &format!(
                        "Periodic: {total_processed} processed, {total_dropped} dropped total"
                    ),
                );
            }
            last_stats_report = std::time::Instant::now();
        }
    }

    // Final drain on shutdown
    let mut pctx = PipelineContext::new(
        c.signature_engine,
        c.rate_limiter,
        c.drop_level_policy,
        c.enable_signature,
        c.dispatch,
    );

    let mut final_writes: Vec<String> = Vec::new();
    let remaining = c.ring_buffer.drain(usize::MAX, |record_ptr| {
        // SAFETY: final drain on shutdown — record_ptr has exclusive ownership.
        let record = unsafe { &mut *record_ptr };
        if run_pipeline(record, &mut pctx) {
            final_writes.push(format_record(record, c.dispatch));
        }

        // Mirror the accepted record into the shared-memory sink (if any).
        if let Some(shm) = c.shm_sink {
            let sif = crate::sif::encode_record(record);
            shm.write(&sif);
        }

        // SAFETY: record_ptr was obtained from this pool via alloc() and
        // has exclusive ownership at shutdown (final drain). It has not
        // been freed yet.
        unsafe {
            c.pool.free(record as *const Record);
        }
    });

    for formatted in final_writes {
        c.dispatch_write(formatted);
    }

    if remaining > 0 {
        crate::sys::diagnostics::info(
            "pipeline",
            &format!("Shutdown: flushed {remaining} remaining records"),
        );
    }

    report_stats(&pctx, remaining);

    // Final flush and close — dispatched through the same path.
    c.dispatch_flush();
    c.dispatch_close();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{thread_id_u64, LogLevel};
    use crate::sink::{Sink, SinkResult};
    use crate::sys::TimeSource;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    /// A test sink that records the thread ID of each `write()` call
    /// so we can verify that I/O ran on a different thread than the consumer.
    struct ThreadTrackingSink {
        records_written: Arc<AtomicUsize>,
        /// Thread IDs that called `write()`.
        write_threads: Arc<Mutex<Vec<thread::ThreadId>>>,
        is_open: bool,
    }

    impl ThreadTrackingSink {
        fn new(
            records_written: Arc<AtomicUsize>,
            write_threads: Arc<Mutex<Vec<thread::ThreadId>>>,
        ) -> Self {
            Self {
                records_written,
                write_threads,
                is_open: false,
            }
        }
    }

    impl Sink for ThreadTrackingSink {
        fn open(&mut self) -> SinkResult {
            self.is_open = true;
            Ok(())
        }

        fn write(&mut self, _formatted: &str) -> SinkResult {
            self.records_written.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut threads) = self.write_threads.lock() {
                threads.push(thread::current().id());
            }
            Ok(())
        }

        fn flush(&mut self) -> SinkResult {
            Ok(())
        }

        fn close(&mut self) -> SinkResult {
            self.is_open = false;
            Ok(())
        }
    }

    #[test]
    fn test_pipeline_with_inline_sink_writes() {
        // Without io_pool, writes happen inline on the consumer thread.
        let config = DologgerConfig::dev_profile();
        let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
        let ring_buffer = Arc::new(RingBuffer::new(config.ring_buffer_size));

        let records_written = Arc::new(AtomicUsize::new(0));
        let write_threads = Arc::new(Mutex::new(Vec::new()));

        let mut sink = SinkRef::new(ThreadTrackingSink::new(
            Arc::clone(&records_written),
            Arc::clone(&write_threads),
        ));
        sink.open().unwrap();

        let mut pipeline = Pipeline::new(
            &config,
            Arc::clone(&ring_buffer),
            Arc::clone(&pool),
            sink,
            Arc::new(SignatureEngine::new()),
            Arc::new(RateLimiter::default()),
            Arc::new(DropLevelPolicy::new(LogLevel::Trace)),
            PluginDispatch::default(), // no plugins loaded
            None,                      // No io_pool — inline writes
            None,                      // No shm sink
        )
        .expect("Pipeline creation should succeed");

        let time_source = TimeSource::new();
        let tid = thread_id_u64();
        let pid = std::process::id();

        // Submit a batch of records
        const N: usize = 100;
        for i in 0..N {
            let record_ptr = pool.alloc().expect("Pool exhausted");
            // SAFETY: pool.alloc() returns a valid, exclusively-owned pointer
            // from the pre-allocated object pool. No other thread holds a
            // reference to this record until it is pushed to the ring buffer.
            unsafe {
                let record = &mut *record_ptr;
                record.id = time_source.next_id();
                record.timestamp = time_source.now_utc();
                record.level = LogLevel::Info;
                record.message.set(&format!("test record {i}"));
                record.thread_id = tid;
                record.process_id = pid;
                record.process_name.set("test");
                record.host_name.set("localhost");
                record.environment.set("test");
            }
            ring_buffer
                .try_push(record_ptr)
                .expect("Ring buffer should accept");
        }

        pipeline.shutdown();

        assert_eq!(
            records_written.load(Ordering::Relaxed),
            N,
            "All records should be written"
        );
    }

    #[test]
    fn test_pipeline_with_io_pool_offloads_writes() {
        // With io_pool enabled, writes must happen on a different thread
        // than the current (test) thread and, critically, on a thread
        // that is NOT the consumer thread (dologger-pipeline).
        let config = DologgerConfig::dev_profile();
        let pool = Arc::new(RecordPool::new(config.ring_buffer_size));
        let ring_buffer = Arc::new(RingBuffer::new(config.ring_buffer_size));

        let records_written = Arc::new(AtomicUsize::new(0));
        let write_threads: Arc<Mutex<Vec<thread::ThreadId>>> = Arc::new(Mutex::new(Vec::new()));

        let mut sink = SinkRef::new(ThreadTrackingSink::new(
            Arc::clone(&records_written),
            Arc::clone(&write_threads),
        ));
        sink.open().unwrap();

        let io_pool = Arc::new(ThreadPool::new(2, "io-test"));

        let mut pipeline = Pipeline::new(
            &config,
            Arc::clone(&ring_buffer),
            Arc::clone(&pool),
            sink,
            Arc::new(SignatureEngine::new()),
            Arc::new(RateLimiter::default()),
            Arc::new(DropLevelPolicy::new(LogLevel::Trace)),
            PluginDispatch::default(), // no plugins loaded
            Some(io_pool),             // io_pool enabled
            None,                      // No shm sink
        )
        .expect("Pipeline creation should succeed");

        let time_source = TimeSource::new();
        let tid = thread_id_u64();
        let pid = std::process::id();

        const N: usize = 100;
        for i in 0..N {
            let record_ptr = pool.alloc().expect("Pool exhausted");
            // SAFETY: pool.alloc() returns a valid, exclusively-owned pointer
            // from the pre-allocated object pool. No other thread holds a
            // reference to this record until it is pushed to the ring buffer.
            unsafe {
                let record = &mut *record_ptr;
                record.id = time_source.next_id();
                record.timestamp = time_source.now_utc();
                record.level = LogLevel::Info;
                record.message.set(&format!("test record {i}"));
                record.thread_id = tid;
                record.process_id = pid;
                record.process_name.set("test");
                record.host_name.set("localhost");
                record.environment.set("test");
            }
            ring_buffer
                .try_push(record_ptr)
                .expect("Ring buffer should accept");
        }

        pipeline.shutdown();

        // All records must have been delivered.
        assert_eq!(
            records_written.load(Ordering::Relaxed),
            N,
            "All records should be written via io_pool"
        );

        // All writes must have happened on a thread whose name starts
        // with "dologger-io" — the I/O pool workers.
        let threads = write_threads.lock().unwrap();
        assert!(
            !threads.is_empty(),
            "At least one write should have occurred"
        );

        let test_tid = thread::current().id();
        for &tid_write in threads.iter() {
            // The write must NOT happen on the test thread.
            assert_ne!(
                tid_write, test_tid,
                "Sink writes must NOT happen on the test thread when io_pool is used"
            );
        }
    }
}
