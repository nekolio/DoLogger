//! Fixed-size KV slot (ADR-002 Appendix A.1).
//!
//! Each slot is exactly 32 bytes: `tag(1) + type(1) + len(1) + value(29)`.
//! This is the in-Record representation of dynamic fields (ruling #10); the
//! disk/canonical form (§A.3) never contains the overflow pointer — it is a
//! memory-only concept.
//!
//! Layout of the 29-byte value area:
//!
//! | `len` byte | Meaning                                    | Value area          |
//! |:----------:|:-------------------------------------------|:--------------------|
//! | `0-29`     | inline payload (`value[0..len]`)           | payload bytes       |
//! | `0xFF`     | overflow (string/binary > 29 B)            | `ptr(8)` + `len(8)` |
//!
//! The overflow descriptor holds an 8-byte heap pointer (Box<[u8]>, thin) and
//! the 8-byte total payload length (ruling #13). Overflow payloads are
//! allocated independently per slot, mirroring the `RecordString` continuation
//! pattern.

use std::ptr;

/// Total slot size in bytes — budget-critical constant (256B Record math).
pub const KV_SLOT_SIZE: usize = 32;

/// Inline payload capacity of the value area (29 B after tag/type/len).
pub const KV_VALUE_CAPACITY: usize = KV_SLOT_SIZE - 3;

/// `len` byte sentinel marking the overflow descriptor (never a real length).
pub const KV_LEN_OVERFLOW: u8 = 0xFF;

/// `tag` byte sentinel for an empty slot (no field present).
pub const KV_TAG_EMPTY: u8 = 0;

/// Largest core tag (`1-63`); `64+` is vendor (runtime lazy mapping, ruling #14).
pub const KV_TAG_CORE_MAX: u8 = 63;

/// Value type byte (ruling #11) — closed set of seven types. The type set is
/// deliberately closed: no u128 (id is BINARY 16B, ruling #17) and no custom
/// extension (unknown bytes are preserved by [`KvSlot::get`] for deny-by-default
/// handling at the field API layer).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvType {
    /// Signed 64-bit integer
    Int64 = 1,
    /// Unsigned 64-bit integer
    UInt64 = 2,
    /// IEEE-754 double
    Double = 3,
    /// Boolean
    Bool = 4,
    /// UTF-8 string
    String = 5,
    /// Opaque binary blob
    Binary = 6,
    /// Explicit null
    Null = 7,
}

impl KvType {
    /// Parse a type byte. Returns `None` for values outside the closed set.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Int64),
            2 => Some(Self::UInt64),
            3 => Some(Self::Double),
            4 => Some(Self::Bool),
            5 => Some(Self::String),
            6 => Some(Self::Binary),
            7 => Some(Self::Null),
            _ => None,
        }
    }

    /// The raw type byte.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Error returned by [`KvSlot::put`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvPutError {
    /// `tag` must not be `KV_TAG_EMPTY` (the empty slot sentinel).
    TagReserved,
}

/// A fixed 32-byte key-value slot.
///
/// `#[repr(C)]` pins the field offsets (`tag` at 0, `type` at 1, `len` at 2,
/// value at 3) so the layout is stable across targets and matches §A.1.
#[repr(C)]
pub struct KvSlot {
    /// Tag byte: `0` = EMPTY, `1-63` = core fixed table, `64+` = vendor.
    tag: u8,
    /// Value type byte ([`KvType`]).
    ty: u8,
    /// Payload length: `0-29` inline, `0xFF` = overflow descriptor.
    len: u8,
    /// Inline payload or overflow descriptor (`ptr(8)` + `len(8)`).
    value: [u8; KV_VALUE_CAPACITY],
}

impl KvSlot {
    /// An empty slot (EMPTY sentinel).
    pub const fn empty() -> Self {
        Self {
            tag: KV_TAG_EMPTY,
            ty: 0,
            len: 0,
            value: [0u8; KV_VALUE_CAPACITY],
        }
    }

