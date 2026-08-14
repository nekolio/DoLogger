//! Public key rotation and revocation mechanism.
//!
//! Supports:
//! - Multi-key parallel verification (any active key = valid)
//! - Key rotation lifecycle (initiate -> grace period -> complete)
//! - Certificate Revocation List (CRL)
//! - Key fingerprinting via SHA-256
//!
//! # Design
//!
//! During key rotation, two active keys coexist for a configurable grace
//! period (default 7 days). Records signed by either key are accepted,
//! allowing in-flight records to remain valid while the system transitions
//! to the new key.
//!
//! # Revocation
//!
//! Keys can be permanently revoked via the CRL. Once revoked, a key's
//! fingerprint is added to the denied set and any record signed by that
//! key fails verification regardless of cryptographic validity.

// TODO: Remove #![allow(missing_docs)] and add doc comments to remaining public
// items. Most types already have struct-level docs but field-level docs are
// missing (e.g., KeyRotationManager fields, CrlReason variants).
#![allow(missing_docs)]

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use crate::record::Record;
use crate::security::SignatureError;

// ---------------------------------------------------------------------------
// Key Fingerprint
// ---------------------------------------------------------------------------

/// SHA-256 fingerprint of a public key (32 bytes).
///
/// Used as a stable, collision-resistant identifier for key revocation
/// and rotation audit trails. The fingerprint is computed from the raw
/// public key bytes and is independent of the serialisation format.
pub type KeyFingerprint = [u8; 32];

/// Compute the SHA-256 fingerprint of a verifying key.
pub fn fingerprint_key(vk: &VerifyingKey) -> KeyFingerprint {
    let mut hasher = Sha256::new();
    hasher.update(vk.as_bytes());
    hasher.finalize().into()
}

// ---------------------------------------------------------------------------
// CrlReason
// ---------------------------------------------------------------------------

/// Reason for key revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrlReason {
    /// Key has been compromised (emergency — sysmon CRITICAL).
    Compromised,
    /// Key has been replaced by a newer key after a successful rotation.
    Superseded,
    /// Key has been administratively deactivated (not compromised).
    Deactivated,
}

impl CrlReason {
    /// Human-readable string for the revocation reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compromised => "compromised",
            Self::Superseded => "superseded",
            Self::Deactivated => "deactivated",
        }
    }

    /// Parse a CrlReason from its string representation.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "compromised" => Some(Self::Compromised),
            "superseded" => Some(Self::Superseded),
            "deactivated" => Some(Self::Deactivated),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// CrlEntry
// ---------------------------------------------------------------------------

/// A single entry in the Certificate Revocation List.
///
/// Each entry records a permanently revoked key with the reason
/// and timestamp of revocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrlEntry {
    /// SHA-256 fingerprint of the revoked key.
    pub fingerprint: KeyFingerprint,
    /// UNIX timestamp (seconds) when the key was revoked.
    pub revoked_at: u64,
    /// Reason for revocation.
    pub reason: CrlReason,
}

// ---------------------------------------------------------------------------
// RotationStatus
// ---------------------------------------------------------------------------

/// Current status of a key rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationStatus {
    /// Rotation is in its grace period — both old and new keys are active.
    InProgress,
    /// Rotation completed successfully — old key retired.
    Completed,
    /// Rotation was cancelled — reverted to previous state.
    Cancelled,
}

// ---------------------------------------------------------------------------
// RotationEvent
// ---------------------------------------------------------------------------

/// A key rotation event recorded in the audit trail.
#[derive(Debug, Clone)]
pub struct RotationEvent {
    /// Fingerprint of the old (retiring) key, if any.
    pub old_fingerprint: Option<KeyFingerprint>,
    /// Fingerprint of the new (incoming) key.
    pub new_fingerprint: KeyFingerprint,
    /// UNIX timestamp (seconds) when the rotation was initiated.
    pub initiated_at: u64,
    /// Current status of the rotation.
    pub status: RotationStatus,
}

// ---------------------------------------------------------------------------
// RotationError
// ---------------------------------------------------------------------------

/// Errors from key rotation operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotationError {
    /// No rotation is currently in progress.
    NoRotationInProgress,
    /// A rotation is already in progress — complete or cancel it first.
    RotationAlreadyInProgress,
    /// The specified key fingerprint was not found among active keys.
    KeyNotFound,
    /// Refusing to revoke the only active key (would break verification).
    CannotRevokeOnlyKey,
    /// The grace period has not elapsed — cannot complete rotation yet.
    GracePeriodNotElapsed,
}

