# SIF — Standard Intermediate Format

SIF is DoLogger's neutral serialization boundary for records that cross a
process, shared-memory, file, plugin, or language boundary. It is not a sink
and it is not a display encoding.

The in-memory `Record` uses fixed hot fields plus dynamic KV fields. The SIF
codec serializes that model into a bounded little-endian frame:

```text
SIF magic | header length | flags | total length | KV count | fixed length
fixed record metadata | raw message bytes | KV entries
```

KV is the data organization inside the frame; SIF is the transport-neutral
serialization boundary. In-process sinks may bypass SIF and consume `Record`
or an immutable derived view directly.

## Rust API

The public API is in `dologger_core::sif`:

- `encode_record` / `decode_record` serialize and restore one `Record`;
- `validate_frame` checks framing and resource bounds without materializing a record;
- `entries` exposes bounded borrowed KV entries for inspection;
- `FrameScanner` handles length-prefixed fragmented streams;
- `ReusableEncoder` avoids repeated output allocations for one producer.

The default codec preserves raw message bytes, validates field names and lengths,
and optionally verifies the canonical content hash. The explicit FlatBuffers
backend is available as `encode_record_flatbuffers` and
`decode_record_flatbuffers`; it uses the same Record/KV semantics and is useful
for zero-copy-compatible or cross-language consumers. Neither backend performs
display encoding, localization, or audit activation.

## Backend roles

```text
Record (fixed fields + KV + raw bytes)
             |
             +-- Native KV-backed SIF (default, bounded and allocation-aware)
             +-- FlatBuffers SIF (explicit backend, schema-driven access)
```

KV is the data organization model inside Record and inside each backend's
payload. SIF is the neutral serialization boundary. FlatBuffers is a
serialization technology used to implement that boundary; it is not a second
Record model and it does not make audit mandatory.

## C and other hosts

The C ABI can request a SIF frame through the existing SIF encode/decode
functions. Other language bindings should consume the documented frame layout
and use the same little-endian bounds checks. No host is required to use SIF
for an in-process sink.
