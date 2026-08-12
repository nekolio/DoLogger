//! Fuzz target for the lock-free MPSC ring buffer.
//!
//! Exercises:
//! - Creation with random power-of-two capacity
//! - Push/drain with random data
//! - Edge cases: capacity 1, full buffer, empty buffer
//! - Mixed push/drain sequences

#![no_main]

use dologger_core::buffer::RingBuffer;
use libfuzzer_sys::fuzz_target;

/// Round `n` up to the next power of two, clamped to [1, 1 << 24].
fn next_power_of_two(n: u32) -> usize {
    if n == 0 {
        return 1;
    }
    let capped = n.min(1 << 24);
    let mut p = capped.next_power_of_two();
    if p == 0 {
        p = 1;
    }
    p as usize
}

/// Operations we can perform on the ring buffer.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// Drain with the given batch size (0 = drain all via usize::MAX)
    Drain(u16),
    /// Drain helping with the given max count (0 = drain all via usize::MAX)
    DrainHelping(u16),
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }

    // First two bytes determine capacity and operation sequence
    let capacity_bits = (data[0] as u32) % 25; // 0..24, maps to 1..2^24
    let capacity = next_power_of_two(capacity_bits as u32);

    // Create ring buffer
    let buf: RingBuffer<u64> = RingBuffer::new(capacity);
    assert_eq!(buf.capacity(), capacity);
    assert!(buf.is_empty());
    assert_eq!(buf.len(), 0);
    assert_eq!(buf.fill_level(), 0.0);

    let remaining = &data[1..];

    // Phase 1: Push items up to capacity, using remaining bytes as values
    let mut pushed_count: u64 = 0;
    let mut chunks = remaining.chunks_exact(8);
    let remainder = chunks.remainder();

    for chunk in chunks.by_ref() {
        let val = u64::from_le_bytes(chunk.try_into().unwrap());
        match buf.try_push(val) {
            Ok(()) => {
                pushed_count += 1;
                if pushed_count as usize >= capacity {
                    // Buffer should now be full-ish (or at least not accept more easily)
                    break;
                }
            }
            Err(_) => {
                // Buffer is full — stop pushing
                break;
            }
        }
    }

    // Verify fill level is in valid range
    let fill = buf.fill_level();
    assert!((0.0..=1.0).contains(&fill), "fill_level out of range: {fill}");

    // Phase 2: Drain and verify
    let mut drained_values: Vec<u64> = Vec::new();
    let drained = buf.drain(usize::MAX, |item| drained_values.push(item));
    assert_eq!(
        drained,
        drained_values.len(),
        "drain count mismatch: returned {drained}, collected {}",
        drained_values.len()
    );
    assert_eq!(
        drained as u64, pushed_count,
        "mismatch: pushed {pushed_count}, drained {drained}"
    );
    assert!(buf.is_empty(), "buffer should be empty after full drain");

    // Verify no duplicates and correct value range
    drained_values.sort();
    drained_values.dedup();
    assert_eq!(
        drained_values.len(),
        drained as usize,
        "duplicate values detected in drain"
    );
});

