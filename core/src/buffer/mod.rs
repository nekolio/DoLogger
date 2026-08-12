//! Lock-free data structures for the DoLogger core.
//!
//! This module contains the ring buffer, object pool, and emergency buffer
//! used for high-performance, low-latency log record handling.

pub mod emergency_buffer;
pub mod object_pool;
pub mod ring_buffer;

pub use emergency_buffer::{EmergencyBuffer, EmergencyPushResult, EmergencyStats};
pub use object_pool::RecordPool;
pub use ring_buffer::{RingBuffer, DEFAULT_CAPACITY};