impl std::fmt::Display for RotationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRotationInProgress => write!(f, "No key rotation in progress"),
            Self::RotationAlreadyInProgress => {
                write!(f, "Key rotation already in progress")
            }
            Self::KeyNotFound => write!(f, "Key fingerprint not found among active keys"),
            Self::CannotRevokeOnlyKey => {
                write!(f, "Cannot revoke the only active key")
            }
            Self::GracePeriodNotElapsed => {
                write!(f, "Grace period has not elapsed")
            }
        }
    }
}

/// Result type for key rotation operations.
pub type RotationResult<T> = Result<T, RotationError>;

// ---------------------------------------------------------------------------
// KeyRotationManager
// ---------------------------------------------------------------------------

/// Manages key rotation and revocation for the DoLogger signature subsystem.
///
/// # Requirements
///
/// - **Multi-key parallel verification**: Records are validated against all
///   active keys simultaneously. Any match succeeds.
/// - **Rotation lifecycle**: initiate -> grace period -> complete (or cancel).
/// - **CRL**: Permanently revoked keys are tracked and enforced at
///   verification time.
/// - **Audit trail**: Every rotation event is recorded in `rotation_history`.
///
/// # Thread safety
///
/// This type does not provide internal synchronisation. The caller is
/// responsible for wrapping it in `Arc<Mutex<>>` if concurrent access
/// is required.
pub struct KeyRotationManager {
    /// Currently active verifying keys. Normally contains 1 key,
    /// 2 during a rotation grace period.
    pub active_keys: Vec<VerifyingKey>,
    /// Signing keys, parallel to `active_keys`. `active_signing_keys[i]`
    /// corresponds to `active_keys[i]`.
    active_signing_keys: Vec<SigningKey>,
    /// Index of the key currently used for NEW signatures.
    pub primary_key_index: usize,
    /// Audit trail of all key rotation events.
    pub rotation_history: Vec<RotationEvent>,
    /// Permanently revoked key fingerprints (denied set).
    pub revoked_keys: HashSet<KeyFingerprint>,
    /// Certificate Revocation List entries.
    pub crl: Vec<CrlEntry>,
    /// Grace period in days before a rotation can be completed.
    grace_period_days: u32,
}

impl KeyRotationManager {
    /// Create a new `KeyRotationManager` with a single initial key pair.
    ///
    /// The manager starts with one active key and no rotation history.
    ///
    /// # Parameters
    ///
    /// - `signing_key`: Initial Ed25519 signing key (from KeyProvider or
    ///   SignatureEngine).
    /// - `grace_period_days`: Grace period for key rotations (typically from
    ///   `DologgerConfig::key_rotation_grace_period_days`).
    pub fn new(signing_key: SigningKey, grace_period_days: u32) -> Self {
        let verifying_key = signing_key.verifying_key();
        Self {
            active_keys: vec![verifying_key],
            active_signing_keys: vec![signing_key],
            primary_key_index: 0,
            rotation_history: Vec::new(),
            revoked_keys: HashSet::new(),
            crl: Vec::new(),
            grace_period_days,
        }
    }

    // ------------------------------------------------------------------
    // Active key queries
    // ------------------------------------------------------------------

    /// Number of currently active verifying keys.
    ///
    /// Normally 1; 2 during a rotation grace period.
    pub fn active_key_count(&self) -> usize {
        self.active_keys.len()
    }

    /// Get the primary signing key (used for new signatures).
    pub fn primary_signing_key(&self) -> &SigningKey {
        &self.active_signing_keys[self.primary_key_index]
    }

    /// Get the primary verifying key.
    pub fn primary_verifying_key(&self) -> &VerifyingKey {
        &self.active_keys[self.primary_key_index]
    }

    /// Get the fingerprint of the primary key.
    pub fn primary_fingerprint(&self) -> KeyFingerprint {
        fingerprint_key(&self.active_keys[self.primary_key_index])
    }

    /// Check whether a rotation is currently in progress.
    pub fn rotation_in_progress(&self) -> bool {
        self.active_keys.len() > 1
    }

    // ------------------------------------------------------------------
    // Rotation lifecycle
    // ------------------------------------------------------------------