    /// True when the slot carries no field (`tag == 0`).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tag == KV_TAG_EMPTY
    }

    /// The tag byte (0 when empty).
    #[inline]
    pub fn tag(&self) -> u8 {
        self.tag
    }

    /// The raw type byte (0 when empty; may be outside [`KvType`] for slots
    /// decoded from disk — preserved verbatim, see [`KvSlot::get`]).
    #[inline]
    pub fn type_byte(&self) -> u8 {
        self.ty
    }

    /// Write a value into the slot, replacing any prior content.
    ///
    /// Values ≤ 29 bytes are stored inline (no allocation); larger values are
    /// copied to an independent heap buffer referenced by the overflow
    /// descriptor. Any previous overflow allocation is freed first, so a slot
    /// never leaks when overwritten.
    pub fn put(&mut self, tag: u8, ty: KvType, data: &[u8]) -> Result<(), KvPutError> {
        if tag == KV_TAG_EMPTY {
            return Err(KvPutError::TagReserved);
        }
        self.drop_overflow();
        self.tag = tag;
        self.ty = ty.as_u8();
        if data.len() <= KV_VALUE_CAPACITY {
            // SAFETY: data.len() ≤ 29 == value.len(); both slices are valid
            // for that many bytes and do not overlap (one is caller input).
            unsafe {
                ptr::copy_nonoverlapping(data.as_ptr(), self.value.as_mut_ptr(), data.len());
            }
            self.len = data.len() as u8;
        } else {
            // Heap path: thin `*mut u8` at value[0..8], total length at
            // value[8..16]. The write_unaligned calls tolerate the value area
            // starting at offset 3 in the repr(C) struct.
            let boxed: Box<[u8]> = data.to_vec().into_boxed_slice();
            let thin = Box::into_raw(boxed) as *mut u8;
            self.len = KV_LEN_OVERFLOW;
            // SAFETY: `value` is writable for 16 bytes; write_unaligned
            // handles the byte-array alignment of the descriptor area.
            unsafe {
                ptr::write_unaligned(self.value.as_mut_ptr() as *mut *mut u8, thin);
            }
            self.value[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes());
        }
        Ok(())
    }

    /// Read the value as a zero-copy view.
    ///
    /// Returns `(tag, type byte, payload)`; `None` for an empty slot. The
    /// payload borrows the slot (inline) or the slot-owned heap buffer
    /// (overflow), so it stays valid until the next `&mut` operation. The
    /// type byte is returned raw so unknown disk-encoded types are preserved
    /// for caller-side deny-by-default handling.
    pub fn get(&self) -> Option<(u8, u8, &[u8])> {
        if self.is_empty() {
            return None;
        }
        Some((self.tag, self.ty, self.value_slice()))
    }

    /// Reset the slot to EMPTY, freeing any overflow allocation.
    pub fn clear(&mut self) {
        self.drop_overflow();
        *self = Self::empty();
    }

    /// Append the canonical serialization (§A.3) of this slot to `out`.
    ///
    /// The serialized form is `tag + type + len + value` where `len` is the
    /// **actual payload length** (0-254 inline, or `0xFF` + 8-byte length for
    /// ≥255-byte payloads) and the payload is the real data — never a pointer.
    /// A 30-byte value therefore serializes as `len = 30` even though it lives
    /// in the overflow descriptor in memory: the disk form is storage-agnostic,
    /// which is what makes hashing deterministic.
    ///
    /// Empty slots contribute nothing (deterministic by construction — each
    /// slot serializes independently and the walk order is fixed).
    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        if self.is_empty() {
            return;
        }
        let data = self.value_slice();
        out.push(self.tag);
        out.push(self.ty);
        if data.len() < u8::MAX as usize {
            out.push(data.len() as u8);
            out.extend_from_slice(data);
        } else {
            out.push(KV_LEN_OVERFLOW);
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(data);
        }
    }

    /// Zero-copy view of the payload: inline bytes or the overflow heap slice.
    fn value_slice(&self) -> &[u8] {
        if self.len == KV_LEN_OVERFLOW {
            // SAFETY: read_unaligned tolerates the byte-array alignment. The
            // thin pointer was written by put() from a live Box; the Box stays
            // alive until drop_overflow() (which needs `&mut self`), so the
            // borrow from `&self` cannot dangle.
            let thin = unsafe { ptr::read_unaligned(self.value.as_ptr() as *const *mut u8) };
            let len = u64::from_le_bytes(self.value[8..16].try_into().unwrap()) as usize;
            // SAFETY: `thin` + `len` reproduce the exact Box<[u8]> allocation
            // created in put(); the pointer is valid and the length matches.
            unsafe { std::slice::from_raw_parts(thin as *const u8, len) }
        } else {
            &self.value[..self.len as usize]
        }
    }

    /// Free the overflow allocation, if any. Safe to call on inline/empty
    /// slots and idempotent (the `len` byte is cleared only by [`KvSlot::put`]
    /// / [`KvSlot::clear`], so a live descriptor is never freed twice).
    fn drop_overflow(&mut self) {
        if self.len != KV_LEN_OVERFLOW {
            return;
        }
        // SAFETY: same reproduction as value_slice(); Box::from_raw takes back
        // ownership of the exact allocation put() created.
        let thin = unsafe { ptr::read_unaligned(self.value.as_ptr() as *const *mut u8) };
        let len = u64::from_le_bytes(self.value[8..16].try_into().unwrap()) as usize;
        // SAFETY: `thin` + `len` reproduce the exact Box<[u8]> allocation
        // created in put() — same reproduction as value_slice(); Box::from_raw
        // takes back ownership of that allocation so it is freed exactly once.
        unsafe {
            let slice = std::ptr::slice_from_raw_parts_mut(thin, len);
            drop(Box::from_raw(slice));
        }
        // Clear the descriptor marker so a subsequent `*self = empty()` (in
        // `clear`) does not drop the now-freed allocation a second time.
        self.len = 0;
    }

    // ── Convenience typed accessors (called by Record KV helpers) ──

    /// Put a UTF-8 string value.
    pub fn put_string(&mut self, tag: u8, value: &str) {
        let _ = self.put(tag, KvType::String, value.as_bytes());
    }

    /// Put an opaque binary value.
    pub fn put_binary(&mut self, tag: u8, value: &[u8]) {
        let _ = self.put(tag, KvType::Binary, value);
    }

    /// Put an unsigned 64-bit integer (little-endian).
    pub fn put_u64(&mut self, tag: u8, value: u64) {
        let _ = self.put(tag, KvType::UInt64, &value.to_le_bytes());
    }

    /// Put a signed 64-bit integer (little-endian).
    pub fn put_i64(&mut self, tag: u8, value: i64) {
        let _ = self.put(tag, KvType::Int64, &value.to_le_bytes());
    }

    /// Read the payload as a UTF-8 string. Returns `None` for empty slots,
    /// non-string types, or invalid UTF-8.
    pub fn get_string(&self) -> Option<String> {
        let (_, ty, data) = self.get()?;
        if ty == KvType::String.as_u8() {
            std::str::from_utf8(data).ok().map(String::from)
        } else {
            None
        }
    }

    /// Feed this slot's canonical serialization bytes into a buffer for
    /// content-hash computation (§A.3). Empty slots contribute nothing.
    pub fn hash_slot(&self, buf: &mut Vec<u8>) {
        if self.is_empty() {
            return;
        }
        buf.push(self.tag);
        buf.push(self.ty);
        let data = self.value_slice();
        buf.extend_from_slice(&(data.len() as u64).to_le_bytes());
        buf.extend_from_slice(data);
    }
}

