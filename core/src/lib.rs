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
pub mod util;

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use crate::audit::AuditPipeline;
use crate::buffer::RecordPool;
use crate::buffer::RingBuffer;
use crate::config::{ConfigWatcher, DologgerConfig, HotReloadManager};
use crate::ffi::DologgerHandle;
use crate::pipeline::Pipeline;
use crate::plugin::PluginManager;
use crate::policy::{DropLevelPolicy, RateLimiter};
use crate::security::ExternalAnchor;
use crate::security::{SignatureEngine, TpmKeyProvider};
use crate::sink::ConsoleSink;
use crate::sink::SecuritySink;
use crate::sink::ShmSink;
use crate::sink::ShmSinkConfig;
use crate::sink::SinkRef;
use crate::sink::WormSink;
use crate::sys::control_plane::{ControlPlane, ControlPlaneConfig, ControlPlaneStats, ReloadCb};
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
pub use util::hex;

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
        let sink = self.sink.lock().unwrap();

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
    /// Shared, swappable fan-out sink. Held so hot reload can atomically
    /// replace the active output without rebuilding the pipeline.
    pub sink: Arc<SinkRef>,
    /// Optional shared-memory sink, wired separately from `[sinks.*]`.
    pub shm_sink: Option<Arc<ShmSink>>,
    /// Active configuration
    pub config: DologgerConfig,
    /// Live counters shared with the optional control plane.
    pub control_stats: Arc<ControlPlaneStats>,
    /// Epoch-aware state manager for plugin and configuration reloads.
    pub hot_reload_manager: Arc<HotReloadManager>,
    /// Pending configuration parsed by the watcher thread.
    pending_reload: Arc<Mutex<Option<DologgerConfig>>>,
    /// Optional native config watcher owned by the engine.
    config_watcher: Option<ConfigWatcher>,
    /// Optional operational control-plane listener.
    control_plane: Option<ControlPlane>,
    /// Time source for timestamps and IDs.
    pub time_source: TimeSource,
    /// Ed25519 signature engine — owns the signing key
    pub signature_engine: Arc<SignatureEngine>,
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
        // config layer. The sink is shared (`Arc<SinkRef>`) so hot reload can
        // swap its inner sink atomically at runtime.
        let sink = Arc::new(SinkRef::new(crate::sink::registry::build_fanout(
            &config.sinks,
        )?));
        sink.open()
            .map_err(|e| format!("Failed to open sink: {e}"))?;

        // Initialise live operational metrics before any producer can submit.
        let pending_reload = Arc::new(Mutex::new(None));
        let control_stats = Arc::new(ControlPlaneStats::new());
        control_stats.configure(
            &format!("{:?}", config.performance_profile),
            pool_capacity,
            config.enable_signature,
        );

        // Initialise epoch-aware reload state.
        let hot_reload_manager = Arc::new(HotReloadManager::new());
        control_stats.set_hot_reload_epoch(hot_reload_manager.current_epoch());

        // Initialise signature engine with a fresh key pair
        if config.enable_signature
            && std::env::var_os("DO_LOGGER_KEY_PROVIDER").as_deref()
                == Some(std::ffi::OsStr::new("tpm"))
        {
            let mut tpm_provider = TpmKeyProvider::new(None);
            tpm_provider
                .open()
                .map_err(|error| format!("TPM key provider required but unavailable: {error}"))?;
            return Err(
                "TPM backend is available but SignatureEngine integration is not enabled"
                    .to_string(),
            );
        }
        let signature_engine = Arc::new(SignatureEngine::new());

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
            match crate::util::hex::decode(anchor_hex) {
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
        // its built-in plain-text formatting — unchanged from v0.0.1.
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

        control_stats.set_plugins(plugin_manager.plugin_names().len());

        // Start sysmon channel before building sinks, so early telemetry
        // (e.g. SHM_INIT) is captured.
        let sysmon = Sysmon::start();

        // Wire the optional shared-memory sink before the main pipeline so the
        // consumer thread can mirror accepted records into it. sink_shm is
        // wired separately from `[sinks.*]` (see sink/registry) and is never
        // used for the AUDIT domain — it is rejected when the audit pipeline is on.
        let shm_sink = match &config.shm {
            Some(shm_config) => {
                ShmSinkConfig::check_audit_forbidden(config.enable_audit)?;
                shm_config.validate()?;
                let sink = Arc::new(ShmSink::new(shm_config.clone()));
                sink.open(&sysmon)
                    .map_err(|e| format!("Failed to open sink_shm: {e}"))?;
                Some(sink)
            }
            None => None,
        };
        let pipeline_shm = shm_sink.clone();

        // Create the main pipeline for ordinary records. AUDIT records are
        // routed away before they can enter this ring.
        let mut pipeline = Pipeline::new(
            &config,
            Arc::clone(&ring_buffer),
            Arc::clone(&pool),
            Arc::clone(&sink),
            Arc::clone(&signature_engine),
            Arc::clone(&rate_limiter),
            Arc::clone(&drop_level_policy),
            dispatch,
            None,
            pipeline_shm,
        )?;
        if config.enable_audit && config.enable_signature && config.sig_sidecar_path.is_none() {
            return Err("signed audit mode requires sig_sidecar_path".to_string());
        }

        // Create the dedicated audit partition only when explicitly enabled.
        // Signing is an independent option within that partition.
        let (audit_pipeline, external_anchor) = if config.enable_audit {
            let sidecar_path = config.sig_sidecar_path.as_deref();
            let worm_sink = WormSink::new(Default::default());
            let security_sink = SecuritySink::new(Default::default());
            let anchor = config
                .enable_signature
                .then(|| Arc::new(Mutex::new(ExternalAnchor::new(3600))));
            let anchor_for_pipeline = anchor.clone();
            let audit_pipeline = match AuditPipeline::new(
                config.ring_buffer_size,
                crate::audit::DEFAULT_AUDIT_BUFFER_RATIO,
                Arc::clone(&pool),
                worm_sink,
                security_sink,
                Arc::clone(&signature_engine),
                anchor_for_pipeline,
                sidecar_path,
                config.enable_signature,
            ) {
                Ok(audit_pipeline) => audit_pipeline,
                Err(error) => {
                    pipeline.shutdown();
                    return Err(error);
                }
            };
            sysmon.info(
                "engine",
                &format!(
                    "Audit pipeline started (signed={}, external_anchor={})",
                    config.enable_signature, config.enable_signature
                ),
            );
            (Some(audit_pipeline), anchor)
        } else {
            sysmon.info("engine", "Audit pipeline disabled (enable_audit=false)");
            (None, None)
        };

        // Create cooperative helping context
        let coop_helping = if config.ring_buffer_coop_helping {
            let helping_sink = SinkRef::new(ConsoleSink::new());
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
            sink,
            shm_sink,
            config,
            control_stats,
            hot_reload_manager,
            pending_reload,
            config_watcher: None,
            control_plane: None,
            time_source: TimeSource::new(),
            signature_engine,
            plugin_manager,
            sysmon,
            audit_pipeline,
            external_anchor,
            coop_helping,
        })
    }

    /// Atomically reload the engine's output configuration.
    ///
    /// Builds a new fan-out sink from `new_config` and opens it; only on
    /// success does it swap the active sink (under `SinkRef`'s write lock) and
    /// replace the stored config. On any error the previous configuration and
    /// sink stay in effect and no records are lost. The returned error is one
    /// of the config reload codes:
    /// [`DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID`] (new config rejected) or
    /// [`DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED`] (sink build/open failed).
    ///
    /// [`DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID`]: crate::error::DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID
    /// [`DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED`]: crate::error::DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED
    pub fn reload_config(&mut self, new_config: DologgerConfig) -> Result<(), i32> {
        let reload_ticket = self.hot_reload_manager.begin_config_reload();
        let reload_epoch = reload_ticket.epoch;
        self.control_stats.set_hot_reload_epoch(reload_epoch);
        self.control_stats.record_reload();

        use crate::error::{
            DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED, DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID,
        };

        // Build a new fan-out from the incoming config. Validation happens
        // here via the registry's sink construction; a rejected config is
        // reported and the previous one is left untouched.
        let new_sink = match crate::sink::registry::build_fanout(&new_config.sinks) {
            Ok(s) => s,
            Err(e) => {
                self.sysmon.error(
                    "engine",
                    &format!(
                        "Config reload rejected: {e} (err {DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID})"
                    ),
                );
                self.hot_reload_manager.complete_config_reload(
                    reload_ticket.clone(),
                    false,
                    Some(e.to_string()),
                );
                return Err(DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID);
            }
        };
        let new_ref = SinkRef::new(new_sink);
        if let Err(e) = new_ref.open() {
            self.sysmon.error(
                "engine",
                &format!("Config reload failed to open sink: {e} (err {DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED})"),
            );
            self.hot_reload_manager.complete_config_reload(
                reload_ticket.clone(),
                false,
                Some(e.to_string()),
            );
            return Err(DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED);
        }

        // Swap atomically; close the replaced sink after the swap so any
        // in-flight write finishes under the same lock acquisition.
        let mut old = self.sink.swap(new_ref);
        if let Err(e) = old.close() {
            self.sysmon.warn(
                "engine",
                &format!("Config reload: closing previous sink failed: {e}"),
            );
        }

        self.config = new_config;
        self.hot_reload_manager
            .complete_config_reload(reload_ticket, true, None);
        self.sysmon.info("engine", "Configuration hot-reloaded");
        Ok(())
    }

    /// Start the configured native watcher for one TOML file.
    ///
    /// The watcher callback only parses and queues a new configuration. It
    /// never mutates sinks or engine state from the background thread.
    pub fn start_config_watcher(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref().to_path_buf();
        let pending = Arc::clone(&self.pending_reload);
        let watcher_config = self.config.watcher.clone();
        let watcher = ConfigWatcher::start(
            vec![path],
            Box::new(move |changed| {
                let path = changed
                    .to_str()
                    .ok_or_else(|| "Config watcher path is not valid UTF-8".to_string())?;
                let (config, warnings) =
                    DologgerConfig::load_from_file(path).map_err(|(_, message)| message)?;
                for warning in warnings {
                    crate::sys::diagnostics::warn("config_watcher", &warning);
                }
                *pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(config);
                Ok(())
            }),
            watcher_config,
        )?;
        self.config_watcher = Some(watcher);
        Ok(())
    }

    /// Apply one configuration queued by the watcher, if any.
    pub fn poll_config_reload(&mut self) -> Result<bool, i32> {
        let next = self
            .pending_reload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        match next {
            Some(config) => self.reload_config(config).map(|()| true),
            None => Ok(false),
        }
    }

    /// Stop the watcher and discard a configuration that has not been applied.
    pub fn stop_config_watcher(&mut self) {
        if let Some(mut watcher) = self.config_watcher.take() {
            watcher.shutdown();
        }
        self.pending_reload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    /// Start the opt-in operational control plane for this engine.
    ///
    /// The listener receives the same atomic metrics object returned by
    /// [`Engine::status_stats`]. Reload requests are acknowledged only after
    /// the caller has installed a watcher and polls `poll_config_reload`.
    pub fn start_control_plane(&mut self, config: ControlPlaneConfig) -> Result<(), String> {
        if self.control_plane.is_some() {
            return Err("control plane is already running".into());
        }
        let level = Arc::new(Mutex::new(self.config.level.clone()));
        let reload_callback: ReloadCb = Arc::new(Mutex::new(Some(Box::new(|| Ok(())))));
        let control_plane =
            ControlPlane::start_with_stats(config, level, reload_callback, self.status_stats())?;
        self.control_plane = Some(control_plane);
        Ok(())
    }

    /// Stop the opt-in operational control plane.
    pub fn stop_control_plane(&mut self) {
        if let Some(mut control_plane) = self.control_plane.take() {
            control_plane.shutdown();
        }
    }

    /// Return the live metrics object used by the control plane.
    pub fn status_stats(&self) -> Arc<ControlPlaneStats> {
        Arc::clone(&self.control_stats)
    }

    /// Shutdown the engine gracefully.
    pub fn shutdown(&mut self) {
        self.stop_control_plane();
        self.stop_config_watcher();
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

        // Close the shared-memory sink (mark producer dead + cleanup) after
        // the consumer thread has joined, so no writer races the close.
        if let Some(shm) = &self.shm_sink {
            shm.close(&self.sysmon);
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