    /// Initiate a key rotation.
    ///
    /// Generates a fresh Ed25519 key pair, adds it to the active set,
    /// and records a `RotationEvent` in the audit trail. After this
    /// call, `active_key_count()` returns 2 and `primary_key_index`
    /// points to the new key.
    ///
    /// # Errors
    ///
    /// Returns `RotationAlreadyInProgress` if a rotation is already active.
    pub fn initiate_rotation(&mut self) -> RotationResult<RotationEvent> {
        if self.rotation_in_progress() {
            return Err(RotationError::RotationAlreadyInProgress);
        }

        // Generate a fresh key pair
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let new_signing_key = SigningKey::from_bytes(&seed);
        let new_verifying_key = new_signing_key.verifying_key();
        let new_fingerprint = fingerprint_key(&new_verifying_key);

        let old_fingerprint = fingerprint_key(&self.active_keys[self.primary_key_index]);

        // Record the rotation event
        let now = current_unix_timestamp();
        let event = RotationEvent {
            old_fingerprint: Some(old_fingerprint),
            new_fingerprint,
            initiated_at: now,
            status: RotationStatus::InProgress,
        };

        // Add the new key
        self.active_signing_keys.push(new_signing_key);
        self.active_keys.push(new_verifying_key);
        self.primary_key_index = self.active_keys.len() - 1;

        self.rotation_history.push(event.clone());

        crate::sys::diagnostics::info(
            "key-rotation",
            &format!(
                "Key rotation initiated: new key fingerprint {} (grace period: {} days)",
                hex::encode(new_fingerprint),
                self.grace_period_days
            ),
        );

        Ok(event)
    }

    /// Complete an in-progress rotation.
    ///
    /// Removes the old (non-primary) key from the active set and marks
    /// the rotation event as `Completed`. The old key is added to the
    /// CRL with reason `Superseded`.
    ///
    /// # Errors
    ///
    /// - `NoRotationInProgress` if only one key is active.
    /// - `GracePeriodNotElapsed` if the configured grace period has not
    ///   passed since the rotation was initiated.
    pub fn complete_rotation(&mut self) -> RotationResult<KeyFingerprint> {
        if !self.rotation_in_progress() {
            return Err(RotationError::NoRotationInProgress);
        }

        // Check grace period
        let rotation_event = self
            .rotation_history
            .iter()
            .rev()
            .find(|e| e.status == RotationStatus::InProgress)
            .ok_or(RotationError::NoRotationInProgress)?;

        let now = current_unix_timestamp();
        let grace_period_secs = self.grace_period_days as u64 * 86400;

        if now < rotation_event.initiated_at + grace_period_secs {
            return Err(RotationError::GracePeriodNotElapsed);
        }

        // Identify the old key (the one that is NOT primary)
        let old_index = if self.primary_key_index == 0 { 1 } else { 0 };
        let old_vk = self.active_keys[old_index];
        let old_fingerprint = fingerprint_key(&old_vk);

        // Add old key to CRL as Superseded
        let crl_entry = CrlEntry {
            fingerprint: old_fingerprint,
            revoked_at: now,
            reason: CrlReason::Superseded,
        };
        self.crl.push(crl_entry);
        self.revoked_keys.insert(old_fingerprint);

        // Remove old key from active sets
        self.active_keys.remove(old_index);
        self.active_signing_keys.remove(old_index);

        // Adjust primary index if we removed a key before it
        if old_index < self.primary_key_index {
            self.primary_key_index -= 1;
        }

        // Update rotation event status
        if let Some(event) = self
            .rotation_history
            .iter_mut()
            .rev()
            .find(|e| e.status == RotationStatus::InProgress)
        {
            event.status = RotationStatus::Completed;
        }

        crate::sys::diagnostics::info(
            "key-rotation",
            &format!(
                "Key rotation completed: old key {} retired, new key {} now sole active",
                hex::encode(old_fingerprint),
                hex::encode(self.primary_fingerprint())
            ),
        );

        Ok(old_fingerprint)
    }

