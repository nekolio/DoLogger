# SIF — Standard Intermediate Format

The **SIF (Standard Intermediate Format)** is the zero-copy binary wire format
that sits between the **Formatter** and **Sink** stages in the DoLogger pipeline.
It uses [FlatBuffers](https://flatbuffers.dev/) to provide a schema-driven,
language-neutral representation of log records that can be consumed without
deserialisation or heap allocation.

## Role in the Pipeline

```
┌─────────┐     ┌───────────┐     ┌──────────┐     ┌──────────┐
│  App    │ ──> │  Core     │ ──> │ Formatter│ ──> │  Sink    │
│ (log!)  │     │ (Record)  │     │ ( → SIF) │     │ (SIF → ) │
└─────────┘     └───────────┘     └──────────┘     └──────────┘
                                       │
                                 SIF lives here
                             (zero-copy handoff)
```

1. **Core Engine** populates an in-memory `Record` struct.
2. **Formatter** serialises the `Record` into a SIF FlatBuffer.
3. **Sink** receives the SIF byte slice and reads fields directly via
   FlatBuffers' accessor methods — no parsing, no allocation, no copies.
4. After the Sink returns, the `Record` is recycled back to the object pool.

## Zero-Copy Design Philosophy

FlatBuffers stores data in a **wire-ready** format.  Fields are accessed by
following relative offsets from the root pointer; strings and vectors are
referenced in-place.  This means:

- **No deserialisation step** — the Sink calls `record.message()` and gets a
  `&str` that points directly into the SIF buffer.
- **No intermediate allocations** — SIF buffers can be stack-allocated or
  pulled from a dedicated buffer pool.
- **Schema evolution** — new fields can be added to the `Record` table without
  breaking existing Sinks; old consumers simply ignore unknown fields.
- **Language neutrality** — the same `.fbs` schema can generate C, C++, Rust,
  Go, Python, Java, and TypeScript bindings.

## Schema File

- **`dologger_sif.fbs`** — the canonical FlatBuffers schema defining the
  `Record` table with all Ring 0 through Ring 3 fields.

## Generating Rust Bindings

Install the FlatBuffers compiler (`flatc`):

```bash
# macOS
brew install flatbuffers

# Ubuntu / Debian
apt install flatbuffers-compiler

# Or build from source
git clone https://github.com/google/flatbuffers.git
cd flatbuffers && cmake -G "Unix Makefiles" && make -j$(nproc)
```

Generate Rust code from the schema:

```bash
cd core/sif
flatc --rust -o ../src/sif/ dologger_sif.fbs
```

This produces:
- `core/src/sif/dologger_sif_generated.rs` — the generated FlatBuffers code

`core/build.rs` regenerates this file automatically whenever `flatc` is on
`PATH`, and the committed copy keeps builds that lack `flatc` working. The
generated code is already wired in via an active `include!` (module
`generated`) in `core/src/sif/mod.rs` — no manual `include!` uncommenting is
required.

## Integration Point

The generated bindings land in `core/src/sif/dologger_sif_generated.rs`,
pulled in by an active `include!` (module `generated`) in `mod.rs`. The
hand-written `mod.rs` exposes the public frame API:

| Item              | Purpose                                                       |
|-------------------|---------------------------------------------------------------|
| `SIF_MAGIC`       | 4-byte magic constant (`b"SIF1"`) for frame validation        |
| `SifHeader`       | 12-byte framing header (version, total_length, record_count)  |
| `encode_record`   | Serialise a `Record` into a framed SIF byte buffer (encoder)  |
| `decode_record`   | Parse a framed SIF buffer back into a `Record` (decoder)      |
| `validate_frame`  | Check the magic + header consistency of a SIF buffer          |
| `SifError`        | Typed error for decode / validate failures                    |

The FlatBuffers internals stay crate-private; `encode_record` /
`decode_record` / `validate_frame` are the stable public API.

## Status — v0.0.1

The wire format and the `encode_record` / `decode_record` /
`validate_frame` API are delivered and already used by the shared-memory
sink and the `dologger` CLI (`record` / `verify`). The **pipeline handoff**
(Formatter → Sink carrying SIF frames) is planned: the engine's Formatting
stage still produces plain text internally, so the Formatter→SIF→Sink
path above describes the target design rather than today's wiring.

## SIF Wire Format (Frame Layout)

```
┌──────────┬──────────┬─────────────────────────────────────┐
│  Magic   │  Header  │  FlatBuffer (Record table)          │
│  4 bytes │ 12 bytes │  variable length                    │
├──────────┼──────────┼─────────────────────────────────────┤
│ "SIF1"   │ version  │  ┌──────────────────────────────┐   │
│          │ total_len│  │ root table offset (u32)      │   │
│          │ count    │  │ vtable + data                │   │
│          │          │  └──────────────────────────────┘   │
└──────────┴──────────┴─────────────────────────────────────┘
```

- **Magic**: `b"SIF1"` — identifies the stream as a DoLogger SIF payload.
- **Header**: 12-byte `SifHeader` struct with version, total_length, record_count.
- **Payload**: FlatBuffer-encoded `Record` table, starting with the root offset.

## Versioning

| Schema Version | SIF Magic | Changes                                           |
|----------------|-----------|---------------------------------------------------|
| 1.0.0          | `SIF1`    | Initial schema with Ring 0–3, all Record fields   |

The `SifHeader.version` field encodes the schema version as a `u32` in
`MAJOR << 24 | MINOR << 16 | PATCH` format, so consumers can negotiate
compatibility at runtime.

## References

- [FlatBuffers: Writing a schema](https://flatbuffers.dev/flatbuffers_guide_writing_schema.html)
- [FlatBuffers: Rust usage](https://flatbuffers.dev/flatbuffers_guide_use_rust.html)
- [DoLogger Architecture Reference — SIF Binary Format Overview](../../docs/en_US/ArchitectureReference.md#sif-binary-format-overview)
