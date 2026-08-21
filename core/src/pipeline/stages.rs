//! Multi-stage pipeline stages.
//!
//! The pipeline processes records through an ordered chain of stages:
//!
//! | Stage | Order | Description | Can drop? | Can modify? |
//! |-------|-------|-------------|-----------|-------------|
//! | PreFilter | 0 | PolicyProvider (rate_limit, drop_level) | Yes | No |
//! | Filter | 1 | Filter plugins | Yes | No |
//! | FieldProvider | 2 | HostInfoProvider + FieldProvider plugins | No | Ring1 write |
//! | Assembly | 3 | Core: LSN assign, sign (if AUDIT) | No | Ring0+1 write |
//! | Processing | 4 | Processor plugins (enrich, mask) | Yes | Ring2+3 write |
//! | Formatting | 5 | Formatter plugins → SIF/text | No | Read-only |
//! | Sink | 6 | Core built-in sinks (fan-out write) | No | Read-only |

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::pipeline::policy::{DropLevelPolicy, RateLimiter};
use crate::plugin::vtable::PluginDispatch;
use crate::record::{LogLevel, Record};
use crate::security::SecretDetector;
use crate::security::SignatureEngine;
use crate::sys::diagnostics;

// ===========================================================================
// Pipeline stage result
// ===========================================================================

/// Result of processing a record through a pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageAction {
    /// Continue to the next stage
    Continue,
    /// Drop this record (return to pool, stop processing)
    Drop,
    /// Drop and record as rate-limited
    RateLimited,
    /// Fatal error — stop the pipeline
    Abort,
}

/// Statistics collected per stage for sysmon reporting.
#[derive(Debug, Clone, Default)]
pub struct StageStats {
    /// Records that passed through this stage
    pub passed: u64,
    /// Records dropped by this stage
    pub dropped: u64,
    /// Records that caused an error in this stage
    pub errors: u64,
    /// Cumulative processing time (microseconds)
    pub total_time_us: u64,
}

// ===========================================================================
// Pipeline context — shared state across stages
// ===========================================================================

/// Signature slot produced by the post-processing signing step: `(lsn,
/// content_hash, signature)`. Stored per pipeline context so the consumer loop
/// can drain it and persist it to the `<log>.sig` sidecar (ADR-002 A.6).
type SignatureSlot = Option<(u64, [u8; 32], [u8; 64])>;

/// Context shared across all pipeline stages within a single drain cycle.
///
/// Persistent chain state (LSN counter, predecessor hash inputs) is *borrowed*
/// from the consumer loop: a fresh context is created per drain cycle, so any
/// state that must survive batch boundaries cannot live here as an owned field.
pub struct PipelineContext<'a> {
    /// Signature engine (for assembly stage)
    pub signature_engine: &'a SignatureEngine,
    /// Rate limiter (for pre_filter stage)
    pub rate_limiter: &'a RateLimiter,
    /// Drop level policy (for pre_filter stage)
    pub drop_level_policy: &'a DropLevelPolicy,
    /// Whether AUDIT-level signatures are enabled
    pub enable_signature: bool,
    /// Per-stage statistics
    pub stage_stats: [StageStats; 7],
    /// Monotonically increasing LSN counter. Borrowed from the consumer loop so
    /// the sequence survives batch boundaries (0 is reserved for uninitialized
    /// records, so the first assigned LSN is 1).
    pub lsn_counter: &'a AtomicU64,
    /// A.6 predecessor content_hash for the derived prev_hash (persists across
    /// batches; owned by the consumer loop).
    pub prev_content_hash: &'a Mutex<[u8; 32]>,
    /// A.6 predecessor LSN for the derived prev_hash (persists across batches;
    /// owned by the consumer loop).
    pub prev_lsn: &'a Mutex<u64>,
    /// Slot for the most recently produced AUDIT signature `(lsn, content_hash,
    /// signature)`. The consumer drains it right after each accepted record and
    /// writes the sidecar line when the LSN matches the record.
    last_signature: Mutex<SignatureSlot>,
    /// Format kind set by the Formatting stage (e.g., "plain", "sif", "json")
    pub format_kind: Mutex<Option<String>>,
    /// Resolved plugin dispatch (formatter + field-provider vtables, M6). The
    /// consumer loop holds one `PluginDispatch` and loans it to every batch.
    /// Empty by default (no plugins loaded), in which case the FieldProvider
    /// and Formatting stages dispatch nothing and the built-in plain-text
    /// formatting is used — behaviour unchanged from v0.0.1.
    pub dispatch: &'a PluginDispatch,
}

