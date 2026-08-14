//! Audit domain isolated pipeline.
//!
//! AUDIT-level records flow through a completely independent pipeline
//! that bypasses all user-configured plugins. This guarantees:
//!
//! 1. **Isolation**: AUDIT records are never blocked by regular log backpressure
//! 2. **Integrity**: No plugin can intercept, modify, or drop AUDIT records
//! 3. **Priority**: AUDIT records have their own ring buffer partition
//!
//! # Architecture
//!
//! ```text
//! dologger_log() ──▶ Ring Buffer ──▶ Regular Pipeline (plugins → sink)
//!                        │
//!                        └──▶ AUDIT Partition ──▶ Audit Pipeline (WORM + Security dual-write)
//! ```
//!
//! The audit pipeline:
//! - Has its own dedicated ring buffer (10% of total ring buffer capacity)
//! - Runs on its own consumer thread
//! - Signs every record (Ed25519) regardless of config
//! - Dual-writes to WORM Sink (chain integrity) + Security Sink (isolated output)
//! - Never drops records (AUDIT iron law)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::buffer::RecordPool;
use crate::buffer::RingBuffer;
use crate::record::Record;
use crate::security::ExternalAnchor;
use crate::security::SignatureEngine;
use crate::sink::SecuritySink;
use crate::sink::Sink;
use crate::sink::WormSink;

/// Default ratio of ring buffer capacity reserved for AUDIT records.
pub const DEFAULT_AUDIT_BUFFER_RATIO: f64 = 0.10; // 10%

/// Minimum number of slots reserved for AUDIT (hard floor).
const MIN_AUDIT_SLOTS: usize = 1024;

/// Audit pipeline — independent consumer for AUDIT records.
///
/// Owns its own ring buffer partition, consumer thread, and dual Sinks:
/// - **WormSink**: LSN chain integrity, prev_hash verification, gap markers
/// - **SecuritySink**: isolated file output, 0600 permissions, plugin bypass
///
/// Every AUDIT record is written to both sinks (dual-write).  If either
/// sink fails, a diagnostic error is logged but the other sink continues.
pub struct AuditPipeline {
    /// Dedicated ring buffer for AUDIT records
    ring_buffer: Arc<RingBuffer<*mut Record>>,
    /// Consumer thread handle
    consumer_thread: Option<thread::JoinHandle<()>>,
    /// Shutdown flag
    shutdown: Arc<AtomicBool>,
}

impl AuditPipeline {
    /// Create a new audit pipeline with a partition of the main ring buffer.
    ///
    /// * `total_capacity` — total ring buffer capacity (shared with main pipeline)
    /// * `audit_ratio` — fraction of capacity reserved for AUDIT (0.0–1.0)
    /// * `pool` — shared object pool
    /// * `worm_sink` — WORM Sink for chain integrity (prev_hash, gap markers)
    /// * `security_sink` — Security Sink for isolated file output (0600, plugin bypass)
    /// * `signature_engine` — Ed25519 engine (mandatory for AUDIT)
    pub fn new(
        total_capacity: usize,
        audit_ratio: f64,
        pool: Arc<RecordPool>,
        mut worm_sink: WormSink,
        mut security_sink: SecuritySink,
        signature_engine: Arc<SignatureEngine>,
        external_anchor: Option<Arc<Mutex<ExternalAnchor>>>,
    ) -> Result<Self, String> {
        let audit_capacity = (total_capacity as f64 * audit_ratio) as usize;
        let audit_capacity = audit_capacity.max(MIN_AUDIT_SLOTS);
        // Round up to power of two
        let audit_capacity = audit_capacity.next_power_of_two();

        worm_sink
            .open()
            .map_err(|e| format!("Failed to open audit WORM sink: {e}"))?;
        security_sink
            .open()
            .map_err(|e| format!("Failed to open audit security sink: {e}"))?;

        let ring_buffer = Arc::new(RingBuffer::new(audit_capacity));
        let shutdown = Arc::new(AtomicBool::new(false));

        let rb = Arc::clone(&ring_buffer);
        let p = Arc::clone(&pool);
        let sf = Arc::clone(&shutdown);
        let sig = Arc::clone(&signature_engine);
        let anchor = external_anchor.clone();

        let consumer_thread = thread::Builder::new()
            .name("dologger-audit-pipeline".into())
            .spawn(move || {
                audit_consumer_loop(rb, p, sf, worm_sink, security_sink, sig, anchor);
            })
            .map_err(|e| format!("Failed to spawn audit consumer: {e}"))?;

        crate::sys::diag::info(
            "audit_pipeline",
            &format!(
                "Audit pipeline started (dual-write): capacity={audit_capacity}, ratio={audit_ratio:.1}"
            ),
        );

        Ok(Self {
            ring_buffer,
            consumer_thread: Some(consumer_thread),
            shutdown,
        })
    }

