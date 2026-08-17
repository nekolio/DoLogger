//! External anchoring proof.
//!
//! Periodically posts the root hash of the audit LSN chain to an
//! external immutable storage (HTTP endpoint or local file) for
//! independent tamper evidence.
//!
//! # Design
//!
//! Each AUDIT record signed by the engine produces a **chain hash**:
//! `SHA-256(lsn || Ed25519_signature)`.  These hashes accumulate
//! between anchors.  When the anchor interval elapses (default: 1 hour),
//! a Merkle-like root hash is computed over all accumulated chain
//! hashes, signed with the engine's Ed25519 key, and stored as an
//! `AnchorRecord`.
//!
//! If an `anchor_url` is configured, the anchor JSON is POSTed to that
//! endpoint so an external verifier can independently attest to chain
//! continuity.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::security::SignatureEngine;

// ---------------------------------------------------------------------------
// AnchorRecord
// ---------------------------------------------------------------------------

/// A signed snapshot of the audit LSN chain posted to external storage.
///
/// Each anchor proves that the audit chain up to `last_lsn` was intact
/// at `timestamp_ms`.  An external verifier can replay the chain from
/// the anchor's `chain_root_hash` and detect any tampering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRecord {
    /// Monotonically increasing anchor sequence number (starts at 1).
    pub anchor_id: u64,
    /// Unix timestamp in milliseconds when this anchor was created.
    pub timestamp_ms: u64,
    /// The highest LSN whose chain hash is included in `chain_root_hash`.
    pub last_lsn: u64,
    /// SHA-256 Merkle root of all accumulated record chain hashes since
    /// the previous anchor.
    pub chain_root_hash: [u8; 32],
    /// Ed25519 signature over `(anchor_id || timestamp_ms || last_lsn || chain_root_hash)`.
    pub signature: [u8; 64],
}

// ---------------------------------------------------------------------------
// ExternalAnchor
// ---------------------------------------------------------------------------

/// Manages periodic anchoring of the audit LSN chain.
///
/// # Thread safety
///
/// Designed for single-threaded use by the audit consumer.  If shared
/// between threads, wrap in `Arc<Mutex<>>`.
pub struct ExternalAnchor {
    /// History of every anchor record produced.
    anchor_history: Vec<AnchorRecord>,
    /// Instant of the most recent anchor (for interval gating).
    last_anchor_time: std::time::Instant,
    /// Minimum wall-clock interval between successive anchors.
    anchor_interval: std::time::Duration,
    /// Accumulated record chain hashes since the last anchor.
    chain_hashes: Vec<[u8; 32]>,
    /// Monotonically increasing anchor ID counter.
    anchor_id_counter: u64,
    /// Optional remote URL to POST anchor JSON to.
    anchor_url: Option<String>,
}

impl ExternalAnchor {
    /// Create a new anchor manager with the given minimum interval.
    ///
    /// `interval_secs` is the minimum number of wall-clock seconds
    /// between successive anchors (default: 3600 / 1 hour).
    pub fn new(interval_secs: u64) -> Self {
        Self {
            anchor_history: Vec::new(),
            last_anchor_time: std::time::Instant::now(),
            anchor_interval: std::time::Duration::from_secs(interval_secs),
            chain_hashes: Vec::new(),
            anchor_id_counter: 1,
            anchor_url: None,
        }
    }

    /// Create a new anchor manager with an optional HTTP posting URL.
    ///
    /// When `url` is `Some(...)`, each anchor is POSTed as JSON to the
    /// remote endpoint (requires the `sink-webhook` feature).
    pub fn with_url(interval_secs: u64, url: Option<String>) -> Self {
        Self {
            anchor_url: url,
            ..Self::new(interval_secs)
        }
    }

    /// Change or set the anchor URL for HTTP posting.
    pub fn set_url(&mut self, url: String) {
        self.anchor_url = Some(url);
    }

    /// Accumulate a record chain hash into the pending anchor batch.
    ///
    /// Call this **after** each AUDIT record is signed.  The hash must
    /// be `SHA-256(lsn || signature)` — the same value used for LSN
    /// chain continuity (see [`SignatureEngine::record_chain_hash`]).
    pub fn accumulate_hash(&mut self, hash: &[u8; 32]) {
        self.chain_hashes.push(*hash);
    }

