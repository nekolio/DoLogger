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

The codec preserves raw message bytes, validates field names and lengths,
and optionally verifies the canonical content hash. It does not perform display
encoding, localization, or audit activation.

## C and other hosts

The C ABI can request a SIF frame through the existing SIF encode/decode
functions. Other language bindings should consume the documented frame layout
and use the same little-endian bounds checks. No host is required to use SIF
for an in-process sink.