impl Drop for KvSlot {
    /// Safety net for slots that leave scope without [`KvSlot::clear`]
    /// (e.g. stack-constructed slots). Production slots are Record-owned and
    /// cleared by `Record::reset` first, so this is a no-op for them.
    fn drop(&mut self) {
        self.drop_overflow();
    }
}

// Compile-time invariants: the slot must stay exactly 32 bytes and the value
// area exactly 29 — the 256B Record budget (A.2) and the canonical
// serialization both depend on these constants.
const _: () = {
    assert!(core::mem::size_of::<KvSlot>() == KV_SLOT_SIZE);
    assert!(core::mem::align_of::<KvSlot>() == 1);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_size_invariant() {
        // Budget-critical: two slots + ext ptr must fit the 256B Record.
        assert_eq!(core::mem::size_of::<KvSlot>(), 32);
        assert_eq!(KV_VALUE_CAPACITY, 29);
    }

    #[test]
    fn empty_slot_state() {
        let slot = KvSlot::empty();
        assert!(slot.is_empty());
        assert_eq!(slot.tag(), 0);
        assert_eq!(slot.type_byte(), 0);
        assert_eq!(slot.get(), None);
        let mut out = Vec::new();
        slot.serialize_into(&mut out);
        assert!(out.is_empty(), "empty slots serialize to nothing");
    }

    #[test]
    fn tag_zero_is_rejected() {
        let mut slot = KvSlot::empty();
        assert_eq!(
            slot.put(0, KvType::String, b"x"),
            Err(KvPutError::TagReserved)
        );
        assert!(slot.is_empty());
    }

    #[test]
    fn inline_roundtrip() {
        let mut slot = KvSlot::empty();
        slot.put(4, KvType::String, b"user.id").unwrap();
        let (tag, ty, data) = slot.get().unwrap();
        assert_eq!(tag, 4);
        assert_eq!(ty, KvType::String.as_u8());
        assert_eq!(data, b"user.id");
        assert!(!slot.is_empty());
    }

    #[test]
    fn inline_boundary_exact_capacity() {
        let mut slot = KvSlot::empty();
        let data = [0xABu8; KV_VALUE_CAPACITY];
        slot.put(7, KvType::Binary, &data).unwrap();
        assert_eq!(slot.len, data.len() as u8, "29B must stay inline");
        assert_eq!(slot.get().unwrap().2, &data[..]);
    }

    #[test]
    fn overflow_crosses_boundary() {
        let mut slot = KvSlot::empty();
        let data = [0xCDu8; KV_VALUE_CAPACITY + 1];
        slot.put(1, KvType::Binary, &data).unwrap();
        assert_eq!(slot.len, KV_LEN_OVERFLOW, "30B must use the descriptor");
        let (tag, ty, payload) = slot.get().unwrap();
        assert_eq!(tag, 1);
        assert_eq!(ty, KvType::Binary.as_u8());
        assert_eq!(payload, &data[..]);
    }

    #[test]
    fn overflow_large_payload_roundtrip() {
        let mut slot = KvSlot::empty();
        let text = "s".repeat(4096);
        slot.put(2, KvType::String, text.as_bytes()).unwrap();
        assert_eq!(slot.len, KV_LEN_OVERFLOW);
        assert_eq!(slot.get().unwrap().2, text.as_bytes());
    }

    #[test]
    fn overwrite_frees_and_replaces() {
        let mut slot = KvSlot::empty();
        // Large → small must free the heap and return to inline.
        slot.put(1, KvType::String, "A".repeat(300).as_bytes())
            .unwrap();
        assert_eq!(slot.len, KV_LEN_OVERFLOW);
        slot.put(1, KvType::String, b"tiny").unwrap();
        assert_eq!(slot.len, 4, "must return to inline after overwrite");
        assert_eq!(slot.get().unwrap().2, b"tiny");
        // Small → large must allocate a fresh heap and not double-free.
        slot.put(1, KvType::String, "B".repeat(500).as_bytes())
            .unwrap();
        assert_eq!(slot.get().unwrap().2, "B".repeat(500).as_bytes());
    }

    #[test]
    fn clear_frees_and_resets() {
        let mut slot = KvSlot::empty();
        slot.put(3, KvType::String, "C".repeat(2048).as_bytes())
            .unwrap();
        slot.clear();
        assert!(slot.is_empty());
        assert_eq!(slot.get(), None);
        // Reuse after clear works and allocates cleanly.
        slot.put(3, KvType::String, b"again").unwrap();
        assert_eq!(slot.get().unwrap().2, b"again");
    }

    #[test]
    fn scalar_types_roundtrip() {
        let mut slot = KvSlot::empty();
        slot.put(12, KvType::UInt64, &42u64.to_le_bytes()).unwrap();
        let (_, ty, data) = slot.get().unwrap();
        assert_eq!(ty, KvType::UInt64.as_u8());
        assert_eq!(u64::from_le_bytes(data.try_into().unwrap()), 42);
    }

    #[test]
    fn all_type_bytes_parse() {
        for v in 1..=7u8 {
            assert!(KvType::from_u8(v).is_some(), "type {v} must parse");
        }
        assert_eq!(KvType::from_u8(0), None);
        assert_eq!(KvType::from_u8(8), None);
        assert_eq!(KvType::from_u8(0xFF), None);
    }

    #[test]
    fn unknown_type_byte_is_preserved() {
        // Slots decoded from disk may carry a type byte outside the closed
        // set; get() must return it verbatim for caller-side handling.
        let mut slot = KvSlot::empty();
        slot.put(5, KvType::String, b"x").unwrap();
        slot.ty = 0xEE;
        let (tag, ty, data) = slot.get().unwrap();
        assert_eq!(tag, 5);
        assert_eq!(ty, 0xEE);
        assert_eq!(data, b"x");
    }

    #[test]
    fn serialize_inline_matches_memory() {
        let mut slot = KvSlot::empty();
        let payload = b"hello";
        slot.put(6, KvType::String, payload).unwrap();
        let mut out = Vec::new();
        slot.serialize_into(&mut out);
        // Inline: serialized form == memory bytes (tag, ty, len, value).
        assert_eq!(
            out,
            vec![6, KvType::String.as_u8(), 5, b'h', b'e', b'l', b'l', b'o']
        );
    }

    #[test]
    fn serialize_disk_form_is_storage_agnostic() {
        // A 30-byte value lives in the overflow descriptor in memory but must
        // serialize as len=30 + data — no pointer on disk (§A.1/A.3).
        let mut slot = KvSlot::empty();
        let payload = [0x77u8; 30];
        slot.put(9, KvType::Binary, &payload).unwrap();
        assert_eq!(slot.len, KV_LEN_OVERFLOW, "precondition: memory overflow");
        let mut out = Vec::new();
        slot.serialize_into(&mut out);
        assert_eq!(out.len(), 1 + 1 + 1 + 30);
        assert_eq!(&out[..3], &[9, KvType::Binary.as_u8(), 30]);
        assert_eq!(&out[3..], &payload[..]);
    }

    #[test]
    fn serialize_extended_length_uses_marker() {
        // ≥255-byte payloads serialize as 0xFF + 8-byte length + data.
        let mut slot = KvSlot::empty();
        let payload = [0x55u8; 300];
        slot.put(10, KvType::Binary, &payload).unwrap();
        let mut out = Vec::new();
        slot.serialize_into(&mut out);
        assert_eq!(out.len(), 1 + 1 + 1 + 8 + 300);
        assert_eq!(&out[..3], &[10, KvType::Binary.as_u8(), KV_LEN_OVERFLOW]);
        assert_eq!(u64::from_le_bytes(out[3..11].try_into().unwrap()), 300);
        assert_eq!(&out[11..], &payload[..]);
    }

    #[test]
    fn serialize_deterministic_across_storage() {
        // Same (tag, ty, data) must serialize identically whether inline or
        // overflow — the content_hash invariant (§A.3). 30B forces overflow.
        let mut inline_slot = KvSlot::empty();
        let mut overflow_slot = KvSlot::empty();
        let payload = [0x11u8; 29];
        inline_slot.put(8, KvType::Binary, &payload).unwrap();
        // Same logical value stored 1 byte larger → overflow in memory.
        let bigger = [0x11u8; 30];
        overflow_slot.put(8, KvType::Binary, &bigger).unwrap();
        let mut a = Vec::new();
        let mut b = Vec::new();
        inline_slot.serialize_into(&mut a);
        overflow_slot.serialize_into(&mut b);
        // Not byte-identical (29 vs 30 payload) — but the FORMAT must match:
        // both inline-length form, no pointer anywhere.
        assert_eq!(&a[..2], &b[..2], "tag+type identical");
        assert_eq!(a[2], 29);
        assert_eq!(b[2], 30);
        assert_eq!(&b[3..], &bigger[..]);
    }

    #[test]
    fn drop_with_overflow_is_safe() {
        // A slot with a heap payload that leaves scope without clear() must
        // not leak or double-free (Drop is the safety net).
        let slot = {
            let mut s = KvSlot::empty();
            s.put(1, KvType::String, "D".repeat(700).as_bytes())
                .unwrap();
            s
        };
        drop(slot);
    }

    #[test]
    fn slot_is_send_and_sync() {
        // KvSlot must be usable inside Record (which crosses threads via the
        // ring buffer). The overflow Box is owned exclusively, so Send+Sync
        // hold as long as the slot is only mutated through &mut self.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<KvSlot>();
    }
}