    /// Cancel an in-progress rotation.
    ///
    /// Removes the new (primary) key from the active set, reverts the
    /// primary index to the old key, and marks the rotation event as
    /// `Cancelled`.
    ///
    /// # Errors
    ///
    /// Returns `NoRotationInProgress` if only one key is active.
    pub fn cancel_rotation(&mut self) -> RotationResult<KeyFingerprint> {
        if !self.rotation_in_progress() {
            return Err(RotationError::NoRotationInProgress);
        }

        // Identify the new key (the primary one — about to be removed)
        let new_vk = self.active_keys[self.primary_key_index];
        let new_fingerprint = fingerprint_key(&new_vk);

        let old_index = if self.primary_key_index == 0 { 1 } else { 0 };
        let old_fingerprint = fingerprint_key(&self.active_keys[old_index]);

        // Remove the new key
        self.active_keys.remove(self.primary_key_index);
        self.active_signing_keys.remove(self.primary_key_index);

        // Revert to old key as primary
        self.primary_key_index = 0;

        // Update rotation event status
        if let Some(event) = self
            .rotation_history
            .iter_mut()
            .rev()
            .find(|e| e.status == RotationStatus::InProgress)
        {
            event.status = RotationStatus::Cancelled;
        }

        crate::sys::diagnostics::warn(
            "key-rotation",
            &format!(
                "Key rotation cancelled: new key {} removed, reverted to {}",
                hex::encode(new_fingerprint),
                hex::encode(old_fingerprint)
            ),
        );

        Ok(new_fingerprint)
    }

    // ------------------------------------------------------------------
    // Revocation
    // ------------------------------------------------------------------

    /// Revoke a key by its fingerprint.
    ///
    /// Adds the key to the denied set and CRL, and removes it from the
    /// active set if present. This is an **emergency** operation and
    /// logs at CRITICAL level via both `diag` and `sysmon`.
    ///
    /// # Safety
    ///
    /// Revoking the only active key is refused — at least one active key
    /// must remain for the system to function.
    ///
    /// # Errors
    ///
    /// - `CannotRevokeOnlyKey` if this is the sole active key.
    /// - `KeyNotFound` if the fingerprint does not match any active key
    ///   (revoking a non-active key is still permitted for CRL hygiene).
    pub fn revoke_key(
        &mut self,
        fingerprint: KeyFingerprint,
        reason: CrlReason,
    ) -> RotationResult<()> {
        // Prevent revoking the only active key
        if self.active_keys.len() == 1 && fingerprint_key(&self.active_keys[0]) == fingerprint {
            return Err(RotationError::CannotRevokeOnlyKey);
        }

        let now = current_unix_timestamp();

        // Add to CRL
        let crl_entry = CrlEntry {
            fingerprint,
            revoked_at: now,
            reason,
        };
        self.crl.push(crl_entry);

        // Add to denied set
        self.revoked_keys.insert(fingerprint);

        // Remove from active keys if present
        if let Some(pos) = self
            .active_keys
            .iter()
            .position(|vk| fingerprint_key(vk) == fingerprint)
        {
            self.active_keys.remove(pos);
            self.active_signing_keys.remove(pos);
            if pos < self.primary_key_index {
                self.primary_key_index -= 1;
            } else if pos == self.primary_key_index && !self.active_keys.is_empty() {
                self.primary_key_index = 0;
            }
        }

        // Emergency log at CRITICAL level
        crate::sys::diagnostics::critical(
            "key-rotation",
            &format!(
                "KEY REVOKED: fingerprint {} reason {}",
                hex::encode(fingerprint),
                reason.as_str()
            ),
        );

        Ok(())
    }

    /// Check whether a key fingerprint has been revoked.
    pub fn is_revoked(&self, fingerprint: &KeyFingerprint) -> bool {
        self.revoked_keys.contains(fingerprint)
    }

    /// Export the CRL as a JSON byte vector.
    ///
    /// The output is a JSON array of objects, each containing the
    /// fingerprint (hex-encoded), revocation timestamp, and reason.
    /// This format is suitable for external verification tools.
    pub fn export_crl(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct CrlJsonEntry {
            fingerprint: String,
            revoked_at: u64,
            reason: String,
        }

        let entries: Vec<CrlJsonEntry> = self
            .crl
            .iter()
            .map(|e| CrlJsonEntry {
                fingerprint: hex::encode(e.fingerprint),
                revoked_at: e.revoked_at,
                reason: e.reason.as_str().to_string(),
            })
            .collect();

        // serde_json::to_vec returns Vec<u8>; unwrap is safe because
        // all fields are simple primitive types that cannot fail to serialize.
        serde_json::to_vec_pretty(&entries).unwrap_or_else(|_| b"[]".to_vec())
    }

