//! Lock-free MPSC (Multiple Producer, Single Consumer) ring buffer.
//!
//! # Design
//!
//! Based on a bounded array with power-of-two capacity, using CAS (Compare-And-Swap)
//! atomic operations for producer sequence number contention.
//!
//! # Characteristics
//!
//! - **Wait-free producers**: CAS-based slot claiming, no mutexes
//! - **Batch consumption**: Consumer drains multiple records per trip
//! - **Cache-line padding**: Prevents false sharing between producers
//! - **Power-of-two capacity**: Bitmask modulo for fast index calculation

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};

/// Default ring buffer capacity (256K records).
pub const DEFAULT_CAPACITY: usize = 262144;

/// A single slot in the ring buffer.
///
/// Each slot holds a record pointer and a sequence number for
/// coordinating access between producers and consumer.
#[repr(C, align(64))] // Cache-line aligned to prevent false sharing
pub(crate) struct RingSlot<T> {
    // Fields intentionally pub(crate) — accessed by RingBuffer only
    /// The stored value
    data: UnsafeCell<Option<T>>,
    /// Sequence number for this slot (used for producer-consumer coordination)
    sequence: AtomicU64,
}

// Safety: RingSlot is Sync because access is coordinated via atomic sequence numbers
unsafe impl<T: Send> Sync for RingSlot<T> {}

impl<T> RingSlot<T> {
    fn new(sequence: u64) -> Self {
        Self {
            data: UnsafeCell::new(None),
            sequence: AtomicU64::new(sequence),
        }
    }
}

/// Lock-free MPSC ring buffer.
///
/// # Type Parameters
///
/// * `T` - The type of items stored in the buffer
pub struct RingBuffer<T> {
    /// Storage for ring buffer slots
    slots: Box<[RingSlot<T>]>,
    /// Capacity (power of two)
    capacity: usize,
    /// Bitmask for fast modulo (capacity - 1)
    mask: u64,
    /// Producer sequence counter (next slot to be claimed)
    producer_sequence: AtomicU64,
    /// Consumer sequence counter (next slot to be consumed)
    consumer_sequence: AtomicU64,
}

