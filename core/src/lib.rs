//! DoLogger Core Engine (`libdologger_core`)
//!
//! A cross-platform, high-security, plugin-based logging engine
//! that exports a stable C ABI for host applications.

#![warn(
    missing_docs,
    rust_2018_idioms,
    unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks
)]

pub mod audit;
pub mod buffer;
pub mod config;
pub mod error;
pub mod ffi;
pub mod pipeline;
pub mod plugin;
pub mod policy;
pub mod record;
pub mod security;
pub mod sif;
pub mod sink;
pub mod sys;

use std::sync::Arc;
use std::sync::Mutex;

use crate::audit::AuditPipeline;
use crate::buffer::RecordPool;
use crate::buffer::RingBuffer;
use crate::config::DologgerConfig;
use crate::ffi::DologgerHandle;
use crate::pipeline::Pipeline;
use crate::plugin::PluginManager;
use crate::policy::{DropLevelPolicy, RateLimiter};
use crate::security::ExternalAnchor;
use crate::security::SignatureEngine;
use crate::sink::ConsoleSink;
use crate::sink::SecuritySink;
use crate::sink::SinkRef;
use crate::sink::WormSink;
use crate::sys::Sysmon;
use crate::sys::TimeSource;

// Re-exports
pub use error::DologgerError;
pub use record::{LogLevel, Record};

// Backward-compatible re-exports — modules were restructured into
// subdirectories but old paths are preserved for internal callers.
pub use buffer::emergency_buffer;
pub use buffer::object_pool;
pub use buffer::ring_buffer;
pub use config::domain;
pub use config::hot_reload;
pub use config::watcher as config_watcher;
pub use pipeline::backpressure;
pub use pipeline::canary;
pub use pipeline::circuit_breaker;
pub use plugin::dependency;
pub use plugin::phase;
pub use plugin::quota;
pub use plugin::sandbox;
pub use security::crc32c;
pub use security::external_anchor;
pub use security::key_provider;
pub use security::key_rotation;
pub use security::secret_detector;
pub use security::signature;
pub use sink::callback as sink_callback;
pub use sink::file as sink_file;
#[cfg(feature = "sink-kafka")]
pub use sink::kafka as sink_kafka;
#[cfg(feature = "sink-otel")]
pub use sink::open_telemetry as sink_otel;
pub use sink::security as sink_security;
pub use sink::shm as sink_shm;
#[cfg(feature = "sink-sqlite")]
pub use sink::sqlite as sink_sqlite;
pub use sink::syslog as sink_syslog;
#[cfg(feature = "sink-webhook")]
pub use sink::webhook as sink_webhook;
pub use sink::worm as sink_worm;
pub use sys::control_plane;
pub use sys::diagnostics;
pub use sys::diagnostics as diag;
pub use sys::host_info;
pub use sys::internal_log;
pub use sys::io;
pub use sys::system_monitor;
pub use sys::system_monitor as sysmon;
pub use sys::thread_pool;
pub use sys::time;

// ===========================================================================
// Cooperative Helping
// ===========================================================================

/// Cooperative helping state for producer threads.
///
/// When the ring buffer reaches ≥90% fill and cooperative helping is
/// enabled, producer threads temporarily act as mini-consumers: they
/// drain a small batch of records inline, format and write each record
/// to a dedicated helping sink, then retry their own push.
///
/// This prevents indefinite blocking of the calling application thread
/// while maintaining low latency under backpressure.
pub struct CooperativeHelping {
    ring_buffer: Arc<RingBuffer<*mut Record>>,
    pool: Arc<RecordPool>,
    /// Dedicated sink for cooperative helping writes.
    /// Mutex-protected so it can be called from shared references.
    /// Contention is zero in practice — only the helping path locks it.
    sink: Mutex<SinkRef>,
    enabled: bool,
}

impl CooperativeHelping {
    /// Create a new cooperative helping context.
    fn new(
        ring_buffer: Arc<RingBuffer<*mut Record>>,
        pool: Arc<RecordPool>,
        sink: SinkRef,
        enabled: bool,
    ) -> Self {
        Self {
            ring_buffer,
            pool,
            sink: Mutex::new(sink),
            enabled,
        }
    }

