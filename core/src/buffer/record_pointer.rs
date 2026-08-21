//! Ownership token for records transferred through the lock-free ring.

use crate::record::Record;

/// A move-only record pointer owned by the pool/ring hand-off protocol.
///
/// The safe accessor is crate-private; external callers must use the explicit
/// unsafe constructor and consuming escape to make ownership visible.
#[derive(Debug)]
pub struct RecordPtr(*mut Record);

impl RecordPtr {
    /// Wrap a raw record pointer for an explicit cross-thread transfer.
    ///
    /// # Safety
    ///
    /// `pointer` must come from a live [`crate::buffer::RecordPool`] allocation
    /// and ownership must be transferred to this token exactly once. The
    /// caller must not access or free the record until the token is consumed.
    pub unsafe fn from_raw(pointer: *mut Record) -> Self {
        Self::new(pointer)
    }

    /// Consume the token and return the owned raw pointer.
    pub fn into_raw(self) -> *mut Record {
        self.0
    }

    /// Wrap a pointer returned by [`crate::buffer::RecordPool::alloc`].
    pub(crate) fn new(pointer: *mut Record) -> Self {
        Self(pointer)
    }

    /// Borrow the raw pointer without changing ownership.
    pub(crate) fn as_ptr(&self) -> *mut Record {
        self.0
    }
}

// SAFETY: A token is created only from a pool allocation and is moved between
// producer and consumer threads exactly once. The pool protocol grants
// exclusive access to the pointed-to Record until the token is consumed.
unsafe impl Send for RecordPtr {}

// SAFETY: Sharing a token reference does not expose the Record. The only
// operation available through the crate boundary is copying its opaque address;
// mutation remains guarded by the ring's single-consumer ownership protocol.
unsafe impl Sync for RecordPtr {}
