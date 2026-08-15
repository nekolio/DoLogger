# Architecture Evolution: The Root-Level Difference of Moving Sink from Plugin to Core Built-in

> **Version**: v0.1.0 | **Last updated**: 2026-08-15 | **Audience**: core developers, architecture reviewers, plugin authors
>
> **Purpose**: Using design intent as the lens (the in-repo authority being [ArchitectureReference.md](ArchitectureReference.md)), contrast the pre-refactor (`aedcd7f~1`) and post-refactor (`aedcd7f`) architecture models to answer one question: **what is the *root-level* source of the gap between "final result vs. design proposal"?**
>
> 🌐 **Language / Language**: [English](ArchitectureEvolution.md) | [中文：架构演进](../zh_CN/ArchitectureEvolution.md)

---

## TL;DR

The major architecture change reduces to **a single root decision**:

> **Output execution (Sink) moved from the pluggable plugin domain into the trusted-core domain.**

That one decision simultaneously reshaped the system's **ontology** (what category Sink belongs to), **trust boundary** (is output still protected by the sandbox/trust gate), and **dispatch model** (how output is driven). These three layers are not three independent changes — they are **three projections of one root decision**.

The "final result vs. design proposal" gap is shaped primarily by this decision: the proposal treated Sink as one of 10 plugin types (extensible, sandboxable); the final architecture absorbed it into the core (no VTable, beyond the trust gate) in exchange for a simpler config-driven fan-out and a smaller security surface.

---

## 1. The Root Decision