    /// Create an anchor if the configured interval has elapsed AND there
    /// are accumulated chain hashes.
    ///
    /// Returns `None` if the interval has not passed or if
    /// `chain_hashes` is empty (no records signed since the last anchor).
    ///
    /// # Panics
    ///
    /// Never panics — even if HTTP posting fails, the anchor is still
    /// stored locally and returned.
    pub fn maybe_anchor(&mut self, sig_engine: &SignatureEngine) -> Option<AnchorRecord> {
        // Gate on wall-clock interval
        if self.last_anchor_time.elapsed() < self.anchor_interval {
            return None;
        }

        // Nothing to anchor
        if self.chain_hashes.is_empty() {
            self.last_anchor_time = std::time::Instant::now();
            return None;
        }

        // Merkle root over all accumulated chain hashes
        let chain_root_hash = Self::compute_root_hash(&self.chain_hashes);

        // Highest LSN covered by this anchor
        let last_lsn = sig_engine.current_lsn().saturating_sub(1);

        let anchor_id = self.anchor_id_counter;
        self.anchor_id_counter += 1;

        let timestamp_ms = Self::current_time_ms();

        // Build canonical payload and sign
        let payload =
            Self::build_anchor_payload(anchor_id, timestamp_ms, last_lsn, &chain_root_hash);
        let signature = sig_engine.sign_bytes(&payload);

        let record = AnchorRecord {
            anchor_id,
            timestamp_ms,
            last_lsn,
            chain_root_hash,
            signature,
        };

        // Remote posting (no-op when sink-webhook is off)
        if let Some(ref url) = self.anchor_url {
            let json_val = Self::anchor_to_json(&record);
            let json_str = serde_json::to_string(&json_val).unwrap_or_default();
            let _ = Self::post_anchor(url, &json_str);
        }

        self.anchor_history.push(record.clone());
        self.chain_hashes.clear();
        self.last_anchor_time = std::time::Instant::now();

        Some(record)
    }

    /// Verify every anchor signature against the given public key.
    ///
    /// Returns `true` only when **all** stored anchors have valid Ed25519
    /// signatures.
    pub fn verify_anchor_chain(&self, verifying_key: &VerifyingKey) -> bool {
        for anchor in &self.anchor_history {
            let payload = Self::build_anchor_payload(
                anchor.anchor_id,
                anchor.timestamp_ms,
                anchor.last_lsn,
                &anchor.chain_root_hash,
            );

            let sig = Signature::from_bytes(&anchor.signature);

            if verifying_key.verify(&payload, &sig).is_err() {
                return false;
            }
        }
        true
    }

    /// Export all anchor records as a pretty-printed JSON array.
    ///
    /// Suitable for external verification tools that replay the chain
    /// from the exported anchors.
    pub fn export_anchors_json(&self) -> String {
        let anchors: Vec<serde_json::Value> = self
            .anchor_history
            .iter()
            .map(Self::anchor_to_json)
            .collect();

        serde_json::to_string_pretty(&anchors).unwrap_or_else(|_| "[]".to_string())
    }

    /// Number of anchor records created so far.
    pub fn anchor_count(&self) -> usize {
        self.anchor_history.len()
    }

    /// Reference to the full anchor history.
    pub fn anchor_history(&self) -> &[AnchorRecord] {
        &self.anchor_history
    }