impl<'a> PipelineContext<'a> {
    /// Create a new pipeline context.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        signature_engine: &'a SignatureEngine,
        rate_limiter: &'a RateLimiter,
        drop_level_policy: &'a DropLevelPolicy,
        enable_signature: bool,
        dispatch: &'a PluginDispatch,
        lsn_counter: &'a AtomicU64,
        prev_content_hash: &'a Mutex<[u8; 32]>,
        prev_lsn: &'a Mutex<u64>,
    ) -> Self {
        Self {
            signature_engine,
            rate_limiter,
            drop_level_policy,
            enable_signature,
            stage_stats: Default::default(),
            lsn_counter,
            prev_content_hash,
            prev_lsn,
            // The signature slot is drained per record by the consumer loop.
            last_signature: Mutex::new(None),
            // Format kind defaults to "plain"; the Formatting stage may
            // override this when a formatter plugin selects a different format.
            // Pre-initialized to avoid a per-record allocation in the hot path.
            format_kind: Mutex::new(Some("plain".to_string())),
            dispatch,
        }
    }

    /// Drain the most recently produced AUDIT signature.
    ///
    /// The consumer loop calls this after `run_pipeline` accepts a record and
    /// writes the sidecar line only when the returned LSN matches that record's
    /// LSN — a signature left behind by a record dropped at a later stage must
    /// never be attributed to a different record.
    pub fn take_last_signature(&self) -> SignatureSlot {
        self.last_signature.lock().unwrap().take()
    }

    /// Record a statistic for a stage.
    pub fn record(&mut self, stage: StageIndex, action: StageAction) {
        match action {
            StageAction::Continue => self.stage_stats[stage as usize].passed += 1,
            StageAction::Drop | StageAction::RateLimited => {
                self.stage_stats[stage as usize].dropped += 1
            }
            StageAction::Abort => self.stage_stats[stage as usize].errors += 1,
        }
    }
}

/// Index of each pipeline stage (matches the table above).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum StageIndex {
    /// Policy evaluation (rate limit, drop level)
    PreFilter = 0,
    /// Filter plugin evaluation
    Filter = 1,
    /// HostInfo + FieldProvider enrichment
    FieldProvider = 2,
    /// Core assembly + signing
    Assembly = 3,
    /// Processor plugins
    Processing = 4,
    /// Formatter plugins
    Formatting = 5,
    /// Sink output
    Sink = 6,
}

// ===========================================================================
// Pipeline stage runner
// ===========================================================================

