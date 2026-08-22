//! Record object pool using a lock-free Treiber stack.
//!
//! Records are pre-allocated and stored in `UnsafeCell`s. `alloc()` returns
//! a mutable raw pointer, `free()` returns it to the pool. This is safe
//! because the pool grants exclusive access to each record: a record in the
//! free list is never accessed, and a record handed out is never accessed
//! by the pool until it's returned.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::record::Record;

struct PoolNode {
    /// The pre-allocated record (interior mutable via UnsafeCell)
    record: UnsafeCell<Record>,
    /// Index of the next free node (usize::MAX = end of stack)
    next: AtomicUsize,
}

/// Lock-free object pool for Records.
pub struct RecordPool {
    nodes: Box<[PoolNode]>,
    head: AtomicUsize,
    alloc_count: AtomicU64,
    free_count: AtomicU64,
}

// SAFETY: All shared state (head, alloc_count, free_count) uses atomic operations.
// Record access is controlled by the free-stack protocol: a record in the free list
// is never accessed, and a record handed out has exactly one owner until returned.
unsafe impl Send for RecordPool {}
// SAFETY: See Send impl — atomic coordination + exclusive ownership protocol.
unsafe impl Sync for RecordPool {}

impl RecordPool {
    /// Create a new pool with `capacity` pre-allocated records.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Pool capacity must be positive");

        let mut nodes = Vec::with_capacity(capacity);
        for i in 0..capacity {
            nodes.push(PoolNode {
                record: UnsafeCell::new(Record::new(i as u32)),
                next: AtomicUsize::new(if i + 1 < capacity { i + 1 } else { usize::MAX }),
            });
        }

        Self {
            nodes: nodes.into_boxed_slice(),
            head: AtomicUsize::new(0),
            alloc_count: AtomicU64::new(0),
            free_count: AtomicU64::new(0),
        }
    }

    /// Allocate a record from the pool.
    ///
    /// Returns a mutable raw pointer to the record, or `None` if exhausted.
    /// The caller has exclusive mutable access to the record until `free()` is called.
    pub fn alloc(&self) -> Option<*mut Record> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head == usize::MAX {
                return None; // Pool exhausted
            }

            let node = &self.nodes[head];
            let next = node.next.load(Ordering::Acquire);

            if self
                .head
                .compare_exchange_weak(head, next, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.alloc_count.fetch_add(1, Ordering::Relaxed);
                return Some(node.record.get());
            }
            // CAS failed — retry
        }
    }

    /// Return a record to the pool.
    ///
    /// # Safety
    ///
    /// `record_ptr` must have been obtained from this pool via `alloc()` and
    /// not yet returned. The caller must not access the record after freeing.
    /// Passing an invalid pointer (wrong pool, stack-allocated, already freed)
    /// results in undefined behavior.
    pub unsafe fn free(&self, record_ptr: *const Record) {
        // Recover the pool index from the record
        // SAFETY: record_ptr was obtained from this pool via alloc() and hasn't been freed yet.
        // The caller guarantees exclusive access until this free() call completes.
        let record = unsafe { &*record_ptr };
        let index = record.pool_index as usize;
        assert!(index < self.nodes.len(), "Record pool_index out of bounds");

        // Reset the record before returning it
        let node = &self.nodes[index];
        // SAFETY: The record is currently free (off the free stack), so we have
        // exclusive access. UnsafeCell::get() yields a mutable pointer.
        unsafe {
            let record_mut = &mut *node.record.get();
            record_mut.reset();
        }

        // Push back onto the free stack
        loop {
            let head = self.head.load(Ordering::Acquire);
            node.next.store(head, Ordering::Release);

            if self
                .head
                .compare_exchange_weak(head, index, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.free_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
            // CAS failed — retry
        }
    }

    /// Total allocations since pool creation.
    pub fn alloc_count(&self) -> u64 {
        self.alloc_count.load(Ordering::Relaxed)
    }

    /// Total frees since pool creation.
    pub fn free_count(&self) -> u64 {
        self.free_count.load(Ordering::Relaxed)
    }

    /// Approximate number of available records.
    pub fn available(&self) -> usize {
        let allocs = self.alloc_count.load(Ordering::Relaxed);
        let frees = self.free_count.load(Ordering::Relaxed);
        self.nodes
            .len()
            .saturating_sub(allocs.saturating_sub(frees) as usize)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_free() {
        let pool = RecordPool::new(64);

        let r1 = pool.alloc().expect("should allocate");
        // SAFETY: r1 was just allocated, pointer is valid
        assert_eq!(unsafe { (*r1).pool_index }, 0);
        assert_eq!(pool.alloc_count(), 1);

        let r2 = pool.alloc().expect("should allocate");
        // SAFETY: r2 was just allocated, pointer is valid
        assert_eq!(unsafe { (*r2).pool_index }, 1);
        assert_eq!(pool.alloc_count(), 2);

        assert!(r1 != r2);

        // SAFETY: r1 and r2 were obtained from this pool via alloc() and
        // have not been freed yet.
        unsafe {
            pool.free(r1);
            pool.free(r2);
        }
        assert_eq!(pool.free_count(), 2);
    }

    #[test]
    fn test_alloc_all() {
        let pool = RecordPool::new(4);

        let r1 = pool.alloc();
        let r2 = pool.alloc();
        let r3 = pool.alloc();
        let r4 = pool.alloc();

        assert!(r1.is_some());
        assert!(r2.is_some());
        assert!(r3.is_some());
        assert!(r4.is_some());
        assert!(pool.alloc().is_none());
    }

    #[test]
    fn test_reuse_after_free() {
        let pool = RecordPool::new(4);

        let r1 = pool.alloc().unwrap();
        let r2 = pool.alloc().unwrap();
        let _r3 = pool.alloc().unwrap();
        let _r4 = pool.alloc().unwrap();

        assert!(pool.alloc().is_none()); // exhausted

        // SAFETY: r1 and r2 were obtained from this pool via alloc() and
        // have not been freed yet.
        unsafe {
            pool.free(r1);
            pool.free(r2);
        }

        // Should be able to allocate again
        let r5 = pool.alloc().unwrap();
        let _r6 = pool.alloc().unwrap();
        assert!(r5 == r2 || r5 == r1); // Reuses freed slot
        assert!(pool.alloc().is_none());
    }

    #[test]
    fn test_reuse_clears_all_record_hot_fields() {
        let pool = RecordPool::new(1);
        let record_ptr = pool.alloc().expect("record should allocate");

        // SAFETY: record_ptr is exclusively owned by this test until free().
        unsafe {
            (*record_ptr).timestamp = 1_700_000_000_000_000_000;
            (*record_ptr).level = crate::record::LogLevel::Audit;
            (*record_ptr).process_id = 42;
            (*record_ptr).thread_id = 7;
            (*record_ptr).lsn = 99;
            (*record_ptr).flags = 0x55;
            (*record_ptr).message.set("stale payload");
            pool.free(record_ptr);
        }

        let reused = pool.alloc().expect("record should be reusable");
        // SAFETY: reused is exclusively owned by this test.
        unsafe {
            assert_eq!((*reused).timestamp, 0);
            assert_eq!((*reused).level, crate::record::LogLevel::Info);
            assert_eq!((*reused).process_id, 0);
            assert_eq!((*reused).thread_id, 0);
            assert_eq!((*reused).lsn, 0);
            assert_eq!((*reused).flags, 0);
            assert!((*reused).message.is_empty());
            pool.free(reused);
        }
    }
}
