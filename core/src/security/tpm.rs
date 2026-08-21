//! TPM-backed signing boundary for the audit key provider.
//!
//! This module defines the phase-1 contract without pretending that a software
//! key is equivalent to a TPM key. The provider is intentionally explicit:
//! callers request TPM mode, probe the platform, and either receive a hardware
//! signing handle or a typed refusal. PCR measurement, attestation, and a
//! monotonic counter are represented as phase-2 status methods so the missing
//! capabilities cannot be silently forgotten during integration.

use std::fmt;
use std::path::{Path, PathBuf};

/// Hardware capability summary returned by [`TpmKeyProvider::probe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TpmCapabilities {
    /// Whether a platform TPM API is available.
    pub platform_backend: bool,
    /// Whether phase-1 non-exportable signing is available.
    pub hardware_signing: bool,
    /// Whether PCR measurement is implemented.
    pub pcr_measurement: bool,
    /// Whether remote attestation is implemented.
    pub attestation: bool,
    /// Whether hardware monotonic counters are implemented.
    pub monotonic_counter: bool,
}

impl TpmCapabilities {
    /// Return the capability set used before a provider is opened.
    pub const fn unavailable() -> Self {
        Self {
            platform_backend: false,
            hardware_signing: false,
            pcr_measurement: false,
            attestation: false,
            monotonic_counter: false,
        }
    }

    /// Return a compact capability label for diagnostics.
    pub fn summary(&self) -> String {
        format!(
            "backend={},hardware_signing={},pcr={},attestation={},counter={}",
            self.platform_backend,
            self.hardware_signing,
            self.pcr_measurement,
            self.attestation,
            self.monotonic_counter,
        )
    }
    /// Whether the phase-1 contract can be satisfied.
    pub const fn phase1_ready(&self) -> bool {
        self.platform_backend && self.hardware_signing
    }
}

/// Phase-2 features that are deliberately not part of the phase-1 API yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmPhase2Feature {
    /// Platform configuration register measurement.
    PcrMeasurement,
    /// Evidence generation for a remote verifier.
    Attestation,
    /// Hardware monotonic counter allocation.
    MonotonicCounter,
}

/// Errors from the explicit TPM boundary.
#[derive(Debug)]
pub enum TpmError {
    /// The requested provider has not been opened.
    NotOpen,
    /// The current platform has no implemented TPM backend.
    UnsupportedPlatform,
    /// The selected device path is invalid or unavailable.
    InvalidDevice(PathBuf),
    /// The phase-2 feature is reserved but not implemented.
    Phase2NotImplemented(TpmPhase2Feature),
    /// The hardware operation failed.
    Hardware(String),
}

impl fmt::Display for TpmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOpen => write!(f, "TPM key provider is not open"),
            Self::UnsupportedPlatform => {
                write!(f, "no TPM signing backend is implemented on this platform")
            }
            Self::InvalidDevice(path) => write!(f, "TPM device is invalid: {}", path.display()),
            Self::Phase2NotImplemented(feature) => {
                write!(f, "TPM phase-2 feature is not implemented: {feature:?}")
            }
            Self::Hardware(error) => write!(f, "TPM hardware operation failed: {error}"),
        }
    }
}

impl std::error::Error for TpmError {}

/// Result type for TPM operations.
pub type TpmResult<T> = Result<T, TpmError>;

/// Security policy for selecting the TPM provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TpmPolicy {
    /// Refuse startup when the hardware backend is unavailable.
    pub require_hardware: bool,
    /// Permit the caller to request phase-2 operations after opening.
    pub allow_phase2_requests: bool,
}

impl Default for TpmPolicy {
    fn default() -> Self {
        Self {
            require_hardware: true,
            allow_phase2_requests: false,
        }
    }
}

impl TpmPolicy {
    /// Validate the policy against the provider's current capabilities.
    pub fn validate(&self, capabilities: &TpmCapabilities) -> TpmResult<()> {
        if self.require_hardware && !capabilities.phase1_ready() {
            return Err(TpmError::UnsupportedPlatform);
        }
        Ok(())
    }

    /// Return whether a phase-2 request is allowed by policy.
    pub const fn phase2_allowed(&self) -> bool {
        self.allow_phase2_requests
    }
}
/// Non-exportable TPM key provider contract.
///
/// The provider owns an opaque key handle and never exposes private key bytes.
/// Platform-specific backends can be added behind this type without changing
/// the signing pipeline or the audit sidecar format.
pub struct TpmKeyProvider {
    device: Option<PathBuf>,
    capabilities: TpmCapabilities,
    open: bool,
    key_handle: Option<String>,
}

impl TpmKeyProvider {
    /// Construct a provider targeting the platform default TPM device.
    pub fn new(device: Option<PathBuf>) -> Self {
        Self {
            device,
            capabilities: TpmCapabilities::unavailable(),
            open: false,
            key_handle: None,
        }
    }

