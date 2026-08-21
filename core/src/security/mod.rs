//! Cryptographic operations and security infrastructure.
//!
//! Contains the signature engine, key management, key rotation,
//! external anchoring, secret detection, and CRC32C checksum.

pub mod audit;
pub mod crc32c;
pub mod external_anchor;
pub mod key_provider;
pub mod key_rotation;
pub mod os_random;
pub mod replay_guard;
pub mod secret_detector;
pub mod signature;
pub mod tpm;

pub use audit::{AuditPipeline, DEFAULT_AUDIT_BUFFER_RATIO};
pub use crc32c::{crc32c, crc32c_ring3, crc32c_update};
pub use external_anchor::{AnchorRecord, ExternalAnchor};
pub use key_provider::{DefaultKeyProvider, KeyError, KeyResult, SigningProvider};
pub use key_rotation::{
    fingerprint_key, CrlEntry, CrlReason, KeyFingerprint, KeyRotationManager, RotationError,
    RotationEvent, RotationResult, RotationStatus,
};
pub use os_random::{fill_bytes as os_random_fill_bytes, OsRandomError};
pub use replay_guard::{ReplayDecision, ReplayGuard, ReplayPolicy, ReplayStats};
pub use secret_detector::{DetectionResult, Finding, RuleAction, SecretDetector};
pub use signature::{SignatureEngine, SignatureError};
pub use tpm::{TpmCapabilities, TpmError, TpmKeyProvider, TpmPhase2Feature, TpmResult};
