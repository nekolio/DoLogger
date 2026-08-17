# DoLogger Development Progress

> **Live completion-degree record** — updated at the end of each
> development round per the [[per-round-checklist]] convention.
> Last updated: 2026-08-17 (WS-3 hot reload wiring COMPLETE: `[watcher]`
> config section, `ConfigWatcher` with native Windows RDCW + Linux inotify
> backends, swappable `SinkRef` + `Engine::reload_config`, `dologctl run`
> wiring, end-to-end reload tests).

## Legend

| Symbol | Meaning |
|:-:|:--|
| ✅ | Complete |
| 🟡 | Partial |
| ⛔ | Implemented but not wired into Engine::init |
| 🔴 | Gap / missing |

## Overall completion (2026-08-17)

| Dimension | Completion | Top gap |
|:-:|:-:|:--|
| Framework | ✅ ~100% | 3 unwired modules (hot_reload / control_plane / host_info) |
| Functionality | ✅ ~90% | the same 3 + remote sinks depth (WS-4 audit) |
| Details | 🟡 ~80% | 3× `allow(missing_docs)`, shm `allow(dead_code)`, `HotReloadManager` unwired |
| Tests | 🟡 ~60% | CLI 6/7 command modules untested; fuzz never run; C adapter 0 tests |
| Periphery | 🟡 ~50% | Go/Python/C adapters no CI; no pyproject |
| Docs | ✅ ~95% | 28 "not implemented" markers to clear as features land |

## core (`dologger-core`) — 34 modules, 325 tests

| Subsystem | Framework | Functionality | Details | Tests | Wired | Notes |
|:-:|:-:|:-:|:-:|:-:|:-:|:--|
| lib.rs Engine | ✅ | ✅ init/shutdown | ✅ | — | ✅ | `build_fanout → Pipeline::new → AuditPipeline::new` |
| audit.rs | ✅ | ✅ dual-write | ✅ | inline | ✅ | enabled when `enable_signature` |
| error.rs | ✅ | ✅ 77 codes | ✅ | 3 | — | WS-1 new 14-domain scheme |
| ffi.rs | ✅ | ✅ 13 functions | 🟡 | 6 | ✅ | `#![allow(missing_docs)]` TODO |
| policy.rs | ✅ | ✅ RateLimiter+DropLevel | ✅ | — | ✅ | |
| record/ | ✅ | ✅ FieldRing 0-3 | ✅ | 10 | ✅ | |
| buffer/ | ✅ | ✅ ring/pool/emergency | ✅ | 19 | ✅ | |
| config/ | ✅ | ✅ settings/domain + watcher | 🟡 | 21 | ✅ | native RDCW/inotify + polling; `[watcher]` wired |
| pipeline/ | ✅ | ✅ scheduler/stages | ✅ | 22 | ✅ | circuit_breaker/canary/backpressure present |
| plugin/ | ✅ | ✅ manager/sandbox/vtable | 🟡 | 25 | ✅ | sandbox.rs `allow(missing_docs)` |
| security/ | ✅ | ✅ sig/key_rot/external_anchor | 🟡 | 29 | ✅ | key_rotation `allow(missing_docs)` |
| sif/ | ✅ | ✅ encode/decode/generated | ✅ | 18 | ✅ | FlatBuffer codegen committed |
| sink/ | ✅ | ✅ 13 submodules | 🟡 | 18 | ✅ shm wired | shm.rs `allow(dead_code)` removed |
| sys/ | ✅ | ✅ control_plane/host_info | 🟡 | 11 | ⛔ **control_plane unwired** | native watcher backend TODO |
| util/hex | ✅ | ✅ new in WS-6 | ✅ | 9+6doc | ✅ | replaces hex crate |

### Critical unwired modules (all have code + tests, just not in Engine::init)

1. `HotReloadManager` — [hot_reload.rs](../../core/src/config/hot_reload.rs) never instantiated (reload currently driven by `ConfigWatcher` + `Engine::reload_config`)
2. `ControlPlane` — [control_plane.rs](../../core/src/sys/control_plane.rs) /status hardcoded placeholder
3. `HostInfoProvider` — [host_info.rs](../../core/src/sys/host_info.rs) not in Engine

