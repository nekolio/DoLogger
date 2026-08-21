//! Ed25519 signature and LSN audit chain.
//!
//! # Audit Chain
//!
//! For AUDIT-level records, each record gets:
//! - `security.lsn`: Monotonically increasing Log Sequence Number (uint64)
//! - `security.content_hash`: SHA-256 of the canonical serialization (A.3)
//!
//! Signatures follow ADR-002 A.6: the signed digest is
//! `SHA-256(lsn || content_hash || prev_hash)` where `prev_hash` is derived as
//! `SHA-256(prev.content_hash || prev.lsn)` — never stored on the record. The
//! pipeline writes signatures to the `<log>.sig` sidecar (`dologctl verify-log
//! --sidecar` re-derives the chain offline). Any deletion or reordering of
//! audit records is detectable by offline verification.

use crate::security::os_random::fill_bytes;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::record::Record;

/// Manages Ed25519 key pair and LSN allocation for audit records.
pub struct SignatureEngine {
    /// Ed25519 signing key (private key, kept in memory only)
    signing_key: SigningKey,
    /// Ed25519 verifying key (public key, can be shared)
    verifying_key: VerifyingKey,
    /// Monotonically increasing LSN counter
    lsn_counter: std::sync::atomic::AtomicU64,
}

impl SignatureEngine {
    /// Create a new SignatureEngine with a randomly generated key pair.
    pub fn new() -> Self {
        let mut seed = [0u8; 32];
        fill_bytes(&mut seed).expect("OS CSPRNG unavailable for signature key generation");
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        Self {
            signing_key,
            verifying_key,
            lsn_counter: std::sync::atomic::AtomicU64::new(1), // LSN starts at 1
        }
    }

    /// Create from an existing signing key (e.g., from KeyProvider).
    pub fn from_signing_key(signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
            lsn_counter: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Get the public key bytes (32 bytes).
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    /// Get a reference to the verifying key (for signature verification).
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    /// Sign a record: assigns LSN, computes content_hash, and produces the signature.
    ///
    /// The signature covers the A.6 digest `SHA-256(lsn || content_hash || prev_hash)`
    /// where `prev_hash` is the caller-provided derivation of the previous signed
    /// record (`SHA-256(prev.content_hash || prev.lsn)`). Chain state is tracked
    /// by the caller (the pipeline Assembly stage), not stored on the record.
    ///
    /// # Returns
    ///
    /// The 64-byte Ed25519 signature.
    pub fn sign_record(&self, record: &mut Record, prev_hash: &[u8; 32]) -> [u8; 64] {
        // Assign LSN
        let lsn = self
            .lsn_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        record.lsn = lsn;

        // Compute content_hash (canonical serialization, A.3)
        record.compute_content_hash();

        // Build the A.6 digest: SHA-256(LSN || content_hash || prev_hash)
        let data_to_sign = self.build_signing_payload(record, prev_hash);

        // Sign
        self.signing_key.sign(&data_to_sign).to_bytes()
    }

    /// Sign arbitrary data with the signing key without assigning LSN or prev_hash.
    ///
    /// This is a low-level signing primitive for use by the pipeline Assembly stage
    /// when LSN and prev_hash are managed externally (e.g., by PipelineContext).
    /// Returns the 64-byte Ed25519 signature.
    pub fn sign_bytes(&self, data: &[u8]) -> [u8; 64] {
        self.signing_key.sign(data).to_bytes()
    }

    /// Verify a record's signature using the given public key, the signature
    /// bytes, and the derived previous-record hash (A.6).
    ///
    /// `prev_hash` must match the hash the signer used — for the first record
    /// in a chain this is the all-zeros genesis derivation.
    pub fn verify_record(
        verifying_key: &VerifyingKey,
        record: &Record,
        sig_bytes: &[u8; 64],
        prev_hash: &[u8; 32],
    ) -> Result<(), SignatureError> {
        // Content integrity: the signed content_hash must match a fresh
        // canonical-serialization hash (A.3). Catches tampering with any hashed
        // field (message, level, flags, KV slots, LSN, ...) even when the
        // attacker keeps the original signature.
        let recomputed = Record::compute_content_hash_from(record);
        if recomputed != record.content_hash {
            return Err(SignatureError::ContentTampered);
        }

        let signature = ed25519_dalek::Signature::from_bytes(sig_bytes);

        let data = Self::build_signing_payload_static(record, prev_hash);
        verifying_key
            .verify(&data, &signature)
            .map_err(|_| SignatureError::InvalidSignature)
    }

    /// Verify the chain link between two consecutive records.
    ///
    /// Per ruling #15 the chain relation is carried by LSN monotonicity only:
    /// `prev_hash` is a derivation (`SHA-256(prev.content_hash || prev.lsn)`)
    /// computed at sign/verify time, never stored, so there is no stored
    /// predecessor hash to compare here. Signature covers the derived hash
    /// (ADR-002 A.6); a tampered predecessor fails at signature verification.
    pub fn verify_chain_link(prev: &Record, next: &Record) -> Result<(), SignatureError> {
        if next.lsn <= prev.lsn {
            return Err(SignatureError::LsnRegression);
        }
        Ok(())
    }

    /// Build the A.6 digest to be signed for a given record.
    fn build_signing_payload(&self, record: &Record, prev_hash: &[u8; 32]) -> [u8; 32] {
        Self::build_signing_payload_static(record, prev_hash)
    }

    /// Build the A.6 signing digest: `SHA-256(lsn || content_hash || prev_hash)`.
    ///
    /// Content tampering is detected by recomputing `content_hash` from the
    /// canonical serialization (A.3) and comparing it to the signed value.
    pub(crate) fn build_signing_payload_static(record: &Record, prev_hash: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(record.lsn.to_le_bytes());
        hasher.update(record.content_hash);
        hasher.update(prev_hash);
        hasher.finalize().into()
    }

    /// Get the current LSN (next to be assigned).
    pub fn current_lsn(&self) -> u64 {
        self.lsn_counter.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Compute the chain hash for a signed record.
    ///
    /// This is `SHA-256(lsn || signature)` — the root hash accumulated by
    /// [`ExternalAnchor`](crate::security::ExternalAnchor) between anchors.
    pub fn record_chain_hash(lsn: u64, signature: &[u8; 64]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(lsn.to_le_bytes());
        hasher.update(signature);
        hasher.finalize().into()
    }
}

impl Default for SignatureEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during signature verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    /// The Ed25519 signature is invalid
    InvalidSignature,
    /// The LSN chain is broken (prev_hash mismatch)
    ChainBroken,
    /// LSN is not monotonically increasing
    LsnRegression,
    /// The stored content_hash does not match a fresh canonical-serialization
    /// hash (A.3) of the record — a hashed field was modified after signing.
    ContentTampered,
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "Invalid Ed25519 signature"),
            Self::ChainBroken => write!(f, "LSN chain broken: prev_hash mismatch"),
            Self::LsnRegression => write!(f, "LSN regression detected"),
            Self::ContentTampered => write!(f, "Content tampered: content_hash mismatch"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{LogLevel, Record};

    fn make_test_record(id: u64) -> Record {
        let mut r = Record::new(0);
        r.set_id(0, id);
        r.level = LogLevel::Audit;
        r.message.set("test audit message");
        r.thread_id = 1;
        r.process_id = 1234;
        r
    }

    /// Derive the A.6 predecessor hash for the next record in the chain.
    fn derive_prev_hash(prev: &Record) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(prev.content_hash);
        hasher.update(prev.lsn.to_le_bytes());
        hasher.finalize().into()
    }