    /// Attempt to help drain the ring buffer when backpressure is high.
    ///
    /// Called from the producer hot path when `try_push` fails.  Checks
    /// the ring buffer fill level internally; if ≥90% and helping is
    /// enabled, drains a small batch of 32 records inline, formats and
    /// writes each to the helping sink, and returns them to the pool.
    ///
    /// Returns the number of records processed (0 if helping was not
    /// triggered or was unnecessary).
    pub fn try_help(&self) -> usize {
        if !self.enabled {
            return 0;
        }

        let fill = self.ring_buffer.fill_level();
        if fill < 0.90 {
            return 0;
        }

        const HELPING_BATCH: usize = 32;

        // Lock the sink for the duration of the helping drain.
        // This is the only path that locks this Mutex, so contention
        // is zero in practice.
        let mut sink = self.sink.lock().unwrap();

        // SAFETY: drain_helping uses CAS on consumer_sequence so it
        // interoperates safely with the dedicated consumer thread.
        // Each successfully claimed slot grants exclusive access to
        // the record pointer for the duration of this closure.
        let drained = self.ring_buffer.drain_helping(HELPING_BATCH, |record_ptr| {
            // SAFETY: drain_helping CAS protocol guarantees exclusive ownership
            // of the record pointer for the duration of this callback.
            let record = unsafe { &mut *record_ptr };
            let formatted = SinkRef::format_record(record);
            // ConsoleSink::write uses stdout.lock() internally, making
            // concurrent writes from the consumer and helper threads safe.
            let _ = sink.write(&formatted);
            // SAFETY: record is exclusively owned at this point (we won the
            // CAS). Returning it to the pool relinquishes our ownership.
            // The pointer came from this pool via alloc() and has not been freed yet.
            unsafe {
                self.pool.free(record as *const Record);
            }
        });

        if drained > 0 {
            crate::sys::diagnostics::info(
                "coop_helping",
                &format!("Helped drain {drained} records (fill={:.1}%)", fill * 100.0),
            );
        }

        drained
    }

    /// Returns `true` if cooperative helping is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

// ===========================================================================
// Engine
// ===========================================================================

/// The DoLogger engine — holds all runtime state.
pub struct Engine {
    /// Ring buffer for lock-free log submission
    pub ring_buffer: Arc<RingBuffer<*mut Record>>,
    /// Pre-allocated record pool
    pub pool: Arc<RecordPool>,
    /// Background pipeline (consumer thread + sink)
    pub pipeline: Mutex<Option<Pipeline>>,
    /// Active configuration
    pub config: DologgerConfig,
    /// Time source for timestamps and IDs
    pub time_source: TimeSource,
    /// Ed25519 signature engine — owns the signing key
    pub signature_engine: SignatureEngine,
    /// Plugin manager
    pub plugin_manager: PluginManager,
    /// Sysmon self-monitoring channel
    pub sysmon: Sysmon,
    /// Audit pipeline — independent AUDIT record processing
    pub audit_pipeline: Option<AuditPipeline>,
    /// External anchor manager — periodic LSN chain anchoring
    pub external_anchor: Option<Arc<Mutex<ExternalAnchor>>>,
    /// Cooperative helping state for producer threads.
    /// `None` when cooperative helping is disabled in config.
    pub coop_helping: Option<CooperativeHelping>,
}

impl Engine {
    /// Initialize the engine with the given config.
    pub fn init(config: DologgerConfig) -> Result<Self, String> {
        // Initialise diagnostic log FIRST — before any other operations
        crate::sys::diagnostics::init("./dologger_internal.log");

        let pool_capacity = config.ring_buffer_size;
        let pool = Arc::new(RecordPool::new(pool_capacity));
        let ring_buffer = Arc::new(RingBuffer::new(pool_capacity));

        // Create and open the configured sinks, fanning out to all of them.
        // `config.sinks` is guaranteed non-empty (console default) by the
        // config layer.
        let mut sink = SinkRef::new(crate::sink::registry::build_fanout(&config.sinks)?);
        sink.open()
            .map_err(|e| format!("Failed to open sink: {e}"))?;

        // Initialise signature engine with a fresh key pair
        let signature_engine = SignatureEngine::new();

        // Initialise policy components
        let rate_limiter = Arc::new(RateLimiter::default());
        let drop_level_policy = Arc::new(DropLevelPolicy::new(
            crate::record::LogLevel::Trace, // Allow all levels by default
        ));

        // Initialise plugin manager
        let mut plugin_manager = PluginManager::new(
            vec![
                std::path::PathBuf::from("./plugins"),
                std::path::PathBuf::from("/usr/lib/dologger/plugins"),
            ],
            config.is_dev_mode(),
        );
        plugin_manager.set_allow_red_plugins(config.plugin_allow_red_plugins);

        // Wire plugin trust: a committed trust store (active.pub + revoked.txt)
        // is authoritative; otherwise fall back to the legacy single anchor.
        // Failures are logged and startup continues (unsigned = Red) so a
        // missing store can never brick the engine.
        if let Some(store) = &config.plugin_trust_store {
            if let Err(e) = plugin_manager.load_trust_store(std::path::Path::new(store)) {
                crate::sys::diagnostics::warn(
                    "engine",
                    &format!("plugin trust store load failed: {e}"),
                );
            }
        } else if let Some(anchor_hex) = &config.plugin_trust_anchor {
            match hex::decode(anchor_hex) {
                Ok(bytes) if bytes.len() == 32 => {
                    let mut anchor = [0u8; 32];
                    anchor.copy_from_slice(&bytes);
                    plugin_manager.set_trust_anchor(anchor);
                }
                _ => crate::sys::diagnostics::warn(
                    "engine",
                    "plugin_trust_anchor must be a 64-hex Ed25519 public key",
                ),
            }
        }

        // Resolve the plugin dispatch for the pipeline (M6). When
        // `plugin_enable_pipeline` is set, load plugins, initialise them
        // (which hands each its host-accessor bridge), and resolve their
        // formatter/field-provider vtables. Default off: the engine loads no
        // plugins at runtime, so the dispatch is empty and the pipeline uses
        // its built-in plain-text formatting — unchanged from v0.1.0.
        let dispatch = if config.plugin_enable_pipeline {
            for (name, e) in plugin_manager.discover() {
                crate::sys::diagnostics::warn(
                    "engine",
                    &format!("plugin load failed: {name} — {e}"),
                );
            }
            let names: Vec<String> = plugin_manager
                .plugin_names()
                .iter()
                .map(|s| s.to_string())
                .collect();
            for name in &names {
                if let Err(e) = plugin_manager.init_plugin(name) {
                    crate::sys::diagnostics::warn(
                        "engine",
                        &format!("plugin init failed: {name} — {e}"),
                    );
                }
            }
            plugin_manager.resolve_dispatch()
        } else {
            crate::plugin::vtable::PluginDispatch::default()
        };

        // Create pipeline with all stage dependencies.
        // Each pipeline and the main engine use independent SignatureEngine
        // instances for key isolation.  The audit pipeline (below) also
        // receives its own key pair.
        let pipeline_sig_engine = Arc::new(SignatureEngine::new());
        let pipeline = Pipeline::new(
            &config,
            Arc::clone(&ring_buffer),
            Arc::clone(&pool),
            sink,
            pipeline_sig_engine,
            Arc::clone(&rate_limiter),
            Arc::clone(&drop_level_policy),
            dispatch,
            None,
        )?;

        // Start sysmon channel
        let sysmon = Sysmon::start();

        // Create audit pipeline with its own ring buffer partition
        let (audit_pipeline, external_anchor) = if config.enable_signature {
            let worm_sink = WormSink::new(Default::default());
            let security_sink = SecuritySink::new(Default::default());
            let sig_engine = Arc::new(SignatureEngine::new());

            // Create external anchor manager for periodic chain anchoring
            let anchor = Arc::new(Mutex::new(ExternalAnchor::new(3600))); // 1 hour default
            let anchor_for_pipeline = Arc::clone(&anchor);

            match AuditPipeline::new(
                config.ring_buffer_size,
                crate::audit::DEFAULT_AUDIT_BUFFER_RATIO,
                Arc::clone(&pool),
                worm_sink,
                security_sink,
                sig_engine,
                Some(anchor_for_pipeline),
            ) {
                Ok(ap) => {
                    sysmon.info("engine", "Audit pipeline started with external anchoring");
                    (Some(ap), Some(anchor))
                }
                Err(e) => {
                    sysmon.error("engine", &format!("Audit pipeline failed: {e}"));
                    (None, None)
                }
            }
        } else {
            sysmon.info("engine", "Audit pipeline disabled (enable_signature=false)");
            (None, None)
        };

        // Create cooperative helping context
        let coop_helping = if config.ring_buffer_coop_helping {
            let mut helping_sink = SinkRef::new(ConsoleSink::new());
            helping_sink
                .open()
                .map_err(|e| format!("Failed to open cooperative helping sink: {e}"))?;
            Some(CooperativeHelping::new(
                Arc::clone(&ring_buffer),
                Arc::clone(&pool),
                helping_sink,
                true,
            ))
        } else {
            None
        };

        // Report pipeline startup to sysmon
        sysmon.info(
            "engine",
            &format!(
                "Engine initialized: ring_size={}, coop_helping={}",
                config.ring_buffer_size,
                coop_helping.is_some()
            ),
        );

        Ok(Self {
            ring_buffer,
            pool,
            pipeline: Mutex::new(Some(pipeline)),
            config,
            time_source: TimeSource::new(),
            signature_engine,
            plugin_manager,
            sysmon,
            audit_pipeline,
            external_anchor,
            coop_helping,
        })
    }