## CLI (`dologctl`) — 15 commands, thin tests

| Command | Impl | Text | JSON | Tests | Core integration |
|:-:|:-:|:-:|:-:|:-:|:-:|
| run --trace | ✅ | ✅ | — | — | ✅ Engine |
| run (steady) | ✅ | ✅ | — | 3 (run_smoke) | ✅ Engine |
| plugin (10 actions) | ✅ | ✅ | 🔴 | 6 (inline) | ✅ PluginManager |
| config validate | ✅ | ✅ | 🔴 | — | ✅ |
| verify-log/anchor/recovery | ✅ | ✅ | ✅ | 🔴 | 🟡 SIF decode only |
| record/replay/record-stop | ✅ | ✅ | ✅ | 🔴 | ✅ SIF |
| shm status/clear | ✅ | ✅ | ✅ | 🔴 | ✅ `read_status` (core API) |
| perf | ✅ | ✅ | ✅ | 🔴 | ✅ RecordPool+RingBuffer |
| init/about/version/completions | ✅ | ✅ | — | — | — |

### CLI gaps

- `--output json` silently ignored for plugin/config/run/init
- `replay --speed` accepts any string, silently falls back to max
- **Test coverage LOW**: 6/7 command modules have 0 tests; JSON output untested
- `shm status/clear` now reuse core `read_status` (single source of truth); still thin tests

## plugins / adapters / fuzz

| Component | State | Tests | CI | Notes |
|:-:|:-:|:-:|:-:|:--|
| formatter_text | ✅ | 6 | ✅ | |
| formatter_json | ✅ | 14 | ✅ | |
| filter_level | ✅ | 17 | ✅ | CI single-threaded (global statics) |
| field_container | ✅ | 7 | ✅ | cgroup detection |
| bundle (4-in-1 cdylib) | ✅ | 4+5 dlopen | ✅ release build+sign | all 4 plugins registered |
| adapters/rust SDK | ✅ | 6+5 | ✅ workspace | log/tracing/slog facades maintained |
| adapters/go | ✅ | 5 | 🔴 no CI | needs prebuilt core lib |
| adapters/python | ✅ | 4 | 🔴 no CI | no pyproject (not a package) |
| adapters/c | ✅ header-only | 🔴 0 tests | 🔴 | no Makefile/CI |
| core/fuzz 3 targets | ✅ implemented | 51 edge tests | 🔴 no CI | **no artifacts/ dir (never run)** |

## docs — 22 files × 2 languages, perfect 1:1

- **Bilingual 1:1**: 22 EN ↔ 22 zh-CN all MATCH, no orphans
- **CLI coverage**: all 25 subcommands/sub-actions have docs sections
- **Error code coverage**: error.rs 73 codes → ErrorCodesReference 1:1
- **docs/README** accurate and fresh; site README accurate
- **"not implemented" markers**: ~26, all honest v0.1.0 feature-gap descriptions (sandbox enforcement / daemon mode / KeyProvider / health endpoint / Ring 2 field signing / per-stage perf breakdown) — clear as WS-3/4 land

## Workstream status

| WS | Topic | Status |
|:-:|:--|:--|
| WS-1 | Error code system (14-domain) | ✅ complete |
| WS-6 | hex + hostname native replacement | ✅ complete |
| WS-6 pre | real `dologctl run` loop | ✅ complete |
| WS-2 | sink_shm wiring | ✅ complete |
| WS-3 | hot_reload wiring | ✅ complete |
| WS-4 | remote sinks (Kafka/Syslog/Webhook) | ⛔ pending |
| WS-5 | docs/code consistency cleanup | ⛔ pending |
| WS-6A | `rand` replacement | ⛔ candidate |
| WS-6B | `crossbeam-channel` replacement | ⛔ candidate |
| WS-6C | `serde_json` replacement (CLI) | ⛔ candidate |
| WS-6D | `clap`/`clap_complete` replacement | ⛔ candidate |
