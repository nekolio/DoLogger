//! Isolated runtime pipeline for AUDIT-level records.
//!
//! AUDIT records are admitted through this pipeline only when the caller
//! explicitly enables auditing. The normal ring, rate limiter, drop policy,
//! and user plugins never see them.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::buffer::RecordPool;
use crate::buffer::RingBuffer;
use crate::record::{LogLevel, Record, RECORD_FLAG_AUDIT, RECORD_FLAG_SIGNED};
use crate::security::ExternalAnchor;
use crate::security::SignatureEngine;
use crate::sink::SecuritySink;
use crate::sink::Sink;
use crate::sink::WormSink;
use sha2::{Digest, Sha256};

/// Default ratio of ring buffer capacity reserved for AUDIT records.
pub const DEFAULT_AUDIT_BUFFER_RATIO: f64 = 0.10;

/// Minimum number of slots reserved for AUDIT.
const MIN_AUDIT_SLOTS: usize = 1024;

/// Dedicated AUDIT pipeline with non-droppable admission and dual persistence.
///
/// Signing is optional at this layer. When enabled, the pipeline owns a
/// durable signature sidecar; otherwise the WORM envelope carries the audit
/// hash chain without a signature field.
pub struct AuditPipeline {
    ring_buffer: Arc<RingBuffer<*mut Record>>,
    consumer_thread: Option<thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    failure_message: Arc<Mutex<Option<String>>>,
}

impl AuditPipeline {
    /// Create the isolated AUDIT pipeline and open every required persistence target.
    #[expect(
        clippy::too_many_arguments,
        reason = "Audit construction wires independent ownership and persistence resources"
    )]
    pub fn new(
        total_capacity: usize,
        audit_ratio: f64,
        pool: Arc<RecordPool>,
        mut worm_sink: WormSink,
        mut security_sink: SecuritySink,
        signature_engine: Arc<SignatureEngine>,
        external_anchor: Option<Arc<Mutex<ExternalAnchor>>>,
        sidecar_path: Option<&Path>,
        signature_enabled: bool,
    ) -> Result<Self, String> {
        let audit_capacity = ((total_capacity as f64 * audit_ratio) as usize)
            .max(MIN_AUDIT_SLOTS)
            .next_power_of_two();

        let sidecar = if signature_enabled {
            Some(open_sidecar(sidecar_path.ok_or_else(|| {
                "signed audit mode requires sig_sidecar_path".to_string()
            })?)?)
        } else {
            None
        };
        worm_sink
            .open()
            .map_err(|error| format!("Failed to open audit WORM sink: {error}"))?;
        security_sink
            .open()
            .map_err(|error| format!("Failed to open audit security sink: {error}"))?;

        let ring_buffer = Arc::new(RingBuffer::new(audit_capacity));
        let shutdown = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let failure_message = Arc::new(Mutex::new(None));

        let consumer_thread = thread::Builder::new()
            .name("dologger-audit-pipeline".into())
            .spawn({
                let ring_buffer = Arc::clone(&ring_buffer);
                let shutdown = Arc::clone(&shutdown);
                let failed = Arc::clone(&failed);
                let failure_message = Arc::clone(&failure_message);
                let pool = Arc::clone(&pool);
                let signature_engine = Arc::clone(&signature_engine);
                move || {
                    audit_consumer_loop(
                        ring_buffer,
                        pool,
                        shutdown,
                        failed,
                        failure_message,
                        worm_sink,
                        security_sink,
                        sidecar,
                        signature_engine,
                        external_anchor,
                        signature_enabled,
                    );
                }
            })
            .map_err(|error| format!("Failed to spawn audit consumer: {error}"))?;

        crate::sys::diagnostics::info(
            "audit_pipeline",
            &format!("Audit pipeline started: capacity={audit_capacity}, ratio={audit_ratio:.1}"),
        );

        Ok(Self {
            ring_buffer,
            consumer_thread: Some(consumer_thread),
            shutdown,
            failed,
            failure_message,
        })
    }

    /// Admit an AUDIT record, blocking until the dedicated queue accepts it.
    pub fn push_audit(&self, record_ptr: *mut Record) -> Result<(), *mut Record> {
        if self.shutdown.load(Ordering::Acquire) || self.failed.load(Ordering::Acquire) {
            return Err(record_ptr);
        }

        let mut spins = 0u32;
        loop {
            match self.ring_buffer.try_push(record_ptr) {
                Ok(()) => return Ok(()),
                Err(ptr) => {
                    if self.shutdown.load(Ordering::Acquire) || self.failed.load(Ordering::Acquire)
                    {
                        return Err(ptr);
                    }
                    std::hint::spin_loop();
                    spins = spins.saturating_add(1);
                    if spins.is_multiple_of(64) {
                        thread::yield_now();
                    }
                }
            }
        }
    }

    /// Return the latest fatal persistence error, if the audit path failed.
    pub fn failure_message(&self) -> Option<String> {
        self.failure_message
            .lock()
            .ok()
            .and_then(|message| message.clone())
    }

    /// Drain accepted records and stop the isolated consumer.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.consumer_thread.take() {
            let _ = handle.join();
        }
        if let Some(message) = self.failure_message() {
            crate::sys::diagnostics::error(
                "audit_pipeline",
                &format!("Audit pipeline stopped after fatal persistence error: {message}"),
            );
        } else {
            crate::sys::diagnostics::info("audit_pipeline", "Audit pipeline shutdown complete");
        }
    }
}

