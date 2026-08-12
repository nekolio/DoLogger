//! Cryptographic operations and security infrastructure.
//!
//! Contains the signature engine, key management, key rotation,
//! external anchoring, secret detection, and CRC32C checksum.

pub mod crc32c;
pub mod external_anchor;
pub mod key_provider;
pub mod key_rotation;
pub mod secret_detector;
pub mod signature;

pub use crc32c::{crc32c, crc32c_ring3, crc32c_update};
pub use external_anchor::{AnchorRecord, ExternalAnchor};
pub use key_provider::{DefaultKeyProvider, KeyError, KeyResult};
pub use key_rotation::{
    fingerprint_key, CrlEntry, CrlReason, KeyFingerprint, KeyRotationManager, RotationError,
    RotationEvent, RotationResult, RotationStatus,
};
pub use secret_detector::{DetectionResult, Finding, RuleAction, SecretDetector};
pub use signature::{SignatureEngine, SignatureError};
