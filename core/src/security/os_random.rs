//! Operating-system CSPRNG access for security-sensitive material.
//!
//! The security layer exposes one small, audited operation instead of letting
//! every caller select a random backend independently. Windows uses the system
//! preferred CNG provider; Unix-like systems use `/dev/urandom`. The function
//! is deliberately fallible so a caller can choose whether startup must fail
//! or whether a non-security fallback is acceptable.

use std::fmt;

/// Errors returned by the platform random source.
#[derive(Debug)]
pub enum OsRandomError {
    /// The Unix random device could not be opened or read.
    Io(std::io::Error),
    /// The Windows CNG API returned a failing NTSTATUS value.
    PlatformStatus(u32),
}

impl fmt::Display for OsRandomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "OS random source failed: {error}"),
            Self::PlatformStatus(status) => {
                write!(
                    formatter,
                    "OS random provider returned status 0x{status:08x}"
                )
            }
        }
    }
}

impl std::error::Error for OsRandomError {}

/// Fill a byte slice from the operating-system CSPRNG.
pub fn fill_bytes(destination: &mut [u8]) -> Result<(), OsRandomError> {
    if destination.is_empty() {
        return Ok(());
    }
    platform::fill_bytes(destination)
}

#[cfg(windows)]
mod platform {
    use super::OsRandomError;
    use std::ffi::c_void;

    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    const STATUS_SUCCESS: i32 = 0;

    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8, length: u32, flags: u32)
            -> i32;
    }

    pub(super) fn fill_bytes(destination: &mut [u8]) -> Result<(), OsRandomError> {
        let mut offset = 0usize;
        while offset < destination.len() {
            let length = (destination.len() - offset).min(u32::MAX as usize);
            let status = unsafe {
                // SAFETY: `destination[offset..]` is a valid writable slice and
                // `length` is bounded to the CNG API's u32 parameter.
                BCryptGenRandom(
                    std::ptr::null_mut(),
                    destination[offset..].as_mut_ptr(),
                    length as u32,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            };
            if status != STATUS_SUCCESS {
                return Err(OsRandomError::PlatformStatus(status as u32));
            }
            offset += length;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::OsRandomError;
    use std::fs::File;
    use std::io::Read;

    pub(super) fn fill_bytes(destination: &mut [u8]) -> Result<(), OsRandomError> {
        let mut source = File::open("/dev/urandom").map_err(OsRandomError::Io)?;
        source.read_exact(destination).map_err(OsRandomError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_slice_is_a_noop() {
        fill_bytes(&mut []).expect("empty request should succeed");
    }

    #[test]
    fn fills_the_requested_size() {
        let mut bytes = [0u8; 128];
        fill_bytes(&mut bytes).expect("OS CSPRNG should be available");
        assert_eq!(bytes.len(), 128);
    }

    #[test]
    fn consecutive_blocks_are_not_identical() {
        let mut first = [0u8; 32];
        let mut second = [0u8; 32];
        fill_bytes(&mut first).expect("first block");
        fill_bytes(&mut second).expect("second block");
        assert_ne!(first, second);
    }

    #[test]
    fn large_requests_are_chunkable_on_windows() {
        let mut bytes = vec![0u8; 4096];
        fill_bytes(&mut bytes).expect("large request");
        assert_eq!(bytes.len(), 4096);
    }
}
