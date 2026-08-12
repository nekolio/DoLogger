//! Ed25519 signature and LSN audit chain.
//!
//! # Audit Chain
//!
//! For AUDIT-level records, each record gets:
//! - `security.lsn`: Monotonically increasing Log Sequence Number (uint64)
//! - `security.prev_hash`: SHA-256(prev_lsn || prev_signature)
//!
//! This forms a blockchain-like chain — any deletion or reordering
//! is detectable by offline verification tools.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
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
    /// Hash of the previous signed record (for chain continuity)
    prev_hash: std::sync::Mutex<[u8; 32]>,
}

impl SignatureEngine {
    /// Create a new SignatureEngine with a randomly generated key pair.
    pub fn new() -> Self {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        Self {
            signing_key,
            verifying_key,
            lsn_counter: std::sync::atomic::AtomicU64::new(1), // LSN starts at 1
            prev_hash: std::sync::Mutex::new([0u8; 32]),       // First record: all zeros
        }
    }

    /// Create from an existing signing key (e.g., from KeyProvider).
    pub fn from_signing_key(signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
            lsn_counter: std::sync::atomic::AtomicU64::new(1),
            prev_hash: std::sync::Mutex::new([0u8; 32]),
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

    /// Sign a record: assigns LSN, computes prev_hash, and produces the signature.
    ///
    /// The signature covers Ring 0 + Ring 1 fields (excluding the signature field itself).
    ///
    /// # Returns
    ///
    /// The 64-byte Ed25519 signature.
    pub fn sign_record(&self, record: &mut Record) -> [u8; 64] {
        // Assign LSN
        let lsn = self
            .lsn_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        record.lsn = lsn;

        // Set prev_hash
        {
            let prev = self.prev_hash.lock().unwrap();
            record.prev_hash = *prev;
        }

        // Build the data to sign: LSN || prev_hash || serialised Ring 0+1 fields
        let data_to_sign = self.build_signing_payload(record);

        // Sign
        let signature = self.signing_key.sign(&data_to_sign);

        // Update prev_hash for the next record
        let mut hasher = Sha256::new();
        hasher.update(lsn.to_le_bytes());
        hasher.update(signature.to_bytes());
        let new_prev_hash: [u8; 32] = hasher.finalize().into();

        {
            let mut prev = self.prev_hash.lock().unwrap();
            *prev = new_prev_hash;
        }

        signature.to_bytes()
    }

    /// Sign arbitrary data with the signing key without assigning LSN or prev_hash.
    ///
    /// This is a low-level signing primitive for use by the pipeline Assembly stage
    /// when LSN and prev_hash are managed externally (e.g., by PipelineContext).
    /// Returns the 64-byte Ed25519 signature.
    pub fn sign_bytes(&self, data: &[u8]) -> [u8; 64] {
        self.signing_key.sign(data).to_bytes()
    }

    /// Verify a record's signature using the given public key.
    pub fn verify_record(
        verifying_key: &VerifyingKey,
        record: &Record,
    ) -> Result<(), SignatureError> {
        let sig_bytes: [u8; 64] = record.signature;
        let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);

        let data = Self::build_signing_payload_static(record);
        verifying_key
            .verify(&data, &signature)
            .map_err(|_| SignatureError::InvalidSignature)
    }

    /// Verify the prev_hash chain for two consecutive records.
    pub fn verify_chain_link(prev: &Record, next: &Record) -> Result<(), SignatureError> {
        // Check LSN monotonicity
        if next.lsn <= prev.lsn {
            return Err(SignatureError::LsnRegression);
        }

        let mut hasher = Sha256::new();
        hasher.update(prev.lsn.to_le_bytes());
        hasher.update(prev.signature);
        let expected: [u8; 32] = hasher.finalize().into();

        if expected != next.prev_hash {
            return Err(SignatureError::ChainBroken);
        }
        Ok(())
    }

    /// Build the payload to be signed for a given record.
    fn build_signing_payload(&self, record: &Record) -> Vec<u8> {
        Self::build_signing_payload_static(record)
    }

    pub(crate) fn build_signing_payload_static(record: &Record) -> Vec<u8> {
        let mut data = Vec::with_capacity(256);

        // Ring 0: id, timestamp (exclude signature and origin_lsn for simplicity in M2)
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

    /// Get the current LSN (next to be assigned).
    pub fn current_lsn(&self) -> u64 {
        self.lsn_counter.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Compute the chain hash for a signed record.
    ///
    /// This is `SHA-256(lsn || signature)` — the same hash stored as
    /// `prev_hash` on the **next** record in the chain.  Used by
    /// [`ExternalAnchor`](crate::security::ExternalAnchor) to
    /// accumulate record hashes between anchors.
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
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "Invalid Ed25519 signature"),
            Self::ChainBroken => write!(f, "LSN chain broken: prev_hash mismatch"),
            Self::LsnRegression => write!(f, "LSN regression detected"),
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
        r.id.hi = 0;
        r.id.lo = id;
        r.level = LogLevel::Audit;
        r.message.set("test audit message");
        r.thread_id = 1;
        r.process_id = 1234;
        r
    }

    #[test]
    fn test_sign_and_verify() {
        let engine = SignatureEngine::new();
        let mut record = make_test_record(1);

        let sig = engine.sign_record(&mut record);
        record.signature = sig;
        assert!(record.lsn > 0);

        let result = SignatureEngine::verify_record(&engine.verifying_key, &record);
        assert!(result.is_ok());
    }

    #[test]
    fn test_chain_continuity() {
        let engine = SignatureEngine::new();

        let mut r1 = make_test_record(1);
        let sig1 = engine.sign_record(&mut r1);
        r1.signature = sig1;

        let mut r2 = make_test_record(2);
        let sig2 = engine.sign_record(&mut r2);
        r2.signature = sig2;

        // r2.prev_hash should match SHA-256(r1.lsn || r1.signature)
        let result = SignatureEngine::verify_chain_link(&r1, &r2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_lsn_monotonic() {
        let engine = SignatureEngine::new();

        let mut r1 = make_test_record(1);
        engine.sign_record(&mut r1);

        let mut r2 = make_test_record(2);
        engine.sign_record(&mut r2);

        assert!(r2.lsn > r1.lsn);
    }

    #[test]
    fn test_tampered_record_fails_verification() {
        let engine = SignatureEngine::new();
        let mut record = make_test_record(1);

        let sig = engine.sign_record(&mut record);
        record.signature = sig;

        // Tamper with the message
        record.message.set("tampered message");
        record.signature = sig; // old signature

        let result = SignatureEngine::verify_record(&engine.verifying_key, &record);
        assert!(result.is_err());
    }
}
