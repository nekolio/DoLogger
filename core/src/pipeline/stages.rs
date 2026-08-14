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

use crate::policy::{DropLevelPolicy, RateLimiter};
use crate::record::{LogLevel, Record};
use crate::security::SecretDetector;
use crate::security::SignatureEngine;
use crate::sys::diag;

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

/// Context shared across all pipeline stages within a single drain cycle.
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
    /// Monotonically increasing LSN counter (persists across batches)
    pub lsn_counter: AtomicU64,
    /// SHA-256 hash of the previous record's (lsn || signature)
    pub prev_hash: Mutex<[u8; 32]>,
    /// Format kind set by the Formatting stage (e.g., "plain", "sif", "json")
    pub format_kind: Mutex<Option<String>>,
}

impl<'a> PipelineContext<'a> {
    /// Create a new pipeline context.
    pub fn new(
        signature_engine: &'a SignatureEngine,
        rate_limiter: &'a RateLimiter,
        drop_level_policy: &'a DropLevelPolicy,
        enable_signature: bool,
    ) -> Self {
        Self {
            signature_engine,
            rate_limiter,
            drop_level_policy,
            enable_signature,
            stage_stats: Default::default(),
            // LSN counter starts at 1 (0 is reserved for uninitialized records)
            lsn_counter: AtomicU64::new(1),
            // First record's prev_hash is all zeros
            prev_hash: Mutex::new([0u8; 32]),
            // Format kind defaults to "plain"; the Formatting stage may
            // override this when a formatter plugin selects a different format.
            // Pre-initialized to avoid a per-record allocation in the hot path.
            format_kind: Mutex::new(Some("plain".to_string())),
        }
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

/// Build the signing payload for the Assembly stage.
///
/// This mirrors `SignatureEngine::build_signing_payload_static` and covers
/// Ring 0 + Ring 1 fields: id, timestamp, lsn, prev_hash, level, message,
/// source location, thread_id, and process_id.
fn build_assembly_signing_payload(record: &Record) -> Vec<u8> {
    let mut data = Vec::with_capacity(256);

    // Ring 0: id, timestamp
    data.extend_from_slice(&record.id.hi.to_le_bytes());
    data.extend_from_slice(&record.id.lo.to_le_bytes());
    data.extend_from_slice(&record.timestamp.hi.to_le_bytes());
    data.extend_from_slice(&record.timestamp.lo.to_le_bytes());

    // LSN + prev_hash
    data.extend_from_slice(&record.lsn.to_le_bytes());
    data.extend_from_slice(&record.prev_hash);

    // Ring 1: level + message
    data.push(record.level as u8);
    data.extend_from_slice(record.message.as_str().as_bytes());

    // Source location
    data.extend_from_slice(&record.source_line.to_le_bytes());
    data.extend_from_slice(&record.source_column.to_le_bytes());

    // Thread/process
    data.extend_from_slice(&record.thread_id.to_le_bytes());
    data.extend_from_slice(&record.process_id.to_le_bytes());

    data
}

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
    // HostInfoProvider + FieldProvider plugins enrich the record.
    // Built-in fields (thread_id, process_id, host_name) are set in
    // the hot-path dologger_log FFI call, not here.
    ctx.record(StageIndex::FieldProvider, StageAction::Continue);

    // ── Stage 3: Assembly ───────────────────────────────────────────
    // Assign LSN, compute prev_hash, and optionally sign AUDIT records.
    {
        // 1. Allocate a unique LSN for every record passing Assembly
        let lsn = ctx.lsn_counter.fetch_add(1, Ordering::Relaxed);
        record.lsn = lsn;

        // 2. Set prev_hash from the previous record's (lsn || signature)
        {
            let prev = ctx.prev_hash.lock().unwrap();
            record.prev_hash = *prev;
        }

        // 3. Sign only AUDIT records when signing is enabled
        if record.level == LogLevel::Audit && ctx.enable_signature {
            // Build the signing payload (same schema as SignatureEngine)
            let payload = build_assembly_signing_payload(record);
            // SAFETY: sign_bytes uses the Ed25519 signing key to produce a
            // cryptographic signature over the provided bytes. The key is
            // always valid (initialised at engine startup). The caller is
            // responsible for building a correct payload.
            record.signature = ctx.signature_engine.sign_bytes(&payload);
        }

        // 4. Update prev_hash for the next record: SHA-256(lsn || signature)
        {
            let mut hasher = Sha256::new();
            hasher.update(lsn.to_le_bytes());
            hasher.update(record.signature);
            let new_prev_hash: [u8; 32] = hasher.finalize().into();
            // SAFETY: Mutex::lock returns a poisoned lock if a previous holder
            // panicked. We unwrap() to propagate the panic — there is no safe
            // recovery path if pipeline prev_hash state is corrupted.
            let mut prev = ctx.prev_hash.lock().unwrap();
            *prev = new_prev_hash;
        }
    }
    ctx.record(StageIndex::Assembly, StageAction::Continue);

    // ── Stage 4: Processing ─────────────────────────────────────────
    // Verify Ring 3 ext_data integrity via CRC32C.
    // If ext_data is non-empty, recompute its CRC32C and compare against
    // the stored ext_crc32c. A mismatch indicates data corruption or
    // tampering by an untrusted plugin — log a security warning and
    // zero out the extension data.
    if !record.ext_data.is_empty() {
        let computed_crc = crate::security::crc32c(record.ext_data.as_str().as_bytes());
        if computed_crc != record.ext_crc32c {
            // CRC mismatch — tampering or corruption detected
            diag::warn(
                "security",
                &format!(
                    "CRC32C mismatch on ext_data for record LSN={}: expected=0x{:08x}, got=0x{:08x}. Zeroing ext_data.",
                    record.lsn, record.ext_crc32c, computed_crc
                ),
            );
            record.ext_data.set("");
        }
    }
    // Secret leak detection — scan the record message for leaked credentials
    // (AWS keys, GitHub tokens, JWT, private keys, etc.).  The detector is
    // created once per pipeline instance (lightweight, no heap allocation in
    // the hot path after construction).
    {
        let mut detector = SecretDetector::default();
        let result = detector.scan(record.message.as_str());
        if result.should_block {
            record.message.set("[SECRET-BLOCKED]");
            diag::warn(
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
            diag::info(
                "security",
                &format!("Secret leak MASKED in record LSN={}", record.lsn),
            );
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
        diag::info(
            "pipeline",
            &format!(
                "Batch: {batch_size} records. PreFilter dropped {prefilter_dropped}, Filter dropped {filter_dropped}"
            ),
        );
    }
}