    /// Number of chain hashes currently accumulated (not yet anchored).
    pub fn pending_hash_count(&self) -> usize {
        self.chain_hashes.len()
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Compute the Merkle-style root hash from accumulated chain hashes.
    ///
    /// `root = SHA-256(hash_0 || hash_1 || ... || hash_n)`
    fn compute_root_hash(hashes: &[[u8; 32]]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for h in hashes {
            hasher.update(h);
        }
        hasher.finalize().into()
    }

    /// Build the canonical byte-string signed in an anchor record.
    ///
    /// Layout (little-endian): `anchor_id(8) || timestamp_ms(8) || last_lsn(8) || chain_root_hash(32)`.
    fn build_anchor_payload(
        anchor_id: u64,
        timestamp_ms: u64,
        last_lsn: u64,
        chain_root_hash: &[u8; 32],
    ) -> Vec<u8> {
        let mut data = Vec::with_capacity(8 + 8 + 8 + 32);
        data.extend_from_slice(&anchor_id.to_le_bytes());
        data.extend_from_slice(&timestamp_ms.to_le_bytes());
        data.extend_from_slice(&last_lsn.to_le_bytes());
        data.extend_from_slice(chain_root_hash);
        data
    }

    /// Return the current system time in milliseconds since Unix epoch.
    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Serialize a single `AnchorRecord` to a JSON value.
    fn anchor_to_json(record: &AnchorRecord) -> serde_json::Value {
        serde_json::json!({
            "anchor_id": record.anchor_id,
            "timestamp_ms": record.timestamp_ms,
            "last_lsn": record.last_lsn,
            "chain_root_hash": crate::util::hex::encode(record.chain_root_hash),
            "signature": crate::util::hex::encode(record.signature),
        })
    }

    /// POST the anchor JSON to a remote HTTP endpoint.
    ///
    /// Feature-gated: requires `sink-webhook` to actually send data.
    /// Without it this is a silent no-op.
    #[cfg(feature = "sink-webhook")]
    fn post_anchor(url: &str, json: &str) -> Result<(), String> {
        let resp = ureq::post(url)
            .set("Content-Type", "application/json")
            .send_string(json)
            .map_err(|e| format!("Anchor POST failed: {e}"))?;

        if resp.status() != 200 {
            return Err(format!("Anchor POST returned HTTP {}", resp.status()));
        }
        Ok(())
    }

    #[cfg(not(feature = "sink-webhook"))]
    fn post_anchor(_url: &str, _json: &str) -> Result<(), String> {
        // Without sink-webhook, this is a deliberate no-op.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::SignatureEngine;

    /// Build a dummy chain hash from a seed byte.
    fn dummy_hash(seed: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = seed;
        h
    }

    // ------------------------------------------------------------------
    // test_anchor_creation
    // ------------------------------------------------------------------

    #[test]
    fn test_anchor_creation() {
        let engine = SignatureEngine::new();
        // Use a very short interval so maybe_anchor fires immediately.
        let mut anchor = ExternalAnchor::new(0);

        // Simulate signing a few records: accumulate chain hashes.
        anchor.accumulate_hash(&dummy_hash(1));
        anchor.accumulate_hash(&dummy_hash(2));
        anchor.accumulate_hash(&dummy_hash(3));

        // Interval is 0, so anchor should be created right away.
        let record = anchor.maybe_anchor(&engine).expect("should create anchor");

        assert_eq!(record.anchor_id, 1);
        assert!(record.timestamp_ms > 0);
        // No LSNs were actually assigned (engine is fresh, current_lsn == 1),
        // so last_lsn should be 0.
        assert_eq!(record.last_lsn, 0);
        assert_ne!(record.chain_root_hash, [0u8; 32]);
        assert_ne!(record.signature, [0u8; 64]);

        // Pending hashes should be cleared.
        assert_eq!(anchor.pending_hash_count(), 0);
        assert_eq!(anchor.anchor_count(), 1);
    }

    // ------------------------------------------------------------------
    // test_anchor_chain_verification
    // ------------------------------------------------------------------

    #[test]
    fn test_anchor_chain_verification() {
        let engine = SignatureEngine::new();
        let vk = *engine.verifying_key();
        let mut anchor = ExternalAnchor::new(0);

        // Create a few anchors
        anchor.accumulate_hash(&dummy_hash(10));
        anchor.maybe_anchor(&engine);

        anchor.accumulate_hash(&dummy_hash(20));
        anchor.accumulate_hash(&dummy_hash(21));
        anchor.maybe_anchor(&engine);

        anchor.accumulate_hash(&dummy_hash(30));
        anchor.maybe_anchor(&engine);

        assert_eq!(anchor.anchor_count(), 3);

        // All signatures must verify
        assert!(anchor.verify_anchor_chain(&vk));

        // Verify each signature manually too
        for rec in anchor.anchor_history() {
            let payload = ExternalAnchor::build_anchor_payload(
                rec.anchor_id,
                rec.timestamp_ms,
                rec.last_lsn,
                &rec.chain_root_hash,
            );
            let sig = Signature::from_bytes(&rec.signature);
            assert!(vk.verify(&payload, &sig).is_ok());
        }
    }

    // ------------------------------------------------------------------
    // test_tampered_anchor_fails_verification
    // ------------------------------------------------------------------

    #[test]
    fn test_tampered_anchor_fails_verification() {
        let engine = SignatureEngine::new();
        let vk = *engine.verifying_key();
        let mut anchor = ExternalAnchor::new(0);

        anchor.accumulate_hash(&dummy_hash(42));
        let mut rec = anchor.maybe_anchor(&engine).expect("should create anchor");

        // Tamper with last_lsn
        rec.last_lsn = 9999;

        // Verify against the tampered record directly
        let payload = ExternalAnchor::build_anchor_payload(
            rec.anchor_id,
            rec.timestamp_ms,
            rec.last_lsn,
            &rec.chain_root_hash,
        );
        let sig = Signature::from_bytes(&rec.signature);
        assert!(vk.verify(&payload, &sig).is_err());
    }

    // ------------------------------------------------------------------
    // test_interval_scheduling
    // ------------------------------------------------------------------

    #[test]
    fn test_interval_scheduling() {
        let engine = SignatureEngine::new();
        let vk = *engine.verifying_key();

        // Use a zero interval so every call triggers.
        let mut anchor = ExternalAnchor::new(0);

        // First anchor — should fire.
        anchor.accumulate_hash(&dummy_hash(1));
        let r1 = anchor.maybe_anchor(&engine);
        assert!(r1.is_some());

        // No new hashes — should NOT fire (empty chain_hashes).
        let r2 = anchor.maybe_anchor(&engine);
        assert!(r2.is_none());

        // Accumulate and fire again.
        anchor.accumulate_hash(&dummy_hash(2));
        let r3 = anchor.maybe_anchor(&engine);
        assert!(r3.is_some());
        assert_eq!(r3.unwrap().anchor_id, 2);

        assert!(anchor.verify_anchor_chain(&vk));
    }

    // ------------------------------------------------------------------
    // test_interval_not_elapsed
    // ------------------------------------------------------------------

    #[test]
    fn test_interval_not_elapsed() {
        let engine = SignatureEngine::new();
        // Use a very long interval — will never fire.
        let mut anchor = ExternalAnchor::new(86_400); // 24 hours

        anchor.accumulate_hash(&dummy_hash(7));
        let result = anchor.maybe_anchor(&engine);
        assert!(result.is_none());
        // The hash should still be pending
        assert_eq!(anchor.pending_hash_count(), 1);
    }

    // ------------------------------------------------------------------
    // test_anchor_json_roundtrip
    // ------------------------------------------------------------------

    #[test]
    fn test_anchor_json_roundtrip() {
        let engine = SignatureEngine::new();
        let mut anchor = ExternalAnchor::new(0);

        anchor.accumulate_hash(&dummy_hash(100));
        let rec = anchor.maybe_anchor(&engine).expect("should create anchor");

        let json = anchor.export_anchors_json();
        assert!(!json.is_empty());
        assert!(json.contains("anchor_id"));
        assert!(json.contains("chain_root_hash"));
        assert!(json.contains("signature"));
        assert!(json.contains(&crate::util::hex::encode(rec.chain_root_hash)));
        assert!(json.contains(&crate::util::hex::encode(rec.signature)));

        // JSON must be valid and parseable
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("export_anchors_json must produce valid JSON");
        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 1);
    }

    // ------------------------------------------------------------------
    // test_root_hash_determinism
    // ------------------------------------------------------------------

    #[test]
    fn test_root_hash_determinism() {
        // Same inputs -> same root hash
        let hashes_a = vec![dummy_hash(1), dummy_hash(2), dummy_hash(3)];
        let hashes_b = vec![dummy_hash(1), dummy_hash(2), dummy_hash(3)];

        let root_a = ExternalAnchor::compute_root_hash(&hashes_a);
        let root_b = ExternalAnchor::compute_root_hash(&hashes_b);
        assert_eq!(root_a, root_b);

        // Different inputs -> different root hash
        let hashes_c = vec![dummy_hash(1), dummy_hash(2), dummy_hash(4)];
        let root_c = ExternalAnchor::compute_root_hash(&hashes_c);
        assert_ne!(root_a, root_c);

        // Different order -> different root hash
        let hashes_d = vec![dummy_hash(3), dummy_hash(2), dummy_hash(1)];
        let root_d = ExternalAnchor::compute_root_hash(&hashes_d);
        assert_ne!(root_a, root_d);
    }

    // ------------------------------------------------------------------
    // test_empty_anchors
    // ------------------------------------------------------------------

    #[test]
    fn test_empty_anchors() {
        // Verify chain with zero anchors trivially passes.
        let engine = SignatureEngine::new();
        let vk = *engine.verifying_key();
        let anchor = ExternalAnchor::new(0);
        assert!(anchor.verify_anchor_chain(&vk));
        assert_eq!(anchor.export_anchors_json(), "[]");
    }
}
