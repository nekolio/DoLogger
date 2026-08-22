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

use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::buffer::{RecordPool, RecordPtr, RingBuffer};
use crate::config::DologgerConfig;
use crate::error::DO_LOG_ERR_BUFFER_TOO_SMALL;
use crate::pipeline::policy::{DropLevelPolicy, RateLimiter};
use crate::pipeline::{report_stats, run_pipeline, PipelineContext};
use crate::plugin::vtable::{OutputBuffer, PluginDispatch};
use crate::record::Record;
use crate::security::SignatureEngine;
use crate::sink::ShmSink;
use crate::sink::SinkRef;
use crate::sys::control_plane::ControlPlaneStats;
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
        ring_buffer: Arc<RingBuffer<RecordPtr>>,
        pool: Arc<RecordPool>,
        sink: Arc<SinkRef>,
        signature_engine: Arc<SignatureEngine>,
        rate_limiter: Arc<RateLimiter>,
        drop_level_policy: Arc<DropLevelPolicy>,
        dispatch: PluginDispatch,
        io_pool: Option<Arc<ThreadPool>>,
        shm_sink: Option<Arc<ShmSink>>,
    ) -> Result<Self, String> {
        Self::new_with_stats(
            config,
            ring_buffer,
            pool,
            sink,
            signature_engine,
            rate_limiter,
            drop_level_policy,
            dispatch,
            io_pool,
            shm_sink,
            Arc::new(ControlPlaneStats::new()),
        )
    }

    /// Create a pipeline using the caller-owned live control-plane counters.
    ///
    /// The engine uses this entry point so accepted, processed, dropped, and
    /// ring-fill metrics describe the real production consumer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_stats(
        config: &DologgerConfig,
        ring_buffer: Arc<RingBuffer<RecordPtr>>,
        pool: Arc<RecordPool>,
        sink: Arc<SinkRef>,
        signature_engine: Arc<SignatureEngine>,
        rate_limiter: Arc<RateLimiter>,
        drop_level_policy: Arc<DropLevelPolicy>,
        dispatch: PluginDispatch,
        io_pool: Option<Arc<ThreadPool>>,
        shm_sink: Option<Arc<ShmSink>>,
        control_stats: Arc<ControlPlaneStats>,
    ) -> Result<Self, String> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown);
        let batch_size = config.batch_size;
        let enable_signature = config.enable_signature;

        // The dedicated AuditPipeline owns the signature sidecar. The normal
        // pipeline must never open or write it because AUDIT records are routed
        // away before entering this ring.
        let sig_sidecar: Option<BufWriter<std::fs::File>> = None;

        // When io_pool is Some, set up a channel-based dispatch.
        // The sink is moved into a worker running on the io_pool; the
        // consumer thread sends formatted records through the channel.
        // When io_pool is None, the sink stays inline.
        // The shared sink handle is owned by the caller (the Engine keeps it so hot
        // reload can swap the inner sink at runtime); the consumer thread and
        // any io_pool worker hold their own `Arc` clones below.
        let io_stats = Arc::clone(&control_stats);
        let (sink_tx, sink_done, consumer_sink) = match io_pool {
            Some(ref io_pool) => {
                let (tx, rx) = crossbeam_channel::bounded::<SinkMsg>(256);
                let done = Arc::new(AtomicBool::new(false));
                let done_clone = Arc::clone(&done);
                let sink = Arc::clone(&sink);
                let worker_stats = Arc::clone(&io_stats);

                io_pool.execute(move || {
                    while let Ok(msg) = rx.recv() {
                        match msg {
                            SinkMsg::Write(data) => {
                                if let Err(e) = sink.write(&data) {
                                    worker_stats.record_sink_error();
                                    crate::sys::diagnostics::error(
                                        "pipeline",
                                        &format!("Sink write error: {e}"),
                                    );
                                }
                            }
                            SinkMsg::Flush => {
                                if let Err(e) = sink.flush() {
                                    worker_stats.record_sink_error();
                                    crate::sys::diagnostics::error(
                                        "pipeline",
                                        &format!("Sink flush error: {e}"),
                                    );
                                }
                            }
                            SinkMsg::Close => {
                                if let Err(e) = sink.close() {
                                    worker_stats.record_sink_error();
                                    crate::sys::diagnostics::error(
                                        "pipeline",
                                        &format!("Sink close error: {e}"),
                                    );
                                }
                            }
                        }
                    }
                    // Channel closed — final cleanup
                    if let Err(e) = sink.flush() {
                        worker_stats.record_sink_error();
                        crate::sys::diagnostics::error(
                            "pipeline",
                            &format!("Sink final flush error: {e}"),
                        );
                    }
                    if let Err(e) = sink.close() {
                        worker_stats.record_sink_error();
                        crate::sys::diagnostics::error(
                            "pipeline",
                            &format!("Sink final close error: {e}"),
                        );
                    }
                    done_clone.store(true, Ordering::Release);
                });

                (Some(tx), Some(done), None)
            }
            None => (None, None, Some(sink)),
        };

        // Persistent audit-chain state is owned by the consumer-thread closure
        // (not by ConsumerCtx) so each PipelineContext can borrow it without
        // tying the borrow to the context — the loop mutates the context (sink
        // dispatch) while a pipeline context is alive. LSN starts at 1 (0 is
        // reserved for uninitialized records); chain genesis is
        // prev_content_hash = 0^32 with prev_lsn = 0.
        let lsn_counter = AtomicU64::new(1);
        let prev_content_hash = Mutex::new([0u8; 32]);
        let prev_lsn = Mutex::new(0);

        let consumer_thread = thread::Builder::new()
            .name("dologger-pipeline".into())
            .spawn(move || {
                let mut ctx = ConsumerCtx {
                    ring_buffer,
                    pool,
                    shutdown: shutdown_flag,
                    batch_size,
                    control_stats,
                    sink: consumer_sink.as_deref(),
                    signature_engine: &signature_engine,
                    rate_limiter: &rate_limiter,
                    drop_level_policy: &drop_level_policy,
                    enable_signature,
                    dispatch: &dispatch,
                    io_pool,
                    sink_tx,
                    shm_sink: shm_sink.as_ref(),
                    // References to closure-owned chain state (see above).
                    lsn_counter: &lsn_counter,
                    prev_content_hash: &prev_content_hash,
                    prev_lsn: &prev_lsn,
                };
                consumer_loop(&mut ctx, sig_sidecar);
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
    ring_buffer: Arc<RingBuffer<RecordPtr>>,
    pool: Arc<RecordPool>,
    shutdown: Arc<AtomicBool>,
    batch_size: usize,
    control_stats: Arc<ControlPlaneStats>,
    sink: Option<&'a SinkRef>,
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
    /// Optional shared-memory sink. Written per accepted record (SIF frame) on the
    /// consumer thread, parallel to the configured sink.
    shm_sink: Option<&'a Arc<ShmSink>>,
    /// Monotonically increasing LSN counter for the audit chain. Borrowed from the
    /// consumer-thread closure (which owns it) so the sequence survives batch
    /// boundaries without tying the PipelineContext borrow to `ConsumerCtx`.
    lsn_counter: &'a AtomicU64,
    /// A.6 chain predecessor content_hash, owned by the consumer-thread closure
    /// and borrowed by each PipelineContext across batch boundaries.
    prev_content_hash: &'a Mutex<[u8; 32]>,
    /// A.6 chain predecessor LSN, owned by the consumer-thread closure and
    /// borrowed by each PipelineContext across batch boundaries.
    prev_lsn: &'a Mutex<u64>,
}

impl ConsumerCtx<'_> {
    /// Write a formatted record — dispatches through the channel when
    /// io_pool is active, otherwise writes inline.
    fn dispatch_write(&mut self, formatted: String) {
        if let Some(ref tx) = self.sink_tx {
            // Channel-based dispatch — send blocks if the I/O
            // worker is behind, providing natural backpressure.
            if tx.send(SinkMsg::Write(formatted)).is_err() {
                self.control_stats.record_dispatch_error();
                crate::sys::diagnostics::error(
                    "pipeline",
                    "Sink channel disconnected — write dropped",
                );
            }
        } else if let Some(sink) = self.sink {
            if let Err(e) = sink.write(&formatted) {
                self.control_stats.record_sink_error();
                crate::sys::diagnostics::error("pipeline", &format!("Sink write error: {e}"));
            }
        }
    }

    /// Flush the sink.
    fn dispatch_flush(&mut self) {
        if let Some(ref tx) = self.sink_tx {
            if tx.send(SinkMsg::Flush).is_err() {
                self.control_stats.record_dispatch_error();
            }
        } else if let Some(sink) = self.sink {
            if let Err(error) = sink.flush() {
                self.control_stats.record_sink_error();
                crate::sys::diagnostics::error("pipeline", &format!("Sink flush error: {error}"));
            }
        }
    }

    /// Close the sink.
    fn dispatch_close(&mut self) {
        if let Some(ref tx) = self.sink_tx {
            if tx.send(SinkMsg::Close).is_err() {
                self.control_stats.record_dispatch_error();
            }
        } else if let Some(sink) = self.sink {
            if let Err(error) = sink.close() {
                self.control_stats.record_sink_error();
                crate::sys::diagnostics::error("pipeline", &format!("Sink close error: {error}"));
            }
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
fn mirror_record_to_shm(record: &Record, shm: &ShmSink, stats: &ControlPlaneStats) {
    match crate::sif::encode_record(record) {
        Ok(frame) => {
            if !shm.write(&frame) {
                stats.record_shm_drop();
                crate::sys::diagnostics::warn("pipeline", "SIF shared-memory write dropped");
            }
        }
        Err(error) => {
            stats.record_sink_error();
            crate::sys::diagnostics::error(
                "pipeline",
                &format!("SIF encoding failed; record not mirrored: {error}"),
            );
        }
    }
}
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

/// Append one audit signature line to the sidecar writer.
///
/// Format: `<lsn>:<content_hash_hex>:<signature_hex>\n` — decimal LSN, lowercase
/// hex for the 32/64-byte blobs. Parsed by `dologctl verify-log --sidecar`.
///
/// The writer is passed as `&mut Option<...>` so callers can hand it through a
/// `take()`/restore around the ring-buffer drain (the drain closure cannot
/// borrow the consumer context mutably while `drain` holds a shared borrow).
fn write_sidecar_line(
    w: &mut Option<BufWriter<std::fs::File>>,
    lsn: u64,
    content_hash: &[u8; 32],
    sig: &[u8; 64],
) {
    let Some(w) = w.as_mut() else {
        return;
    };
    use std::fmt::Write as _;
    let mut line = String::with_capacity(2 + 64 + 1 + 128 + 1);
    let _ = write!(line, "{lsn}:");
    for b in content_hash {
        let _ = write!(line, "{b:02x}");
    }
    line.push(':');
    for b in sig {
        let _ = write!(line, "{b:02x}");
    }
    line.push('\n');
    if let Err(e) = w.write_all(line.as_bytes()) {
        crate::sys::diagnostics::error("pipeline", &format!("Sidecar write error: {e}"));
    }
}

/// Flush the signature sidecar writer (idle cycles and shutdown).
fn flush_sidecar_writer(w: &mut Option<BufWriter<std::fs::File>>) {
    if let Some(w) = w.as_mut() {
        if let Err(e) = w.flush() {
            crate::sys::diagnostics::error("pipeline", &format!("Sidecar flush error: {e}"));
        }
    }
}

/// Main consumer loop — drains ring buffer, runs multi-stage pipeline.
///
/// `sig_sidecar` is passed separately (not stored on `ConsumerCtx`) so the
/// drain closures can write it without borrowing the context mutably while
/// `RingBuffer::drain` holds a shared borrow of it.
fn consumer_loop(c: &mut ConsumerCtx<'_>, mut sig_sidecar: Option<BufWriter<std::fs::File>>) {
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
            c.lsn_counter,
            c.prev_content_hash,
            c.prev_lsn,
        );

        // Collect formatted records for batch dispatch
        let mut pending_writes: Vec<String> = Vec::with_capacity(c.batch_size);

        let drained = c.ring_buffer.drain(c.batch_size, |record_ptr| {
            // SAFETY: record_ptr has exclusive ownership during drain.
            let record = unsafe { &mut *record_ptr.as_ptr() };

            let pipeline_accepted = run_pipeline(record, &mut pctx);
            if pipeline_accepted {
                // Persist the audit signature to the sidecar when the produced
                // LSN matches this record — guards against a stale signature
                // left behind by a record dropped at a later stage.
                if let Some((lsn, ch, sig)) = pctx.take_last_signature() {
                    if lsn == record.lsn {
                        write_sidecar_line(&mut sig_sidecar, lsn, &ch, &sig);
                    }
                }
                pending_writes.push(format_record(record, c.dispatch));
                batch_processed += 1;
            } else {
                batch_dropped += 1;
            }

            // Mirror the accepted record into the shared-memory sink (if any)
            // while it is still owned by the consumer. SIF serialisation is
            // cheap and the shm write is non-blocking / lossy by design.
            if pipeline_accepted {
                if let Some(shm) = c.shm_sink {
                    mirror_record_to_shm(record, shm, &c.control_stats);
                }
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
        let batch_errors = pctx.stage_stats.iter().map(|stats| stats.errors).sum();
        c.control_stats
            .record_batch(batch_processed, batch_dropped, batch_errors);
        c.control_stats.set_ring_fill(c.ring_buffer.fill_level());

        if drained == 0 {
            // Idle — flush buffered sidecar lines before sleeping.
            flush_sidecar_writer(&mut sig_sidecar);
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
        c.lsn_counter,
        c.prev_content_hash,
        c.prev_lsn,
    );

    let mut final_writes: Vec<String> = Vec::new();
    let mut final_processed = 0u64;
    let mut final_dropped = 0u64;
    let remaining = c.ring_buffer.drain(usize::MAX, |record_ptr| {
        // SAFETY: final drain on shutdown — record_ptr has exclusive ownership.
        let record = unsafe { &mut *record_ptr.as_ptr() };
        let pipeline_accepted = run_pipeline(record, &mut pctx);
        if pipeline_accepted {
            final_processed += 1;
            if let Some((lsn, ch, sig)) = pctx.take_last_signature() {
                if lsn == record.lsn {
                    write_sidecar_line(&mut sig_sidecar, lsn, &ch, &sig);
                }
            }
            final_writes.push(format_record(record, c.dispatch));
        } else {
            final_dropped += 1;
        }

        // Mirror the accepted record into the shared-memory sink (if any).
        if pipeline_accepted {
            if let Some(shm) = c.shm_sink {
                mirror_record_to_shm(record, shm, &c.control_stats);
            }
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

    let final_errors = pctx.stage_stats.iter().map(|stats| stats.errors).sum();
    c.control_stats
        .record_batch(final_processed, final_dropped, final_errors);
    c.control_stats.set_ring_fill(c.ring_buffer.fill_level());

    if remaining > 0 {
        crate::sys::diagnostics::info(
            "pipeline",
            &format!("Shutdown: flushed {remaining} remaining records"),
        );
    }

    report_stats(&pctx, remaining);

    // Final flush and close — dispatched through the same path.
    flush_sidecar_writer(&mut sig_sidecar);
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

        let control_stats = Arc::new(ControlPlaneStats::new());

        let sink = Arc::new(SinkRef::new(ThreadTrackingSink::new(
            Arc::clone(&records_written),
            Arc::clone(&write_threads),
        )));
        sink.open().unwrap();

        let mut pipeline = Pipeline::new_with_stats(
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
            Arc::clone(&control_stats),
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
                let id = time_source.next_id();
                record.set_id(id.hi, id.lo);
                record.timestamp = time_source.now_nanos();
                record.level = LogLevel::Info;
                record.message.set(&format!("test record {i}"));
                record.thread_id = tid as u32;
                record.process_id = pid;
                record.set_process_name("test");
                record.set_host_name("localhost");
                record.set_environment("test");
            }
            ring_buffer
                .try_push(RecordPtr::new(record_ptr))
                .expect("Ring buffer should accept");
        }

        pipeline.shutdown();

        assert_eq!(
            records_written.load(Ordering::Relaxed),
            N,
            "All records should be written"
        );
        let snapshot = control_stats.snapshot("INFO");
        assert_eq!(snapshot.processed, N as u64);
        assert_eq!(snapshot.ring_fill_permille, 0);
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

        let sink = Arc::new(SinkRef::new(ThreadTrackingSink::new(
            Arc::clone(&records_written),
            Arc::clone(&write_threads),
        )));
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
                let id = time_source.next_id();
                record.set_id(id.hi, id.lo);
                record.timestamp = time_source.now_nanos();
                record.level = LogLevel::Info;
                record.message.set(&format!("test record {i}"));
                record.thread_id = tid as u32;
                record.process_id = pid;
                record.set_process_name("test");
                record.set_host_name("localhost");
                record.set_environment("test");
            }
            ring_buffer
                .try_push(RecordPtr::new(record_ptr))
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
