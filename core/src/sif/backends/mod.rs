//! Concrete implementations of the SIF serialization boundary.
//!
//! The native codec remains the default implementation exposed by
//! [`crate::sif`]. FlatBuffers is an explicit backend for zero-copy readers,
//! cross-language consumers, and integrations that already use FlatBuffers.

pub mod flatbuffers;