    /// Construct a provider from a string path without accepting empty paths.
    pub fn from_device(path: impl AsRef<Path>) -> TpmResult<Self> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(TpmError::InvalidDevice(path.to_path_buf()));
        }
        Ok(Self::new(Some(path.to_path_buf())))
    }

    /// Inspect platform support without opening a key.
    pub fn probe(&self) -> TpmCapabilities {
        platform::probe()
    }

    /// Validate an explicit policy before opening the provider.
    pub fn open_with_policy(&mut self, policy: TpmPolicy) -> TpmResult<()> {
        let capabilities = self.probe();
        policy.validate(&capabilities)?;
        self.open()
    }
    /// Open the hardware provider.
    ///
    /// This deliberately returns `UnsupportedPlatform` until a reviewed CNG,
    /// tpm2-tss, or Secure Enclave backend is supplied. It never falls back to
    /// [`crate::security::DefaultKeyProvider`].
    pub fn open(&mut self) -> TpmResult<()> {
        let _device = self.device.as_deref();
        self.capabilities = platform::probe();
        if !self.capabilities.phase1_ready() {
            return Err(TpmError::UnsupportedPlatform);
        }
        self.key_handle = Some(platform::open_key(self.device.as_deref())?);
        self.open = true;
        Ok(())
    }

    /// Close the opaque hardware handle.
    pub fn close(&mut self) {
        if let Some(handle) = self.key_handle.take() {
            platform::close_key(&handle);
        }
        self.open = false;
    }

    /// Whether the provider currently owns an open hardware handle.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Return the configured device, if one was supplied.
    pub fn device(&self) -> Option<&Path> {
        self.device.as_deref()
    }

    /// Sign bytes with the non-exportable TPM key.
    pub fn sign(&self, data: &[u8]) -> TpmResult<[u8; 64]> {
        if !self.open {
            return Err(TpmError::NotOpen);
        }
        let handle = self.key_handle.as_deref().ok_or(TpmError::NotOpen)?;
        platform::sign(handle, data)
    }

    /// Return the phase-2 status for a requested feature.
    pub fn phase2_status(&self, feature: TpmPhase2Feature) -> TpmResult<()> {
        match feature {
            TpmPhase2Feature::PcrMeasurement
            | TpmPhase2Feature::Attestation
            | TpmPhase2Feature::MonotonicCounter => Err(TpmError::Phase2NotImplemented(feature)),
        }
    }
}

impl Drop for TpmKeyProvider {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(any(windows, unix))]
mod platform {
    use super::{TpmCapabilities, TpmError, TpmResult};
    use std::path::Path;

    pub(super) fn probe() -> TpmCapabilities {
        // The platform hook is intentionally visible but conservative. A
        // backend must be implemented and reviewed before this becomes true.
        TpmCapabilities::unavailable()
    }

    pub(super) fn open_key(_device: Option<&Path>) -> TpmResult<String> {
        Err(TpmError::UnsupportedPlatform)
    }

    pub(super) fn close_key(_handle: &str) {}

    pub(super) fn sign(_handle: &str, _data: &[u8]) -> TpmResult<[u8; 64]> {
        Err(TpmError::UnsupportedPlatform)
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use super::{TpmCapabilities, TpmError, TpmResult};
    use std::path::Path;

    pub(super) fn probe() -> TpmCapabilities {
        TpmCapabilities::unavailable()
    }

    pub(super) fn open_key(_device: Option<&Path>) -> TpmResult<String> {
        Err(TpmError::UnsupportedPlatform)
    }

    pub(super) fn close_key(_handle: &str) {}

    pub(super) fn sign(_handle: &str, _data: &[u8]) -> TpmResult<[u8; 64]> {
        Err(TpmError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_summary_is_machine_readable() {
        assert_eq!(
            TpmCapabilities::unavailable().summary(),
            "backend=false,hardware_signing=false,pcr=false,attestation=false,counter=false"
        );
    }
    #[test]
    fn unavailable_provider_never_claims_phase1_ready() {
        let provider = TpmKeyProvider::new(None);
        assert!(!provider.probe().phase1_ready());
        assert!(!provider.is_open());
    }

    #[test]
    fn default_policy_requires_phase1_hardware() {
        let policy = TpmPolicy::default();
        assert!(policy.require_hardware);
        assert!(!policy.phase2_allowed());
        assert!(policy.validate(&TpmCapabilities::unavailable()).is_err());
    }
    #[test]
    fn phase2_features_are_explicitly_rejected() {
        let provider = TpmKeyProvider::new(None);
        assert!(matches!(
            provider.phase2_status(TpmPhase2Feature::Attestation),
            Err(TpmError::Phase2NotImplemented(
                TpmPhase2Feature::Attestation
            ))
        ));
    }

    #[test]
    fn empty_device_is_rejected() {
        assert!(matches!(
            TpmKeyProvider::from_device(""),
            Err(TpmError::InvalidDevice(_))
        ));
    }

    #[test]
    fn unsupported_open_does_not_silently_downgrade() {
        let mut provider = TpmKeyProvider::new(None);
        let result = provider.open();
        assert!(result.is_err());
        assert!(!provider.is_open());
    }
}