    /// Shutdown the engine gracefully.
    pub fn shutdown(&mut self) {
        self.sysmon.info("core", "Engine shutdown initiated");

        // Shutdown audit pipeline first (independent drain)
        if let Some(ref mut ap) = self.audit_pipeline {
            ap.shutdown();
        }

        // Shutdown main pipeline
        if let Ok(mut guard) = self.pipeline.lock() {
            if let Some(ref mut p) = *guard {
                p.shutdown();
            }
            *guard = None;
        }

        self.sysmon.shutdown();
        crate::sys::diagnostics::info("core", "DoLogger engine shutdown complete");
        crate::sys::diagnostics::close();
    }
}

/// Create a `DologgerHandle` from an Engine (returns owning raw pointer).
pub(crate) fn create_handle(engine: Engine) -> *mut DologgerHandle {
    Box::into_raw(Box::new(DologgerHandle { engine }))
}

/// Reclaim and drop a `DologgerHandle`.
///
/// # Safety
///
/// `handle` must have been returned by `create_handle` and not yet destroyed.
pub(crate) unsafe fn destroy_handle(handle: *mut DologgerHandle) {
    if !handle.is_null() {
        // SAFETY: Caller guarantees the handle was created by create_handle
        // and hasn't been destroyed yet. Box::from_raw takes ownership back.
        unsafe {
            let _ = Box::from_raw(handle);
        }
    }
}
