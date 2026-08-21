# Core Module Architecture

> Architecture review baseline: local `main` at `87b8a7b`, plus the uncommitted
> record-safety, error-code, codec, localization, and resource-layout work on
> 2026-08-21. No remote state was used as implementation evidence.

## Decision summary

Encoding and decoding are **core capabilities**, not localization features and
not ordinary dynamic plugins. The core must provide deterministic, validated
codec contracts because the same boundary is used by platform output, FFI
text, configuration values, catalog input, and future non-UTF-8 adapters.

Localization is a consumer of core codec services at the human-facing display
boundary. It owns locale selection, catalog lookup, fallback, and translation;
it does not own code-page policy. Persisted records, SIF/KV data, WORM
containers, hashes, signatures, and audit-chain bytes remain canonical and
never pass through localization or display transcoding.

AUDIT remains an **opt-in use case**. It is placed under `security/audit` to
make its security boundary explicit, not to make audit logging default or
mandatory.

## Canonical module map

```text
core/src/
├── buffer/          ownership tokens, pools, rings, emergency memory
├── codec/           core text encode/decode and platform detection
├── config/          configuration model, validation, watcher, hot reload
├── error.rs         stable numeric error descriptors and fallback messages
├── ffi.rs           C ABI boundary and last-error path
├── localization/    locale chain, catalogs, fallback registry
├── pipeline/        stages, scheduling, backpressure, admission policy
├── plugin/          loading, ABI validation, sandbox, quotas, dispatch
├── record/          hot-path record and KV representation
├── security/        crypto, keys, TPM boundary, secret detection, audit
├── sif/             structured persistence format codecs
├── sink/            output sinks, including WORM/security sinks
├── sys/             OS services, I/O, diagnostics, control plane
└── util/            small dependency-free helpers
```

Compatibility aliases remain for existing Rust callers:

| Legacy path | Canonical path |
|---|---|
| `dologger_core::encoding` | `dologger_core::codec` |
| `dologger_core::i18n` | `dologger_core::localization` |
| `dologger_core::policy` | `dologger_core::pipeline::policy` |
| `dologger_core::audit` | `dologger_core::security::audit` |

New code must use canonical paths. The aliases are not new architecture.

## Why codec is not a plugin

A dynamic plugin is untrusted, versioned, and independently deployable. That
is appropriate for optional presentation and pipeline behavior, but unsafe as
the authority for canonical bytes. If a codec plugin controlled audit or
persistence bytes, it could change hashes, signatures, replay semantics, or
cross-platform verification results. It would also turn a basic core operation
into a startup dependency and add ABI, allocation, and failure overhead to
hot paths.

The core codec boundary therefore owns:

- UTF-8 as the canonical text representation;
- explicit Windows code-page conversion with range validation;
- locale/codeset parsing and platform probes;
- lossless conversion policy and invalid-byte rejection;
- stable error types for callers and future FFI adapters.

A future internal codec trait may allow built-in backends to be selected by
platform or feature. That is an implementation detail, not permission for an
external plugin to redefine canonical serialization.

## Where plugins may participate

| Extension | Allowed role | Forbidden role |
|---|---|---|
| Formatter | Human-facing presentation after record processing | Rewriting canonical audit/persistence bytes |
| Filter / processor | Explicit pipeline behavior | Bypassing security admission or ownership rules |
| Catalog provider (future ABI) | Supply validated locale entries | Changing error codes or localizing audit bytes |
| Codec backend (future, reviewed) | Explicit display/config conversion | FFI layout, SIF/KV, WORM, hash, or signature encoding |

The catalog provider is a localization extension. A codec backend, if ever
needed, is a separate reviewed capability and must not be smuggled in as a
formatter plugin.

## Runtime boundaries

1. A record enters the core in canonical Rust/FFI representation.
2. Pipeline stages apply policy and plugins under their declared phase.
3. Persistence and security sinks serialize canonical bytes and complete hashes
   or signatures without localization.
4. Human-facing output resolves an error/message key through localization.
5. The output layer asks the core codec for the selected display encoding.
6. `sys::io` writes the resulting bytes or uses the platform Unicode console
   API. Redirected/file output remains UTF-8.

No producer hot path performs catalog lookup or locale-dependent conversion.

## Migration and open work

- `core/src/codec/` currently provides the text codec scaffold and common code
  page handling; POSIX/macOS native codeset backends remain TODO(author).
- `core/src/localization/` currently provides validated in-memory catalogs;
  Fluent-compatible compilation, bounded reload, and catalog provider ABI
  remain TODO(author).
- `core/src/sif/` remains a persistence-format boundary; the planned SIF → KV
  migration is separate from display encoding.
- `security/tpm.rs` still needs a real provider before hardware-backed claims
  are made.
- Compatibility aliases can be removed only in a planned major API change.

See [Localization Architecture](LocalizationArchitecture.md),
[ADR-006](../../../.agents/living/decisions/ADR-006-localization-architecture.md),
and [ADR-007](../../../.agents/living/decisions/ADR-007-core-module-boundaries.md).