fn open_sidecar(path: &Path) -> Result<BufWriter<File>, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create signature sidecar directory: {error}")
            })?;
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Failed to open signature sidecar {}: {error}",
                path.display()
            )
        })?;
    Ok(BufWriter::new(file))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Consumer loop owns each isolated runtime resource explicitly"
)]
fn audit_consumer_loop(
    ring_buffer: Arc<RingBuffer<*mut Record>>,
    pool: Arc<RecordPool>,
    shutdown: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    failure_message: Arc<Mutex<Option<String>>>,
    mut worm_sink: WormSink,
    mut security_sink: SecuritySink,
    mut sidecar: Option<BufWriter<File>>,
    signature_engine: Arc<SignatureEngine>,
    external_anchor: Option<Arc<Mutex<ExternalAnchor>>>,
    signature_enabled: bool,
) {
    let empty_sleep = Duration::from_micros(50);
    let mut prev_content_hash = [0u8; 32];
    let mut prev_lsn = 0u64;

    while !shutdown.load(Ordering::Acquire) {
        let drained = ring_buffer.drain(128, |record_ptr| {
            if failed.load(Ordering::Acquire) {
                free_record(&pool, record_ptr);
                return;
            }

            let result = {
                // SAFETY: the ring CAS grants this consumer exclusive ownership.
                let record = unsafe { &mut *record_ptr };
                process_audit_record(
                    record,
                    &mut worm_sink,
                    &mut security_sink,
                    &mut sidecar,
                    &signature_engine,
                    &mut prev_content_hash,
                    &mut prev_lsn,
                    external_anchor.as_ref(),
                    signature_enabled,
                )
            };
            free_record(&pool, record_ptr);

            if let Err(error) = result {
                record_failure(&failed, &failure_message, &shutdown, error);
            }
        });

        if drained == 0 {
            thread::sleep(empty_sleep);
        }
    }

    let remaining = ring_buffer.drain(usize::MAX, |record_ptr| {
        if !failed.load(Ordering::Acquire) {
            let result = {
                // SAFETY: shutdown drain has exclusive ownership of the slot.
                let record = unsafe { &mut *record_ptr };
                process_audit_record(
                    record,
                    &mut worm_sink,
                    &mut security_sink,
                    &mut sidecar,
                    &signature_engine,
                    &mut prev_content_hash,
                    &mut prev_lsn,
                    external_anchor.as_ref(),
                    signature_enabled,
                )
            };
            if let Err(error) = result {
                record_failure(&failed, &failure_message, &shutdown, error);
            }
        }
        free_record(&pool, record_ptr);
    });

    if remaining > 0 {
        crate::sys::diagnostics::info(
            "audit_pipeline",
            &format!("Shutdown: processed {remaining} remaining AUDIT records"),
        );
    }

    if let Some(sidecar) = sidecar.as_mut() {
        if let Err(error) = sidecar.flush() {
            record_failure(
                &failed,
                &failure_message,
                &shutdown,
                format!("sidecar flush: {error}"),
            );
        }
    }
    if let Err(error) = worm_sink.flush() {
        record_failure(
            &failed,
            &failure_message,
            &shutdown,
            format!("WORM flush: {error}"),
        );
    }
    if let Err(error) = security_sink.flush() {
        record_failure(
            &failed,
            &failure_message,
            &shutdown,
            format!("Security flush: {error}"),
        );
    }
    if let Err(error) = worm_sink.close() {
        record_failure(
            &failed,
            &failure_message,
            &shutdown,
            format!("WORM close: {error}"),
        );
    }
    if let Err(error) = security_sink.close() {
        record_failure(
            &failed,
            &failure_message,
            &shutdown,
            format!("Security close: {error}"),
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Record processing keeps chain and dual-write state explicit"
)]
fn process_audit_record(
    record: &mut Record,
    worm_sink: &mut WormSink,
    security_sink: &mut SecuritySink,
    sidecar: &mut Option<BufWriter<File>>,
    signature_engine: &SignatureEngine,
    prev_content_hash: &mut [u8; 32],
    prev_lsn: &mut u64,
    external_anchor: Option<&Arc<Mutex<ExternalAnchor>>>,
    signature_enabled: bool,
) -> Result<(), String> {
    if record.level != LogLevel::Audit {
        return Err("non-AUDIT record reached the isolated audit pipeline".into());
    }

    record.flags |= RECORD_FLAG_AUDIT;
    if signature_enabled {
        record.flags |= RECORD_FLAG_SIGNED;
    } else {
        record.flags &= !RECORD_FLAG_SIGNED;
    }
    record.lsn = prev_lsn
        .checked_add(1)
        .ok_or_else(|| "AUDIT LSN exhausted".to_string())?;
    record.compute_content_hash();
    let mut hasher = Sha256::new();
    hasher.update(*prev_content_hash);
    hasher.update(prev_lsn.to_le_bytes());
    let prev_hash: [u8; 32] = hasher.finalize().into();
    let signature = signature_enabled.then(|| signature_engine.sign_record(record, &prev_hash));
    let formatted = crate::sink::SinkRef::format_record(record);
    let envelope = canonical_audit_envelope(record, signature.as_ref(), &formatted);

    worm_sink
        .write_worm_record_with_hash(
            record.lsn,
            &prev_hash,
            &record.content_hash,
            envelope.as_bytes(),
        )
        .map_err(|error| format!("WORM write: {error}"))?;
    security_sink
        .write(&formatted)
        .map_err(|error| format!("Security write: {error}"))?;
    if let (Some(sidecar), Some(signature)) = (sidecar.as_mut(), signature.as_ref()) {
        write_sidecar_line(sidecar, record.lsn, &record.content_hash, signature)
            .map_err(|error| format!("sidecar write: {error}"))?;
        sidecar
            .flush()
            .map_err(|error| format!("sidecar flush: {error}"))?;
    }

    *prev_content_hash = record.content_hash;
    *prev_lsn = record.lsn;
    if let Some(anchor) = external_anchor {
        let chain_hash = signature.map_or_else(
            || Sha256::digest(envelope.as_bytes()).into(),
            |signature| SignatureEngine::record_chain_hash(record.lsn, &signature),
        );
        if let Ok(mut guard) = anchor.lock() {
            guard.accumulate_hash(&chain_hash);
            let _ = guard.maybe_anchor(signature_engine);
        }
    }
    Ok(())
}

fn canonical_audit_envelope(
    record: &Record,
    signature: Option<&[u8; 64]>,
    formatted: &str,
) -> String {
    let mut content_hash = String::with_capacity(64);
    for byte in record.content_hash {
        content_hash.push_str(&format!("{byte:02x}"));
    }
    let signature_hex = signature.map_or_else(
        || "none".to_string(),
        |signature| {
            let mut encoded = String::with_capacity(128);
            for byte in signature {
                encoded.push_str(&format!("{byte:02x}"));
            }
            encoded
        },
    );
    format!(
        "lsn={};content_hash={};signature={};record={}\n",
        record.lsn, content_hash, signature_hex, formatted
    )
}

fn write_sidecar_line(
    writer: &mut BufWriter<File>,
    lsn: u64,
    content_hash: &[u8; 32],
    signature: &[u8; 64],
) -> std::io::Result<()> {
    write!(writer, "{lsn}:")?;
    for byte in content_hash {
        write!(writer, "{byte:02x}")?;
    }
    writer.write_all(b":")?;
    for byte in signature {
        write!(writer, "{byte:02x}")?;
    }
    writer.write_all(b"\n")
}

fn record_failure(
    failed: &AtomicBool,
    failure_message: &Mutex<Option<String>>,
    shutdown: &AtomicBool,
    message: String,
) {
    if !failed.swap(true, Ordering::AcqRel) {
        if let Ok(mut slot) = failure_message.lock() {
            *slot = Some(message.clone());
        }
        crate::sys::diagnostics::error(
            "audit_pipeline",
            &format!("Fatal AUDIT persistence error: {message}"),
        );
    }
    shutdown.store(true, Ordering::Release);
}

fn free_record(pool: &RecordPool, record_ptr: *mut Record) {
    // SAFETY: each pointer is claimed once by the ring consumer and returned to
    // the same object pool after all persistence attempts finish.
    unsafe { pool.free(&*record_ptr) }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::{SecuritySinkConfig, WormSinkConfig};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_path(prefix: &str, extension: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}.{extension}", nonce))
    }

    #[test]
    fn unsigned_audit_record_gets_chain_fields_without_sidecar() {
        let worm_path = test_path("dologger-audit-worm", "log");
        let security_path = test_path("dologger-audit-security", "log");
        let mut worm = WormSink::new(WormSinkConfig {
            path: worm_path.clone(),
            ..Default::default()
        });
        let mut security = SecuritySink::new(SecuritySinkConfig {
            path: security_path.clone(),
            ..Default::default()
        });
        worm.open().expect("open test WORM sink");
        security.open().expect("open test Security sink");

        let mut record = Record::new(0);
        record.level = LogLevel::Audit;
        record.timestamp = 1;
        record.message.set("unsigned audit");
        let mut previous_hash = [0u8; 32];
        let mut previous_lsn = 0;
        let signature_engine = SignatureEngine::new();
        let mut sidecar = None;

        process_audit_record(
            &mut record,
            &mut worm,
            &mut security,
            &mut sidecar,
            &signature_engine,
            &mut previous_hash,
            &mut previous_lsn,
            None,
            false,
        )
        .expect("unsigned audit record should persist");

        assert_eq!(record.lsn, 1);
        assert_ne!(record.content_hash, [0u8; 32]);
        assert_ne!(record.flags & RECORD_FLAG_AUDIT, 0);
        assert_eq!(record.flags & RECORD_FLAG_SIGNED, 0);

        worm.flush().expect("flush test WORM sink");
        security.flush().expect("flush test Security sink");
        worm.close().expect("close test WORM sink");
        security.close().expect("close test Security sink");
        let _ = std::fs::remove_file(worm_path);
        let _ = std::fs::remove_file(security_path);
    }
}
