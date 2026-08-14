//! Self-monitoring channel (sysmon).
//!
//! Dedicated lock-free ring buffer for operational events.
//! A low-priority independent thread flushes events to stderr as JSON.
//!
//! # Design
//! - Ring buffer: 4096 events, non-blocking
//! - Flush thread: lowest priority, every 1 second
//! - Output format: fixed JSON (one line per event)
//! - NOT affected by user-configured plugins or formatters

use crate::sys::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Maximum number of events in the sysmon ring buffer.
const SYSMON_CAPACITY: usize = 4096;

/// A single sysmon event.
#[derive(Debug, Clone)]
pub struct SysmonEvent {
    /// Error code (0 = informational)
    pub error_code: i32,
    /// Event category (e.g. "pipeline", "plugin", "config", "audit")
    pub category: String,
    /// Human-readable description
    pub description: String,
    /// Monotonic timestamp (milliseconds since sysmon init)
    pub timestamp_ms: u64,
    /// Severity: 0=DEBUG, 1=INFO, 2=WARN, 3=ERROR, 4=CRITICAL, 5=EMERGENCY
    pub severity: u8,
}

impl SysmonEvent {
    /// Create a new sysmon event.
    pub fn new(category: &str, severity: u8, description: &str) -> Self {
        Self {
            error_code: 0,
            category: category.to_string(),
            description: description.to_string(),
            timestamp_ms: 0, // filled by the ring buffer on push
            severity,
        }
    }
}

/// A single slot in the sysmon ring buffer.
struct SysmonSlot {
    event: Mutex<Option<SysmonEvent>>,
    sequence: AtomicU64,
}

/// Lock-free MPSC ring buffer for sysmon events.
struct SysmonRing {
    slots: Box<[SysmonSlot]>,
    mask: u64,
    producer_seq: AtomicU64,
    consumer_seq: AtomicU64,
}

impl SysmonRing {
    fn new(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two());
        let mut slots = Vec::with_capacity(capacity);
        for i in 0..capacity {
            slots.push(SysmonSlot {
                event: Mutex::new(None),
                sequence: AtomicU64::new(i as u64),
            });
        }
        Self {
            slots: slots.into_boxed_slice(),
            mask: (capacity - 1) as u64,
            producer_seq: AtomicU64::new(0),
            consumer_seq: AtomicU64::new(0),
        }
    }

    /// Try to push an event. Non-blocking — drops if full.
    fn push(&self, mut event: SysmonEvent) {
        let producer = self.producer_seq.load(Ordering::Acquire);
        let consumer = self.consumer_seq.load(Ordering::Acquire);

        if producer - consumer >= SYSMON_CAPACITY as u64 {
            // Buffer full — drop the event
            return;
        }

        // Timestamp in milliseconds since UNIX epoch (not producer sequence)
        event.timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let index = (producer & self.mask) as usize;
        let slot = &self.slots[index];

        // Wait for slot availability
        while slot.sequence.load(Ordering::Acquire) != producer {
            std::hint::spin_loop();
        }

        // Write event
        if let Ok(mut guard) = slot.event.lock() {
            *guard = Some(event);
        }

        // Publish
        slot.sequence.store(producer + 1, Ordering::Release);
        self.producer_seq.store(producer + 1, Ordering::Release);
    }

    /// Drain up to `batch` events into the callback.
    fn drain<F: FnMut(SysmonEvent)>(&self, batch: usize, mut f: F) -> usize {
        let consumer = self.consumer_seq.load(Ordering::Acquire);
        let producer = self.producer_seq.load(Ordering::Acquire);
        let available = (producer - consumer) as usize;
        let to_drain = batch.min(available);

        for i in 0..to_drain {
            let seq = consumer + i as u64;
            let index = (seq & self.mask) as usize;
            let slot = &self.slots[index];

            // Wait for publication
            while slot.sequence.load(Ordering::Acquire) != seq + 1 {
                std::hint::spin_loop();
            }

            // Take event
            let event = slot.event.lock().ok().and_then(|mut g| g.take());
            if let Some(e) = event {
                f(e);
            }

            // Release slot
            slot.sequence
                .store(seq + SYSMON_CAPACITY as u64, Ordering::Release);
        }

        self.consumer_seq
            .store(consumer + to_drain as u64, Ordering::Release);
        to_drain
    }
}

/// Sysmon channel — operational monitoring for DoLogger.
pub struct Sysmon {
    ring: Arc<SysmonRing>,
    shutdown: Arc<AtomicBool>,
    _flush_thread: Option<JoinHandle<()>>,
}

impl Sysmon {
    /// Create and start the sysmon channel.
    pub fn start() -> Self {
        let ring = Arc::new(SysmonRing::new(SYSMON_CAPACITY));
        let shutdown = Arc::new(AtomicBool::new(false));

        let ring_clone = Arc::clone(&ring);
        let shutdown_clone = Arc::clone(&shutdown);

        let flush_thread = thread::Builder::new()
            .name("dologger-sysmon".into())
            .spawn(move || {
                sysmon_flush_loop(ring_clone, shutdown_clone);
            })
            .ok();

        Self {
            ring,
            shutdown,
            _flush_thread: flush_thread,
        }
    }

    /// Push an event to the sysmon channel. Non-blocking.
    pub fn event(&self, category: &str, severity: u8, description: &str) {
        self.ring
            .push(SysmonEvent::new(category, severity, description));
    }

    /// Convenience: INFO event.
    pub fn info(&self, category: &str, description: &str) {
        self.event(category, 1, description);
    }

    /// Convenience: WARN event.
    pub fn warn(&self, category: &str, description: &str) {
        self.event(category, 2, description);
    }

    /// Convenience: ERROR event.
    pub fn error(&self, category: &str, description: &str) {
        self.event(category, 3, description);
    }

    /// Convenience: CRITICAL event.
    pub fn critical(&self, category: &str, description: &str) {
        self.event(category, 4, description);
    }

    /// Shutdown the sysmon channel gracefully.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self._flush_thread.take() {
            let _ = handle.join();
        }
        // Final drain
        self.ring.drain(SYSMON_CAPACITY, |event| {
            emit_event(&event);
        });
    }
}

/// Background flush loop.
fn sysmon_flush_loop(ring: Arc<SysmonRing>, shutdown: Arc<AtomicBool>) {
    let interval = Duration::from_secs(1);

    while !shutdown.load(Ordering::Acquire) {
        ring.drain(256, |event| {
            emit_event(&event);
        });
        thread::sleep(interval);
    }
}

/// Emit a single sysmon event as a JSON line to stderr.
fn emit_event(event: &SysmonEvent) {
    // Fixed JSON format
    let json = format!(
        r#"{{"sysmon_version":"1.0","error_code":{},"category":"{}","description":"{}","timestamp_ms":{},"severity":{}}}"#,
        event.error_code,
        event.category,
        event.description.replace('"', "\\\""),
        event.timestamp_ms,
        event.severity
    );
    io::stderr_line(&json);
}
