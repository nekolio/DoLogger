//! KeyProvider — manages signing keys for DoLogger.
//!
//! # Default Implementation
//!
//! Generates a temporary Ed25519 key pair at initialization time.
//! The private key never touches disk. The public key is available
//! via the API for offline verification.
//!
//! # Planned: External KMS
//!
//! KeyProvider plugins can delegate signing to external HSM/KMS,
//! in which case the core never holds the private key.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::RngCore;

/// Result type for KeyProvider operations.
pub type KeyResult<T> = Result<T, KeyError>;

/// Errors from KeyProvider operations.
#[derive(Debug)]
pub enum KeyError {
    /// Key not initialised
    NotInitialised,
    /// Key generation failed
    GenerationFailed,
    /// Signing operation failed
    SigningFailed(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInitialised => write!(f, "KeyProvider not initialised"),
            Self::GenerationFailed => write!(f, "Key generation failed"),
            Self::SigningFailed(msg) => write!(f, "Signing failed: {msg}"),
        }
    }
}

/// Default KeyProvider — generates an ephemeral key pair in memory.
///
/// The private key never leaves process memory. On shutdown, it is
/// zeroized and the memory is freed.
pub struct DefaultKeyProvider {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    is_open: bool,
}

impl DefaultKeyProvider {
    /// Create a new default key provider with a randomly generated key pair.
    pub fn new() -> KeyResult<Self> {
        // ed25519-dalek 2.x: generate from random 32-byte seed
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        Ok(Self {
            signing_key,
            verifying_key,
            is_open: false,
        })
    }

    /// Open/initialise the key provider.
    pub fn open(&mut self) -> KeyResult<()> {
        self.is_open = true;
        Ok(())
    }

    /// Get the public key (32 bytes).
    pub fn public_key(&self) -> KeyResult<[u8; 32]> {
        if !self.is_open {
            return Err(KeyError::NotInitialised);
        }
        Ok(self.verifying_key.to_bytes())
    }

    /// Get the verifying key for signature verification.
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    /// Sign data using the private key (detached Ed25519 signature, 64 bytes).
    pub fn sign(&self, data: &[u8]) -> KeyResult<[u8; 64]> {
        if !self.is_open {
            return Err(KeyError::NotInitialised);
        }
        Ok(self.signing_key.sign(data).to_bytes())
    }

    /// Clone the signing key (for use by SignatureEngine).
    pub fn signing_key_clone(&self) -> SigningKey {
        // Re-derive from stored key bytes
        SigningKey::from_bytes(&self.signing_key.to_bytes())
    }

    /// Close the key provider and zeroize the private key.
    pub fn close(&mut self) {
        // In production, the private key memory would be zeroized here.
        // ed25519-dalek does this automatically on Drop.
        self.is_open = false;
    }
}

impl Drop for DefaultKeyProvider {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    #[test]
    fn test_generate_and_sign() {
        let mut kp = DefaultKeyProvider::new().unwrap();
        kp.open().unwrap();

        let pk = kp.public_key().unwrap();
        assert_eq!(pk.len(), 32);

        let sig = kp.sign(b"hello dologger").unwrap();
        assert_eq!(sig.len(), 64);

        // Verify the signature
        let vk = VerifyingKey::from_bytes(&pk).unwrap();
        let signature = ed25519_dalek::Signature::from_bytes(&sig);
        assert!(vk.verify(b"hello dologger", &signature).is_ok());
        // Wrong message should fail
        assert!(vk.verify(b"wrong message", &signature).is_err());

        kp.close();
    }
}