    /// Import CRL entries from a JSON byte slice.
    ///
    /// Parses the JSON array and validates fingerprint lengths.
    /// Unknown reason strings are mapped to `Deactivated`.
    pub fn import_crl(data: &[u8]) -> Result<Vec<CrlEntry>, String> {
        #[derive(Deserialize)]
        struct CrlJsonEntry {
            fingerprint: String,
            revoked_at: u64,
            reason: String,
        }

        let raw_entries: Vec<CrlJsonEntry> =
            serde_json::from_slice(data).map_err(|e| format!("CRL JSON parse error: {e}"))?;

        let mut entries = Vec::with_capacity(raw_entries.len());
        for raw in raw_entries {
            let fp_bytes = hex::decode(&raw.fingerprint)
                .map_err(|e| format!("Invalid fingerprint hex '{}': {e}", raw.fingerprint))?;
            let fp_len = fp_bytes.len();
            let fp: KeyFingerprint = fp_bytes
                .as_slice()
                .try_into()
                .map_err(|_| format!("Fingerprint must be 32 bytes, got {fp_len}"))?;

            let reason = CrlReason::parse(&raw.reason).unwrap_or(CrlReason::Deactivated);

            entries.push(CrlEntry {
                fingerprint: fp,
                revoked_at: raw.revoked_at,
                reason,
            });
        }

        Ok(entries)
    }

    // ------------------------------------------------------------------
    // Multi-key verification
    // ------------------------------------------------------------------

    /// Verify a record against all active keys.
    ///
    /// Tries each active verifying key in order. The first successful
    /// verification returns `Ok(index)` where `index` is the position
    /// of the matching key in `active_keys`. Revoked keys are skipped
    /// before verification is attempted.
    ///
    /// # Returns
    ///
    /// - `Ok(index)`: The record is valid and was signed by `active_keys[index]`.
    /// - `Err(SignatureError)`: No active non-revoked key verified the record.
    pub fn verify_record_multi(&self, record: &Record) -> Result<usize, SignatureError> {
        let sig_bytes: [u8; 64] = record.signature;
        let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        let data = crate::security::SignatureEngine::build_signing_payload_static(record);

        for (idx, vk) in self.active_keys.iter().enumerate() {
            // Skip revoked keys
            if self.is_revoked(&fingerprint_key(vk)) {
                continue;
            }
            if vk.verify(&data, &signature).is_ok() {
                return Ok(idx);
            }
        }

        Err(SignatureError::InvalidSignature)
    }

    /// Sign the payload for a record using the primary signing key.
    ///
    /// Returns the 64-byte Ed25519 signature.
    pub fn sign_record(&self, record: &Record) -> [u8; 64] {
        let data = crate::security::SignatureEngine::build_signing_payload_static(record);
        self.primary_signing_key().sign(&data).to_bytes()
    }