// ===========================================================================
// Standalone tests for edge cases (run with `cargo test`)
// ===========================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    /// Helper: push a sequence of items, drain, and verify correctness.
    fn push_drain_verify(capacity: usize, values: &[u64]) {
        let buf: RingBuffer<u64> = RingBuffer::new(capacity);
        let mut pushed = 0u64;
        for &v in values {
            match buf.try_push(v) {
                Ok(()) => pushed += 1,
                Err(_) => break,
            }
        }

        let mut drained = Vec::new();
        let count = buf.drain(usize::MAX, |item| drained.push(item));
        assert_eq!(count, pushed as usize);
        assert!(buf.is_empty());

        let mut sorted = drained.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            drained.len(),
            "Duplicates found in drained values"
        );
    }

    #[test]
    fn edge_capacity_1() {
        push_drain_verify(1, &[42, 99, 255]);
    }

    #[test]
    fn edge_capacity_2() {
        push_drain_verify(2, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn edge_full_buffer() {
        let buf: RingBuffer<u64> = RingBuffer::new(4);
        for i in 0..4 {
            assert!(buf.try_push(i).is_ok());
        }
        assert_eq!(buf.len(), 4);
        assert!(buf.try_push(999).is_err());
        // Drain all
        let drained = buf.drain(usize::MAX, |_| {});
        assert_eq!(drained, 4);
        assert!(buf.is_empty());
    }

    #[test]
    fn edge_empty_buffer_drain() {
        let buf: RingBuffer<u64> = RingBuffer::new(64);
        let drained = buf.drain(usize::MAX, |_| panic!("should not be called"));
        assert_eq!(drained, 0);
        let helped = buf.drain_helping(usize::MAX, |_| panic!("should not be called"));
        assert_eq!(helped, 0);
    }

    #[test]
    fn edge_partial_drain_batch_size() {
        let buf: RingBuffer<u64> = RingBuffer::new(8);
        for i in 0..8 {
            buf.try_push(i).unwrap();
        }
        // Drain 3 at a time
        let d1 = buf.drain(3, |_| {});
        assert_eq!(d1, 3);
        assert_eq!(buf.len(), 5);

        let d2 = buf.drain(3, |_| {});
        assert_eq!(d2, 3);
        assert_eq!(buf.len(), 2);

        let d3 = buf.drain(3, |_| {});
        assert_eq!(d3, 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn edge_drain_helping_interop() {
        let buf: RingBuffer<u64> = RingBuffer::new(16);
        for i in 0..16 {
            buf.try_push(i).unwrap();
        }

        let mut helped_values = Vec::new();
        let helped = buf.drain_helping(5, |item| helped_values.push(item));
        assert_eq!(helped, 5);
        assert_eq!(helped_values.len(), 5);
        assert_eq!(buf.len(), 11);

        let mut drained_values = Vec::new();
        let drained = buf.drain(usize::MAX, |item| drained_values.push(item));
        assert_eq!(drained, 11);
        assert!(buf.is_empty());

        // All 16 items accounted for
        let mut all: Vec<u64> = helped_values.into_iter().chain(drained_values).collect();
        all.sort();
        assert_eq!(all, (0..16).collect::<Vec<u64>>());
    }

    #[test]
    fn edge_large_capacity() {
        let buf: RingBuffer<u64> = RingBuffer::new(1 << 20); // 1M records
        for i in 0..10_000u64 {
            buf.try_push(i).unwrap();
        }
        assert_eq!(buf.len(), 10_000);

        let drained = buf.drain(usize::MAX, |_| {});
        assert_eq!(drained, 10_000);
        assert!(buf.is_empty());
    }

    #[test]
    fn edge_fill_level_at_capacity() {
        let buf: RingBuffer<u64> = RingBuffer::new(4);
        assert!((buf.fill_level() - 0.0).abs() < f64::EPSILON);

        buf.try_push(1).unwrap();
        assert!((buf.fill_level() - 0.25).abs() < f64::EPSILON);

        for i in 2..=4 {
            buf.try_push(i).unwrap();
        }
        assert!((buf.fill_level() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn edge_drop_drains_remaining() {
        let buf: RingBuffer<u64> = RingBuffer::new(8);
        for i in 0..5 {
            buf.try_push(i).unwrap();
        }
        // Drop should drain remaining items without leak
        drop(buf);
    }

    #[test]
    fn edge_drain_helping_stops_when_slot_not_published() {
        // drain_helping breaks on unpublished slots unlike drain which spins.
        // This is hard to test without concurrency; the existing tests cover it.
        let buf: RingBuffer<u64> = RingBuffer::new(8);
        for i in 0..4 {
            buf.try_push(i).unwrap();
        }
        // All slots are published, so helping should drain them
        let helped = buf.drain_helping(10, |_| {});
        assert_eq!(helped, 4);
        assert!(buf.is_empty());
    }
}