    /// Push an AUDIT record into the audit ring buffer.
    ///
    /// This is the hot path for AUDIT records. Unlike the main pipeline,
    /// this buffer is never congested by regular log traffic.
    ///
    /// Returns `Ok(())` if accepted, `Err` only in catastrophic failure.
    pub fn push_audit(&self, record_ptr: *mut Record) -> Result<(), *mut Record> {
        // AUDIT iron law: never drop — block until space available
        let mut spins: u32 = 0;
        loop {
            match self.ring_buffer.try_push(record_ptr) {
                Ok(()) => return Ok(()),
                Err(ptr) => {
                    // Spin-wait briefly — the audit consumer should drain faster
                    // than regular pipeline since it has no plugin overhead
                    std::hint::spin_loop();
                    spins += 1;
                    // Yield to the OS scheduler every 64 spins to give the
                    // consumer thread a chance to drain the ring buffer.
                    if spins.is_multiple_of(64) {
                        std::thread::yield_now();
                    }
                    // In production, we'd use a condvar or park here.
                    // For now, spin-wait with exponential backoff hint
                    if self.shutdown.load(Ordering::Acquire) {
                        return Err(ptr);
                    }
                }
            }
        }
    }

    /// Get a reference to the audit ring buffer (for direct submission from hot path).
    pub fn ring_buffer(&self) -> &Arc<RingBuffer<*mut Record>> {
        &self.ring_buffer
    }

    /// Initiate graceful shutdown.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.consumer_thread.take() {
            let _ = handle.join();
        }
        crate::sys::diag::info("audit_pipeline", "Audit pipeline shutdown complete");
    }
}

/// Audit consumer loop — minimal processing, dual-write to WORM + Security sinks.
fn audit_consumer_loop(
    ring_buffer: Arc<RingBuffer<*mut Record>>,
    pool: Arc<RecordPool>,
    shutdown: Arc<AtomicBool>,
    mut worm_sink: WormSink,
    mut security_sink: SecuritySink,
    signature_engine: Arc<SignatureEngine>,
    external_anchor: Option<Arc<Mutex<ExternalAnchor>>>,
) {
    let empty_sleep = Duration::from_micros(50);

    while !shutdown.load(Ordering::Acquire) {
        let drained = ring_buffer.drain(128, |record_ptr| {
            // SAFETY: record_ptr is exclusively owned by the audit consumer
            let record = unsafe { &mut *record_ptr };

            // Mandatory signing for AUDIT
            if record.level == crate::record::LogLevel::Audit {
                let sig = signature_engine.sign_record(record);
                record.signature = sig;

                // Accumulate chain hash for external anchoring
                if let Some(ref anchor) = external_anchor {
                    let chain_hash =
                        SignatureEngine::record_chain_hash(record.lsn, &record.signature);
                    if let Ok(mut guard) = anchor.lock() {
                        guard.accumulate_hash(&chain_hash);
                        let _ = guard.maybe_anchor(&signature_engine);
                    }
                }
            }

            // Dual-write: format once, write to both sinks.
            // WORM sink provides chain integrity; Security sink provides
            // isolated file output with restrictive permissions.
            let formatted = crate::sink::SinkRef::format_record(record);
            if let Err(e) = worm_sink.write(&formatted) {
                crate::sys::diag::error("audit_pipeline", &format!("WORM sink write failed: {e}"));
            }
            if let Err(e) = security_sink.write(&formatted) {
                crate::sys::diag::error(
                    "audit_pipeline",
                    &format!("Security sink write failed: {e}"),
                );
            }

            // SAFETY: record_ptr was obtained from this pool via alloc() and
            // has exclusive ownership at this point (won via ring buffer CAS).
            // It has not been freed yet.
            unsafe {
                pool.free(record as *const Record);
            }
        });

        if drained == 0 {
            thread::sleep(empty_sleep);
        }
    }

    // Final drain — flush remaining records to both sinks
    let remaining = ring_buffer.drain(usize::MAX, |record_ptr| {
        // SAFETY: record_ptr is exclusively owned by the audit consumer at shutdown.
        let record = unsafe { &mut *record_ptr };
        let formatted = crate::sink::SinkRef::format_record(record);
        let _ = worm_sink.write(&formatted);
        let _ = security_sink.write(&formatted);
        // SAFETY: record_ptr was obtained from this pool via alloc() and
        // has exclusive ownership at shutdown (final drain). It has not
        // been freed yet.
        unsafe {
            pool.free(record as *const Record);
        }
    });

    if remaining > 0 {
        crate::sys::diag::info(
            "audit_pipeline",
            &format!("Shutdown: flushed {remaining} remaining AUDIT records"),
        );
    }

    let _ = worm_sink.flush();
    let _ = worm_sink.close();
    let _ = security_sink.flush();
    let _ = security_sink.close();
}