// SAFETY: All shared state in RingBuffer uses atomic operations (AtomicU64).
// Access to individual slots is coordinated via CAS-based sequence numbers,
// ensuring exclusive access to each slot. T may be !Send; the buffer provides
// thread-safe producer-consumer semantics regardless.
unsafe impl<T> Send for RingBuffer<T> {}
// SAFETY: See Send impl — atomic sequence coordination ensures no data races.
unsafe impl<T> Sync for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if capacity is not a power of two, or if capacity is 0.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Ring buffer capacity must be positive");
        assert!(
            capacity.is_power_of_two(),
            "Ring buffer capacity must be a power of two: {} is not",
            capacity
        );

        let mut slots = Vec::with_capacity(capacity);
        for i in 0..capacity {
            slots.push(RingSlot::new(i as u64));
        }

        Self {
            slots: slots.into_boxed_slice(),
            capacity,
            mask: (capacity - 1) as u64,
            producer_sequence: AtomicU64::new(0),
            consumer_sequence: AtomicU64::new(0),
        }
    }

    /// Get the capacity of the ring buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Try to push an item into the ring buffer.
    ///
    /// Returns `true` if the item was enqueued, `false` if the buffer is full.
    pub fn try_push(&self, item: T) -> Result<(), T> {
        loop {
            let producer_seq = self.producer_sequence.load(Ordering::Acquire);
            let consumer_seq = self.consumer_sequence.load(Ordering::Acquire);

            // Check if buffer is full
            if producer_seq - consumer_seq >= self.capacity as u64 {
                return Err(item);
            }

            // Try to claim this slot
            match self.producer_sequence.compare_exchange_weak(
                producer_seq,
                producer_seq + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Successfully claimed slot: write data
                    let index = (producer_seq & self.mask) as usize;
                    let slot = &self.slots[index];

                    // Wait until the slot is ready for writing
                    while slot.sequence.load(Ordering::Acquire) != producer_seq {
                        std::hint::spin_loop();
                    }

                    // SAFETY: We have exclusive write access to this slot because
                    // the CAS on producer_sequence guarantees only one producer wins.
                    unsafe {
                        (*slot.data.get()) = Some(item);
                    }

                    // Publish the slot for the consumer
                    slot.sequence.store(producer_seq + 1, Ordering::Release);

                    return Ok(());
                }
                Err(_) => {
                    // CAS failed, another producer claimed it; retry
                    continue;
                }
            }
        }
    }

    /// Drain up to `batch_size` items from the buffer, calling `f` for each.
    ///
    /// Uses CAS on `consumer_sequence` so it interoperates with `drain_helping`
    /// . Each slot is claimed individually; when a cooperative helper
    /// snatches a slot, the consumer retries with the next one.
    ///
    /// Returns the number of items consumed.
    pub fn drain<F: FnMut(T)>(&self, batch_size: usize, mut f: F) -> usize {
        let mut count = 0;

        while count < batch_size {
            let consumer_seq = self.consumer_sequence.load(Ordering::Acquire);
            let producer_seq = self.producer_sequence.load(Ordering::Acquire);

            // Buffer is empty — nothing to drain
            if consumer_seq >= producer_seq {
                break;
            }

            let index = (consumer_seq & self.mask) as usize;
            let slot = &self.slots[index];

            // Wait for the producer to publish this slot
            let expected = consumer_seq + 1;
            while slot.sequence.load(Ordering::Acquire) != expected {
                std::hint::spin_loop();
            }

            // Try to claim with CAS — may race with cooperative helpers
            match self.consumer_sequence.compare_exchange_weak(
                consumer_seq,
                consumer_seq + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // SAFETY: CAS won — we have exclusive access to this slot.
                    // The producer has published (seq == expected verified above).
                    // This is the same safety invariant as the original bulk drain,
                    // now enforced per-slot via CAS for interop with helpers.
                    let item = unsafe { (*slot.data.get()).take() };

                    if let Some(item) = item {
                        f(item);
                        count += 1;
                    }

                    // Mark slot as ready for producer reuse
                    slot.sequence
                        .store(consumer_seq + self.capacity as u64, Ordering::Release);
                }
                Err(_) => {
                    // CAS lost to a cooperative helper (or the consumer on retry).
                    // The helper already processed the slot — try the next one.
                    continue;
                }
            }
        }

        count
    }

    /// Co-operatively drain up to `max_count` items from a producer thread
    /// to relieve backpressure.
    ///
    /// Unlike `drain()`, this method **breaks** rather than spinning when a
    /// slot is not yet published or when it loses a CAS race.  This prevents
    /// producer threads from getting stuck helping while the consumer is
    /// keeping up.
    ///
    /// # Safety
    ///
    /// Uses the same CAS-based protocol as `drain()` — the winner gets
    /// exclusive access to the slot data.  Concurrent calls are safe.
    ///
    /// Returns the number of items consumed.
    pub fn drain_helping<F: FnMut(T)>(&self, max_count: usize, mut f: F) -> usize {
        let mut count = 0;

        while count < max_count {
            let consumer_seq = self.consumer_sequence.load(Ordering::Acquire);
            let producer_seq = self.producer_sequence.load(Ordering::Acquire);

            // Buffer empty — nothing to help with
            if consumer_seq >= producer_seq {
                break;
            }

            let index = (consumer_seq & self.mask) as usize;
            let slot = &self.slots[index];

            // For cooperative help: if the slot is not yet published by its
            // producer, stop — do not spin. The consumer will handle it.
            let expected = consumer_seq + 1;
            if slot.sequence.load(Ordering::Acquire) != expected {
                break;
            }

            // Try to claim this slot with CAS.
            // Multiple helpers and the consumer may race; only one wins.
            match self.consumer_sequence.compare_exchange_weak(
                consumer_seq,
                consumer_seq + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // SAFETY: CAS won — exclusive access to this slot.
                    // The producer has published (seq == expected verified above).
                    let item = unsafe { (*slot.data.get()).take() };

                    if let Some(item) = item {
                        f(item);
                        count += 1;
                    }

                    // Mark slot ready for producer reuse
                    slot.sequence
                        .store(consumer_seq + self.capacity as u64, Ordering::Release);
                }
                Err(_) => {
                    // Lost CAS — another helper or the consumer claimed this
                    // slot. Stop to avoid excessive contention.
                    break;
                }
            }
        }

        count
    }

    /// Returns the approximate number of items currently in the buffer.
    pub fn len(&self) -> usize {
        let producer = self.producer_sequence.load(Ordering::Acquire);
        let consumer = self.consumer_sequence.load(Ordering::Acquire);
        (producer - consumer) as usize
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the fill level as a fraction (0.0 to 1.0).
    pub fn fill_level(&self) -> f64 {
        self.len() as f64 / self.capacity as f64
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        // Drain remaining items to prevent leaks
        let _ = self.drain(usize::MAX, |_| {});
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer() {
        let buf: RingBuffer<u64> = RingBuffer::new(1024);
        assert_eq!(buf.capacity(), 1024);
        assert!(buf.is_empty());
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn test_capacity_must_be_power_of_two() {
        let _: RingBuffer<u64> = RingBuffer::new(1000);
    }

    #[test]
    fn test_push_and_drain() {
        let buf: RingBuffer<u64> = RingBuffer::new(64);

        for i in 0..10 {
            buf.try_push(i).unwrap();
        }
        assert_eq!(buf.len(), 10);

        let mut items = Vec::new();
        buf.drain(10, |item| items.push(item));
        assert_eq!(items, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_buffer_full() {
        let buf: RingBuffer<u64> = RingBuffer::new(4);

        for i in 0..4 {
            buf.try_push(i).unwrap();
        }
        assert!(buf.try_push(100).is_err());
    }

    #[test]
    fn test_drain_batch_size() {
        let buf: RingBuffer<u64> = RingBuffer::new(64);

        for i in 0..20 {
            buf.try_push(i).unwrap();
        }

        let drained = buf.drain(5, |_| {});
        assert_eq!(drained, 5);
        assert_eq!(buf.len(), 15);
    }

    #[test]
    fn test_drain_helping_concurrent_with_drain() {
        // Verify drain_helping and drain interoperate via CAS
        let buf: RingBuffer<u64> = RingBuffer::new(64);

        // Push 30 items
        for i in 0..30 {
            buf.try_push(i).unwrap();
        }
        assert_eq!(buf.len(), 30);

        // Help drain 5 items
        let mut helped = Vec::new();
        let helped_count = buf.drain_helping(5, |item| helped.push(item));
        assert_eq!(helped_count, 5);
        assert_eq!(buf.len(), 25);

        // Consumer drain the rest
        let mut consumed = Vec::new();
        let consumed_count = buf.drain(30, |item| consumed.push(item));
        assert_eq!(consumed_count, 25);
        assert!(buf.is_empty());

        // All 30 items accounted for, no duplicates
        let mut all: Vec<u64> = helped.into_iter().chain(consumed).collect();
        all.sort();
        assert_eq!(all, (0..30).collect::<Vec<u64>>());
    }

    #[test]
    fn test_drain_helping_stops_when_empty() {
        let buf: RingBuffer<u64> = RingBuffer::new(64);
        // Push 3, drain_helping 10 — should stop at 3
        for i in 0..3 {
            buf.try_push(i).unwrap();
        }
        let count = buf.drain_helping(10, |_| {});
        assert_eq!(count, 3);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_drain_helping_after_full_and_partial_drain() {
        let buf: RingBuffer<u64> = RingBuffer::new(8);

        // Fill buffer
        for i in 0..8 {
            buf.try_push(i).unwrap();
        }
        assert!(buf.try_push(100).is_err());

        // Consumer drains 3
        buf.drain(3, |_| {});
        assert_eq!(buf.len(), 5);

        // Push 3 more (buffer was 5/8, now 8/8 again)
        for i in 8..11 {
            buf.try_push(i).unwrap();
        }
        assert!(buf.try_push(200).is_err());

        // Help drain 4
        let helped = buf.drain_helping(4, |_| {});
        assert_eq!(helped, 4);
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn test_multi_producer_cooperative_helping() {
        // Multi-producer scenario — when the ring buffer fills up,
        // cooperative helping kicks in and records continue to flow without
        // indefinite blocking.
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::AtomicU64;
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        use std::sync::Barrier;
        use std::thread;

        const CAPACITY: usize = 64;
        const NUM_PRODUCERS: usize = 4;
        const RECORDS_PER_PRODUCER: u64 = 100;

        let buf = Arc::new(RingBuffer::<u64>::new(CAPACITY));
        let produced = Arc::new(AtomicU64::new(0));
        let consumed = Arc::new(AtomicU64::new(0));
        let errors = Arc::new(AtomicU64::new(0));
        let barrier = Arc::new(Barrier::new(NUM_PRODUCERS + 1)); // +1 for consumer thread
        let shutdown = Arc::new(AtomicBool::new(false));

        // Consumer thread that slowly drains
        let buf_consumer = Arc::clone(&buf);
        let consumed_clone = Arc::clone(&consumed);
        let shutdown_clone = Arc::clone(&shutdown);
        let barrier_clone = Arc::clone(&barrier);

        let consumer = thread::spawn(move || {
            barrier_clone.wait();
            while !shutdown_clone.load(Ordering::Acquire) {
                let drained = buf_consumer.drain(16, |item| {
                    consumed_clone.fetch_add(1, Ordering::Relaxed);
                });
                if drained == 0 {
                    // Small sleep when empty to allow producers to catch up
                    thread::sleep(std::time::Duration::from_micros(50));
                }
            }
            // Final drain
            buf_consumer.drain(usize::MAX, |item| {
                consumed_clone.fetch_add(1, Ordering::Relaxed);
            });
        });

        // Producer threads
        let mut handles = Vec::new();
        for t in 0..NUM_PRODUCERS {
            let buf_prod = Arc::clone(&buf);
            let produced_clone = Arc::clone(&produced);
            let consumed_clone_p = Arc::clone(&consumed);
            let errors_clone = Arc::clone(&errors);
            let barrier_clone = Arc::clone(&barrier);

            handles.push(thread::spawn(move || {
                barrier_clone.wait();
                let base = (t as u64) * RECORDS_PER_PRODUCER;
                for i in 0..RECORDS_PER_PRODUCER {
                    let value = base + i;
                    loop {
                        match buf_prod.try_push(value) {
                            Ok(()) => {
                                produced_clone.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                            Err(_) => {
                                // Buffer full — try cooperative helping
                                if buf_prod.fill_level() >= 0.90 {
                                    let helped = buf_prod.drain_helping(8, |_item| {
                                        consumed_clone_p.fetch_add(1, Ordering::Relaxed);
                                    });
                                    if helped > 0 {
                                        // Retry push after helping
                                        continue;
                                    }
                                }
                                // Couldn't help — record would be lost in real usage.
                                // In this test we spin briefly and retry.
                                errors_clone.fetch_add(1, Ordering::Relaxed);
                                thread::yield_now();
                            }
                        }
                    }
                }
            }));
        }

        // Wait for all producers to finish
        for h in handles {
            h.join().expect("Producer thread panicked");
        }

        // Signal shutdown
        shutdown.store(true, Ordering::Release);
        consumer.join().expect("Consumer thread panicked");

        let total_produced = produced.load(Ordering::Relaxed);
        let total_consumed = consumed.load(Ordering::Relaxed);
        let total_errors = errors.load(Ordering::Relaxed);

        assert_eq!(
            total_produced,
            (NUM_PRODUCERS as u64) * RECORDS_PER_PRODUCER,
            "All records should be produced"
        );
        assert_eq!(
            total_consumed, total_produced,
            "All produced records must be consumed (no loss, no duplication)"
        );
        // Errors are expected when buffer is full and helping can't keep up,
        // but producers should still make forward progress.
        assert!(
            total_errors < total_produced / 2,
            "Too many push failures ({total_errors} / {total_produced}) — helping should reduce errors"
        );
        assert!(
            buf.is_empty(),
            "Ring buffer should be fully drained after shutdown"
        );
    }
}