/// Runs a single record through all pipeline stages.
///
/// Returns `true` if the record should be written to sinks (i.e., not dropped).
/// Returns `false` if the record was dropped by a stage.
#[inline]
pub fn run_pipeline(record: &mut Record, ctx: &mut PipelineContext<'_>) -> bool {
    // ── Stage 0: PreFilter ──────────────────────────────────────────
    if !ctx.drop_level_policy.evaluate(record.level) {
        ctx.record(StageIndex::PreFilter, StageAction::Drop);
        return false;
    }
    if !ctx.rate_limiter.evaluate() {
        ctx.record(StageIndex::PreFilter, StageAction::RateLimited);
        return false;
    }
    ctx.record(StageIndex::PreFilter, StageAction::Continue);

    // ── Stage 1: Filter ─────────────────────────────────────────────
    // Plugin-based filter dispatch will go here.
    // For now, built-in AUDIT-level filter: always pass AUDIT records.
    ctx.record(StageIndex::Filter, StageAction::Continue);

    // ── Stage 2: FieldProvider ──────────────────────────────────────
    // HostInfoProvider + FieldProvider plugins enrich the record. Built-in
    // fields (thread_id, process_id, host_name) are set in the hot-path
    // dologger_log FFI call, not here. Loaded FieldProvider plugins (M6) are
    // dispatched here: each `provide` writes fields via the host accessor.
    // A provider error is logged and the record continues (enrichment is
    // best-effort; a field provider must never drop the record).
    if !ctx.dispatch.field_providers.is_empty() {
        for fp in &ctx.dispatch.field_providers {
            // SAFETY: the record pointer is a valid, exclusively-owned Record
            // during this drain cycle. We hand it to the plugin as an opaque
            // handle; the plugin only writes it back through the host accessor.
            let rc =
                unsafe { (fp.provide)(record as *mut Record as *mut std::ffi::c_void, fp.config) };
            if rc < 0 {
                diagnostics::warn(
                    "pipeline",
                    &format!(
                        "FieldProvider plugin returned error {rc} for record LSN={}; continuing",
                        record.lsn
                    ),
                );
            }
        }
    }
    ctx.record(StageIndex::FieldProvider, StageAction::Continue);

    // ── Stage 3: Assembly ───────────────────────────────────────────
    // Assign LSN and set audit/signed flags. Content hashing and signing run
    // AFTER the Processing stage (below) so the signature covers the final
    // content — secret masking at Processing mutates the message.
    let lsn = ctx.lsn_counter.fetch_add(1, Ordering::Relaxed);
    record.lsn = lsn;
    {
        // Flags are part of the canonical serialization (A.3), so they are set
        // before content_hash is computed. AUDIT records carry the AUDIT flag;
        // signing additionally marks RECORD_FLAG_SIGNED.
        if record.level == LogLevel::Audit {
            record.flags |= crate::record::RECORD_FLAG_AUDIT;
            if ctx.enable_signature {
                record.flags |= crate::record::RECORD_FLAG_SIGNED;
            }
        }
    }
    ctx.record(StageIndex::Assembly, StageAction::Continue);

    // ── Stage 4: Processing ─────────────────────────────────────────
    // Secret leak detection — scan the record message for leaked credentials
    // (AWS keys, GitHub tokens, JWT, private keys, etc.).  The detector is
    // created once per pipeline instance (lightweight, no heap allocation in
    // the hot path after construction).
    {
        let mut detector = SecretDetector::default();
        let result = detector.scan(record.message.as_str());
        if result.should_block {
            record.message.set("[SECRET-BLOCKED]");
            diagnostics::warn(
                "security",
                &format!(
                    "Secret leak BLOCKED in record LSN={}: {:?}",
                    record.lsn,
                    result
                        .findings
                        .iter()
                        .map(|f| f.rule.as_str())
                        .collect::<Vec<_>>()
                ),
            );
        } else if result.detected {
            record.message.set(&result.message);
            diagnostics::info(
                "security",
                &format!("Secret leak MASKED in record LSN={}", record.lsn),
            );
        }
    }

    // ── Post-Processing signing (ADR-002 A.6) ──────────────────────
    // Content hash + signature run after Processing so the signed digest
    // covers the final serialized content. Non-audit records stay unsigned
    // with a zero content_hash (they are outside the tamper-evident chain).
    if record.level == LogLevel::Audit {
        record.compute_content_hash();
        if ctx.enable_signature {
            // Derive the A.6 predecessor hash from the persistent chain state.
            let (prev_ch, prev_lsn) = {
                let ch = ctx.prev_content_hash.lock().unwrap();
                let l = ctx.prev_lsn.lock().unwrap();
                (*ch, *l)
            };
            let mut hasher = Sha256::new();
            hasher.update(prev_ch);
            hasher.update(prev_lsn.to_le_bytes());
            let prev_hash: [u8; 32] = hasher.finalize().into();

            // Digest = SHA-256(lsn || content_hash || prev_hash); the 64-byte
            // signature is exposed to the consumer, never stored on the record
            // (A.2.2 fixed layout keeps no signature field).
            let digest = SignatureEngine::build_signing_payload_static(record, &prev_hash);
            let sig = ctx.signature_engine.sign_bytes(&digest);

            // Expose (lsn, content_hash, signature) for the sidecar write; the
            // signed record becomes the chain predecessor.
            *ctx.last_signature.lock().unwrap() = Some((lsn, record.content_hash, sig));
            *ctx.prev_content_hash.lock().unwrap() = record.content_hash;
            *ctx.prev_lsn.lock().unwrap() = lsn;
        }
    }
    ctx.record(StageIndex::Processing, StageAction::Continue);

    // ── Stage 5: Formatting ─────────────────────────────────────────
    // The format kind is pre-initialized to "plain" in PipelineContext.
    // When formatter plugins are loaded, this stage will select
    // a format based on plugin configuration (SIF, JSON, text, etc.).
    // For now, the default "plain" is already set — no allocation needed.
    ctx.record(StageIndex::Formatting, StageAction::Continue);

    // ── Stage 6: Sink ───────────────────────────────────────────────
    // Multi-sink fan-out + fallback chain.
    // The sink write is handled by the consumer loop.
    ctx.record(StageIndex::Sink, StageAction::Continue);

    true
}

/// Report stage statistics to sysmon and diag.
pub fn report_stats(ctx: &PipelineContext<'_>, batch_size: usize) {
    let prefilter_dropped = ctx.stage_stats[StageIndex::PreFilter as usize].dropped;
    let filter_dropped = ctx.stage_stats[StageIndex::Filter as usize].dropped;

    if prefilter_dropped > 0 || filter_dropped > 0 {
        diagnostics::info(
            "pipeline",
            &format!(
                "Batch: {batch_size} records. PreFilter dropped {prefilter_dropped}, Filter dropped {filter_dropped}"
            ),
        );
    }
}