    /// Sign arbitrary bytes with the primary signing key.
    ///
    /// Returns the 64-byte Ed25519 signature.
    pub fn sign_bytes(&self, data: &[u8]) -> [u8; 64] {
        self.primary_signing_key().sign(data).to_bytes()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the current UNIX timestamp in seconds.
fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{LogLevel, Record};
    use ed25519_dalek::SigningKey;
    use rand::RngCore;

    /// Helper: generate a new random Ed25519 signing key.
    fn generate_key() -> SigningKey {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    /// Helper: create a simple test record.
    fn make_test_record(id: u64) -> Record {
        let mut r = Record::new(0);
        r.id.hi = 0;
        r.id.lo = id;
        r.level = LogLevel::Audit;
        r.message.set("test audit message for key rotation");
        r.thread_id = 1;
        r.process_id = 1234;
        r
    }

    /// Single key verification.
    #[test]
    fn test_single_key_verify() {
        let sk = generate_key();
        let manager = KeyRotationManager::new(sk.clone(), 7);
        assert_eq!(manager.active_key_count(), 1);

        let mut record = make_test_record(1);
        let sig = manager.sign_record(&record);
        record.signature = sig;

        let result = manager.verify_record_multi(&record);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    /// Multi-key verification — records signed by either active key
    /// are valid.
    #[test]
    fn test_multi_key_verify() {
        let sk_old = generate_key();
        let mut manager = KeyRotationManager::new(sk_old.clone(), 0); // grace=0 for instant complete
        assert_eq!(manager.active_key_count(), 1);

        // Sign a record with the old key before rotation
        let mut old_record = make_test_record(1);
        let old_sig = manager.sign_record(&old_record);
        old_record.signature = old_sig;

        // Initiate rotation
        let event = manager.initiate_rotation().unwrap();
        assert_eq!(manager.active_key_count(), 2);
        assert_eq!(event.status, RotationStatus::InProgress);

        // Sign a record with the new key
        let mut new_record = make_test_record(2);
        let new_sig = manager.sign_record(&new_record);
        new_record.signature = new_sig;

        // Both records should verify (2 active keys)
        let old_result = manager.verify_record_multi(&old_record);
        assert!(old_result.is_ok());
        // old record should match key at index 0 (old key)
        assert_eq!(old_result.unwrap(), 0);

        let new_result = manager.verify_record_multi(&new_record);
        assert!(new_result.is_ok());
        // new record should match key at index 1 (new key = primary)
        assert_eq!(new_result.unwrap(), 1);

        // Complete rotation (grace_period_days=0 so no delay)
        let _retired = manager.complete_rotation().unwrap();
        assert_eq!(manager.active_key_count(), 1);

        // Old record should STILL verify (old key is Superseded, not revoked)
        // Actually after complete, old key is added to CRL with Superseded
        // and removed from active_keys. Let's check if old record still verifies...
        // The old key is removed from active_keys, so old record should FAIL.
        let old_result_after = manager.verify_record_multi(&old_record);
        assert!(
            old_result_after.is_err(),
            "Old record should fail after rotation complete because old key is no longer active"
        );

        // New record should still verify
        let new_result_after = manager.verify_record_multi(&new_record);
        assert!(new_result_after.is_ok());
    }

    /// Revoking a key causes verification failure.
    #[test]
    fn test_revoke_key() {
        let sk = generate_key();
        let vk = sk.verifying_key();
        let _fp = fingerprint_key(&vk);

        // We need two keys so we can revoke one. Start a rotation,
        // then revoke the new key.
        let sk_old = generate_key();
        let mut manager = KeyRotationManager::new(sk_old.clone(), 7);

        // Sign with old key
        let mut record = make_test_record(1);
        let sig = manager.sign_record(&record);
        record.signature = sig;

        // Initiate rotation so we have 2 keys
        manager.initiate_rotation().unwrap();
        assert_eq!(manager.active_key_count(), 2);

        // Get the fingerprint of the non-primary (old) key
        let old_idx = if manager.primary_key_index == 0 { 1 } else { 0 };
        let old_fp = fingerprint_key(&manager.active_keys[old_idx]);

        // Revoke the old key
        manager.revoke_key(old_fp, CrlReason::Compromised).unwrap();

        // The old record was signed with the old key which is now revoked
        // It should fail verification
        let result = manager.verify_record_multi(&record);
        assert!(
            result.is_err(),
            "Record signed by revoked key must fail verification"
        );

        // The CRL should have one entry
        assert_eq!(manager.crl.len(), 1);
        assert_eq!(manager.crl[0].reason, CrlReason::Compromised);
        assert!(manager.is_revoked(&old_fp));
    }

    /// Full rotation lifecycle
    /// initiate -> verify both -> complete -> only new works
    #[test]
    fn test_rotation_lifecycle() {
        let sk_old = generate_key();
        let mut manager = KeyRotationManager::new(sk_old.clone(), 0); // grace=0

        assert_eq!(manager.active_key_count(), 1);
        assert!(!manager.rotation_in_progress());

        // Phase 1: Sign with old key
        let mut old_record = make_test_record(1);
        old_record.signature = manager.sign_record(&old_record);
        assert!(manager.verify_record_multi(&old_record).is_ok());

        // Phase 2: Initiate rotation
        let event = manager.initiate_rotation().unwrap();
        assert_eq!(event.status, RotationStatus::InProgress);
        assert_eq!(manager.active_key_count(), 2);
        assert!(manager.rotation_in_progress());
        assert_eq!(manager.rotation_history.len(), 1);

        // Phase 3: Both keys active -> both records verify
        let mut new_record = make_test_record(2);
        new_record.signature = manager.sign_record(&new_record);

        let old_verify = manager.verify_record_multi(&old_record);
        let new_verify = manager.verify_record_multi(&new_record);
        assert!(
            old_verify.is_ok(),
            "Old record should verify during grace period"
        );
        assert!(
            new_verify.is_ok(),
            "New record should verify during grace period"
        );

        // Phase 4: Complete rotation
        let _retired_fp = manager.complete_rotation().unwrap();
        assert_eq!(manager.active_key_count(), 1);
        assert!(!manager.rotation_in_progress());

        // Phase 5: Old record fails, new record passes
        assert!(
            manager.verify_record_multi(&old_record).is_err(),
            "Old record should fail after rotation complete"
        );
        assert!(
            manager.verify_record_multi(&new_record).is_ok(),
            "New record should pass after rotation complete"
        );

        // Rotation event should be marked Completed
        let last_event = manager.rotation_history.last().unwrap();
        assert_eq!(last_event.status, RotationStatus::Completed);
    }

    /// CRL export and import round-trip.
    #[test]
    fn test_crl_export_import() {
        let sk = generate_key();
        let mut manager = KeyRotationManager::new(sk.clone(), 7);

        // Initiate rotation to get a second key to revoke
        manager.initiate_rotation().unwrap();

        let old_idx = if manager.primary_key_index == 0 { 1 } else { 0 };
        let old_fp = fingerprint_key(&manager.active_keys[old_idx]);

        // Revoke the old key
        manager.revoke_key(old_fp, CrlReason::Compromised).unwrap();

        // Export CRL
        let json_bytes = manager.export_crl();
        let json_str = String::from_utf8(json_bytes.clone()).unwrap();
        assert!(!json_str.is_empty());
        // Should contain the fingerprint as hex
        assert!(json_str.contains(&hex::encode(old_fp)));

        // Import CRL
        let imported = KeyRotationManager::import_crl(&json_bytes).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].fingerprint, old_fp);
        assert_eq!(imported[0].reason, CrlReason::Compromised);
    }

    /// Cancel rotation reverts to single-key state.
    #[test]
    fn test_cancel_rotation() {
        let sk_old = generate_key();
        let mut manager = KeyRotationManager::new(sk_old.clone(), 7);
        assert_eq!(manager.active_key_count(), 1);

        manager.initiate_rotation().unwrap();
        assert_eq!(manager.active_key_count(), 2);

        let _cancelled_fp = manager.cancel_rotation().unwrap();
        assert_eq!(manager.active_key_count(), 1);

        let last_event = manager.rotation_history.last().unwrap();
        assert_eq!(last_event.status, RotationStatus::Cancelled);
    }

    /// Cannot revoke the only active key.
    #[test]
    fn test_cannot_revoke_only_key() {
        let sk = generate_key();
        let fp = fingerprint_key(&sk.verifying_key());
        let mut manager = KeyRotationManager::new(sk.clone(), 7);

        let result = manager.revoke_key(fp, CrlReason::Compromised);
        assert!(result.is_err());
        match result {
            Err(RotationError::CannotRevokeOnlyKey) => {} // expected
            _ => panic!("Expected CannotRevokeOnlyKey"),
        }
    }

    /// Cannot initiate rotation when one is already in progress.
    #[test]
    fn test_cannot_double_rotate() {
        let sk = generate_key();
        let mut manager = KeyRotationManager::new(sk.clone(), 7);

        manager.initiate_rotation().unwrap();
        let result = manager.initiate_rotation();
        assert!(result.is_err());
        match result {
            Err(RotationError::RotationAlreadyInProgress) => {} // expected
            _ => panic!("Expected RotationAlreadyInProgress"),
        }
    }

    /// Grace period prevents premature completion (with grace_period_days=7,
    /// use reflection to verify error, but since we can't mock time easily,
    /// just ensure the rotation flow with non-zero grace period works
    /// for initiate/cancel).
    #[test]
    fn test_grace_period_enforcement() {
        let sk = generate_key();
        // grace_period_days=365 — cannot complete for a year
        let mut manager = KeyRotationManager::new(sk.clone(), 365);

        manager.initiate_rotation().unwrap();
        let result = manager.complete_rotation();
        assert!(result.is_err());
        match result {
            Err(RotationError::GracePeriodNotElapsed) => {} // expected
            _ => panic!("Expected GracePeriodNotElapsed"),
        }

        // Cancel should still work despite grace period
        manager.cancel_rotation().unwrap();
        assert_eq!(manager.active_key_count(), 1);
    }
}