    #[test]
    fn test_sign_and_verify() {
        let engine = SignatureEngine::new();
        let mut record = make_test_record(1);

        let sig = engine.sign_record(&mut record, &[0u8; 32]);
        assert!(record.lsn > 0);

        let result =
            SignatureEngine::verify_record(&engine.verifying_key, &record, &sig, &[0u8; 32]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_chain_continuity() {
        let engine = SignatureEngine::new();

        let mut r1 = make_test_record(1);
        let _sig1 = engine.sign_record(&mut r1, &[0u8; 32]);

        let mut r2 = make_test_record(2);
        let prev_hash = derive_prev_hash(&r1);
        let _sig2 = engine.sign_record(&mut r2, &prev_hash);

        // Chain relation is LSN monotonicity (ruling #15); the derived
        // prev_hash is covered by each record's signature (A.6), not by a
        // stored field, so no stored predecessor hash is compared here.
        let result = SignatureEngine::verify_chain_link(&r1, &r2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_chain_prev_hash_binding() {
        let engine = SignatureEngine::new();

        let mut r1 = make_test_record(1);
        let _sig1 = engine.sign_record(&mut r1, &[0u8; 32]);

        let mut r2 = make_test_record(2);
        let prev_hash = derive_prev_hash(&r1);
        let sig2 = engine.sign_record(&mut r2, &prev_hash);

        // A signature bound to the real predecessor hash verifies...
        let ok = SignatureEngine::verify_record(&engine.verifying_key, &r2, &sig2, &prev_hash);
        assert!(ok.is_ok());

        // ...but fails against a wrong predecessor hash (reordering/insertion).
        let wrong = SignatureEngine::verify_record(&engine.verifying_key, &r2, &sig2, &[0u8; 32]);
        assert!(wrong.is_err());
    }

    #[test]
    fn test_lsn_monotonic() {
        let engine = SignatureEngine::new();

        let mut r1 = make_test_record(1);
        engine.sign_record(&mut r1, &[0u8; 32]);

        let mut r2 = make_test_record(2);
        let prev_hash = derive_prev_hash(&r1);
        engine.sign_record(&mut r2, &prev_hash);

        assert!(r2.lsn > r1.lsn);
    }

    #[test]
    fn test_tampered_record_fails_verification() {
        let engine = SignatureEngine::new();
        let mut record = make_test_record(1);

        let sig = engine.sign_record(&mut record, &[0u8; 32]);

        // Tamper with the message
        record.message.set("tampered message");

        let result =
            SignatureEngine::verify_record(&engine.verifying_key, &record, &sig, &[0u8; 32]);
        assert!(result.is_err());
    }
}