Commit [`aedcd7f`](https://github.com/nekolio/DoLogger/commit/aedcd7f) "refactor(core): Sink is a core built-in, not a plugin type" is the anchor of this architecture change.

| | Pre-refactor (`aedcd7f~1`) | Post-refactor (`aedcd7f`) |
|:-:|:-:|:-:|
| **Sink's category** | Plugin type #5 (`dologger_iosink_vtable_t`) | Core built-in output executor (stage 6) |
| **Phase bit** | `DO_LOG_PHASE_SINK = 0x0020u` | Removed |
| **Plugin VTable type count** | 10 | 9 |
| **Codified in docs** | Plugin VTable spec includes Sink | [ArchitectureReference.md](ArchitectureReference.md#plugin-vtable-spec)：「Sink 不是插件类型……11 种内置接收器由核心直接驱动」 |

Why does one decision ripple so far? Because Sink sat at the **overlap of two domains**: it was both an **output** (the trusted core must guarantee no-loss and tamper-resistance) and an **extension point** (third parties want custom output). These two properties were previously bound into a single type; the refactor split them apart.

---

## 2. Three Projections

### Projection A — Ontology: what category Sink belongs to

**Before**: Sink was plugin type #5 with its own C ABI VTable, defined in `core/include/dologger_core.h`:

```c
#define DO_LOG_PHASE_SINK        0x0020u

/* --- (5) IOSink VTable --- */
typedef struct dologger_iosink_vtable {
    int      (*open)(...);
    int      (*write)(...);
    int      (*write_batch)(void *instance, const uint8_t *const *data, ...);
    int      (*flush)(...);
    void     (*close)(...);
    uint64_t (*get_last_persisted_id)(void *instance);
} dologger_iosink_vtable_t;
```

It obeyed the exact same lifecycle as every other plugin — load, validate, mount, unmount — constrained as an **untrusted extension** by the trust gate and sandbox.

**After**: Sink is a core built-in output executor with **no VTable**, no plugin lifecycle, and no load/unload as a plugin. Codified in [ArchitectureReference.md](ArchitectureReference.md)：「Sink 不是插件类型：它是核心内置的输出执行器（阶段 6），没有 VTable。11 种内置接收器由核心直接驱动。」

**Essence**: Sink went from "**an extension outside the engine**" to "**part of the engine itself**."

### Projection B — Trust boundary: is output still protected?

**Before**: Sink was a **sandbox-loadable plugin type** in the whitelist. The Red (`SandboxLevel::Isolated`) allowed-type array explicitly included `"IOSink"` (`core/src/plugin/sandbox.rs:281`, comment "Red can only be: Filter, FieldProvider, Processor, Formatter, IOSink"). In other words, third-party custom sinks were a **design-permitted extension surface** — and that extension was sandbox-protected.

**After**: Sink belongs to the trusted core and is no longer part of the plugin-sandbox vocabulary. `aedcd7f` closed the loop in three places:
- `sandbox.rs`: the Red/Isolated allowed-type array drops `"IOSink"` (comment becomes "Filter, FieldProvider, Processor, Formatter only").
- Sandbox tests `tests/security/sandbox_escape/mod.rs`: allowed-type arrays and README drop IOSink in lockstep.
- Test rename: `red_allows_only_render_transform_types` → `red_allows_only_transform_plugin_types` — the semantics narrowed from "render/transform output types" to "transform plugin types."

**Implication**: **Output is no longer protected by the trust gate/sandbox** — it is assumed to be the engine itself. The third-party output-extension channel moved from "plugin vtable" to "config + Callback." This is a **narrowing** of the security surface: one less extension path a malicious plugin could impersonate.

### Projection C — Dispatch model: how output is driven

**Before**: Sink mounted at stage 6 via phase bit `0x0020`, dispatched through the same `resolve_dispatch` vtable path as every other plugin — runtime dynamic function-pointer dereference.

**After**: Sink is driven by the `[sinks.*]` TOML registry (`type` tag) + `FanoutSink` (M4+M5) (`core/src/sink/registry.rs`). Plugin dispatch is left to Formatter / FieldProvider only (M6).

```toml
[sinks.stdout]
type = "console"

[sinks.applog]
type = "file"
path = "/var/log/app.log"
```

**Implication**: **The output path changed from polymorphic vtable calls to config-driven core fan-out** — plug-and-play via config, at the cost of abandoning runtime loading of third-party sinks.

### How the three projections relate

```
          Root decision: output execution (Sink) moves plugin domain → core domain
                                  │
             ┌────────────────────┼────────────────────┐
             │                    │                    │
         Projection A          Projection B        Projection C
          Ontology             Trust boundary       Dispatch model
       Sink's category       output no longer      output becomes
        changes              sandbox-protected      config fan-out
       plugin #5 → builtin   (surface narrows)      vtable → [sinks.*]
```

---

## 3. Before / After Comparison Table

| Dimension | Pre-refactor | Post-refactor | Gap class |
|:-:|:-:|:-:|:-:|
| Sink category | Plugin type #5 | Core built-in (stage 6) | Root decision |
| C ABI | `dologger_iosink_vtable_t` | Removed | Root decision |
| Phase bit | `DO_LOG_PHASE_SINK = 0x0020u` | Removed | Root decision |
| Plugin VTable type count | 10 | 9 | Root decision |
| Sandbox whitelist | Red allows IOSink | IOSink removed | Root decision (trust projection) |
| Dispatch | VTable dynamic dispatch | `[sinks.*]` + `FanoutSink` | Root decision (dispatch projection) |
| Sandbox test name | `red_allows_only_render_transform_types` | `red_allows_only_transform_plugin_types` | Root decision |
| Third-party output channel | Plugin vtable | Config + Callback | Root decision |

---

## 4. Derived Gaps (annotated by class)

> Two classes are distinguished here: **"pre/post-architecture" gaps** (produced directly by the root decision) and **"design-vs-implementation" gaps** (design intent decided but not yet implemented/fully implemented). The latter is **out of scope** for this refactor; it is annotated here for next-phase planning.

### 4.1 "Pre/Post-architecture" gaps (this document's core)

1. **Plugin types 10 → 9**: the `PHASE_SINK` bit is removed and `dologger_iosink_vtable_t` disappears from the C ABI. → Produced directly by the root decision.
2. **Only Formatter / FieldProvider are actually dispatched today**: the remaining seven — `Filter`/`Processor`/`ConfigProvider`/`KeyProvider`/`PolicyProvider`/`HostInfoProvider`/`SyscallBroker` — have vtables (`dologger_core.h` + the 9-type table in ArchitectureReference) but are not wired to pipeline stages. → This is a **design-decided, implementation-pending** gap (see 4.2).

### 4.2 "Design-vs-implementation" gaps (next-phase scope)

> These are points the design docs (ArchitectureReference / proposal intent) promise but the current implementation has not yet reached. They are **not** regressions introduced by the Sink refactor, but missing core items to fill.

1. **Seven plugin types undispatched**: Filter(Stage1), Processor(Stage4), Config/Key/Policy/HostInfo/SyscallBroker stage hooks unwired. → Next step **Batch A** scope.
2. **Parallel io_pool fan-out + fallback chain + circuit breaker**: the doc-described parallel fan-out and fallback chain are sequential (or partially unimplemented) in the current `FanoutSink`.
3. **Sandbox (seccomp-bpf/AppContainer) not implemented**: currently only the Ed25519 signing trust gate exists. Blue/Yellow/Red trust colors belong to the trust-gate concept and remain as design intent.
4. **Formatting→Sink SIF handoff is under planning** ([ArchitectureReference.md](ArchitectureReference.md)).

---

## 5. Why This Shapes the "Final Result vs. Proposal" Gap

The proposal's original design treated Sink as one of 10 plugin types — **extensible, sandboxable, runtime-loadable**. The final architecture absorbed it into the core — **non-extensible vtable, beyond the trust gate, config-driven**.

The **trade-off** of this convergence direction:

| Proposed Sink | Final-architecture Sink | Gain | Loss |
|:-:|:-:|:-:|:-:|
| Pluginized, sandboxable | Core built-in, config-driven | Simpler output path, narrower security surface | Gives up runtime third-party sinks |
| 10 plugins | 9 plugins | Cleaner ontology (output ≠ extension) | One more core built-in concept |

**Key insight**: this is not a gap caused by "implementation unfinished" — it is a **deliberate choice made by the architecture intent itself**. The proposal wanted "extensible output"; the final architecture chose "trusted output." This decision is **root-level** — it is no longer a feature that can be retrofitted later, but one that reshaped the entire trust model and extension model.

To close the gap with the proposal, the correct direction is **not** to make Sink a plugin again, but to complete the remaining work *within* the "core built-in" premise: config fan-out completeness, fallback chain, circuit breaker, and truly dispatching the other seven plugin types.

---

## References

- English version: [Architecture Evolution](ArchitectureEvolution.md)
- Design-intent authority: [ArchitectureReference.md](ArchitectureReference.md)
- Refactor commit: `aedcd7f` "refactor(core): Sink is a core built-in, not a plugin type"
- Related implementation notes: [[plugin-m6-dispatch]], [[ffi-field-access-ring3-only]]
