# DoLogger 开发进度记录

> **实时完成度记录** —— 每轮开发结束时按 [[per-round-checklist]] 规范更新。
> 最近更新：2026-08-17（WS-3 热重载接线完成：`[watcher]` 配置段、`ConfigWatcher` 原生 Windows RDCW + Linux inotify 后端、可交换 `SinkRef` + `Engine::reload_config`、`dologctl run` 接线、端到端重载测试）。

## 图例

| 符号 | 含义 |
|:-:|:--|
| ✅ | 完整 |
| 🟡 | 部分 |
| ⛔ | 已实现但未接入 Engine::init |
| 🔴 | 缺口 / 缺失 |

## 总览（2026-08-17）

| 维度 | 完成度 | 主要缺口 |
|:-:|:-:|:--|
| 框架搭建 | ✅ ~100% | 3 个未接线模块（hot_reload / control_plane / host_info） |
| 功能实现 | ✅ ~90% | 同上 3 个 + 远程 sink 深度（WS-4 审计） |
| 细节 | 🟡 ~80% | 3 处 `allow(missing_docs)`、shm `allow(dead_code)`、`HotReloadManager` 未接线 |
| 测试 | 🟡 ~60% | CLI 6/7 命令模块无测试；fuzz 从未运行；C adapter 0 测试 |
| 外围 | 🟡 ~50% | Go/Python/C adapter 无 CI；无 pyproject |
| 文档 | ✅ ~95% | 28 处 "not implemented" 标记待随功能落地清除 |

## core（`dologger-core`）— 34 模块，325 测试

| 子系统 | 框架 | 功能 | 细节 | 测试 | 接线 | 备注 |
|:-:|:-:|:-:|:-:|:-:|:-:|:--|
| lib.rs Engine | ✅ | ✅ init/shutdown | ✅ | — | ✅ | `build_fanout → Pipeline::new → AuditPipeline::new` |
| audit.rs | ✅ | ✅ 双写消费 | ✅ | 内联 | ✅ | enable_signature 时启用 |
| error.rs | ✅ | ✅ 77 码 | ✅ | 3 | — | WS-1 新 14 域体系 |
| ffi.rs | ✅ | ✅ 13 函数 | 🟡 | 6 | ✅ | `#![allow(missing_docs)]` TODO |
| policy.rs | ✅ | ✅ RateLimiter+DropLevel | ✅ | — | ✅ | |
| record/ | ✅ | ✅ FieldRing 0-3 | ✅ | 10 | ✅ | |
| buffer/ | ✅ | ✅ ring/pool/emergency | ✅ | 19 | ✅ | |
| config/ | ✅ | ✅ settings/domain + watcher | 🟡 | 21 | ✅ | 原生 RDCW/inotify + polling；`[watcher]` 已接线 |
| pipeline/ | ✅ | ✅ scheduler/stages | ✅ | 22 | ✅ | circuit_breaker/canary/backpressure 全在 |
| plugin/ | ✅ | ✅ manager/sandbox/vtable | 🟡 | 25 | ✅ | sandbox.rs `allow(missing_docs)` |
| security/ | ✅ | ✅ sig/key_rot/external_anchor | 🟡 | 29 | ✅ | key_rotation `allow(missing_docs)` |
| sif/ | ✅ | ✅ encode/decode/generated | ✅ | 18 | ✅ | FlatBuffer 生成代码已提交 |
| sink/ | ✅ | ✅ 13 子模块 | 🟡 | 18 | ✅ shm 已接线 | shm.rs `allow(dead_code)` 已移除 |
| sys/ | ✅ | ✅ control_plane/host_info | 🟡 | 11 | ⛔ **control_plane 未接线** | |
| util/hex | ✅ | ✅ WS-6 新 | ✅ | 9+6doc | ✅ | 替换 hex crate |

### 关键未接线项（都有代码+测试，唯独不进 Engine::init）

1. `HotReloadManager` — [hot_reload.rs](../../core/src/config/hot_reload.rs) 未实例化（当前重载由 `ConfigWatcher` + `Engine::reload_config` 驱动）
2. `ControlPlane` — [control_plane.rs](../../core/src/sys/control_plane.rs) /status 硬编码占位
4. `HostInfoProvider` — [host_info.rs](../../core/src/sys/host_info.rs) 未进 Engine

## CLI（`dologctl`）— 15 命令全实现，测试薄弱

| 命令 | 实现 | 文本 | JSON | 测试 | 集成 core |
|:-:|:-:|:-:|:-:|:-:|:-:|
| run --trace | ✅ | ✅ | — | — | ✅ Engine |
| run（steady） | ✅ | ✅ | — | 3（run_smoke） | ✅ Engine |
| plugin（10 动作） | ✅ | ✅ | 🔴 | 6（内联） | ✅ PluginManager |
| config validate | ✅ | ✅ | 🔴 | — | ✅ |
| verify-log/anchor/recovery | ✅ | ✅ | ✅ | 🔴 | 🟡 SIF 解码 |
| record/replay/record-stop | ✅ | ✅ | ✅ | 🔴 | ✅ SIF |
| shm status/clear | ✅ | ✅ | ✅ | 🔴 | ✅ `read_status`（core API） |
| perf | ✅ | ✅ | ✅ | 🔴 | ✅ RecordPool+RingBuffer |
| init/about/version/completions | ✅ | ✅ | — | — | — |

### CLI 缺口

- `--output json` 对 plugin/config/run/init 静默忽略
- `replay --speed` 任意字符串静默回退 max
- **测试覆盖 LOW**：6/7 命令模块 0 测试；JSON 输出 0 测试
- `shm status/clear` 现复用 core `read_status`（唯一事实源）；测试仍薄弱

## plugins / adapters / fuzz

| 组件 | 状态 | 测试 | CI | 备注 |
|:-:|:-:|:-:|:-:|:--|
| formatter_text | ✅ | 6 | ✅ | |
| formatter_json | ✅ | 14 | ✅ | |
| filter_level | ✅ | 17 | ✅ | CI 单线程跑（全局静态） |
| field_container | ✅ | 7 | ✅ | cgroup 检测 |
| bundle（4-in-1 cdylib） | ✅ | 4+5 dlopen | ✅ release 构建+签名 | 4 插件全注册 |
| adapters/rust SDK | ✅ | 6+5 | ✅ workspace | log/tracing/slog facade 全维护 |
| adapters/go | ✅ | 5 | 🔴 无 CI | 需预编译 core 库 |
| adapters/python | ✅ | 4 | 🔴 无 CI | 无 pyproject（非包） |
| adapters/c | ✅ header-only | 🔴 0 测试 | 🔴 | 无 Makefile/CI |
| core/fuzz 3 targets | ✅ 实现 | 51 边缘测试 | 🔴 无 CI | **无 artifacts/ 目录（从未跑过）** |

## docs — 22 文件 × 2 语言完美 1:1

- **双语 1:1**：22 EN ↔ 22 zh-CN 全部 MATCH，无孤儿文件
- **CLI 覆盖**：25 子命令/子动作全部有 docs 段
- **错误码覆盖**：error.rs 73 码 → ErrorCodesReference 全表 1:1
- **docs/README** 准确且新；site README 准确
- **"not implemented" 标记**：~26 处，全部是诚实的 v0.1.0 功能缺口描述（sandbox enforcement / daemon mode / KeyProvider / health endpoint / Ring 2 field signing / per-stage perf breakdown）——随 WS-3/4 落地逐项清除

## 工作流状态

| WS | 主题 | 状态 |
|:-:|:--|:--|
| WS-1 | 错误码体系（14 域） | ✅ 完成 |
| WS-6 | hex + hostname 原生替换 | ✅ 完成 |
| WS-6 前 | 真实 `dologctl run` 循环 | ✅ 完成 |
| WS-2 | sink_shm 接线 | ✅ 完成 |
| WS-3 | 热重载接线 | ✅ 完成 |
| WS-4 | 远程 sink（Kafka/Syslog/Webhook） | ⛔ 待办 |
| WS-5 | 文档/代码一致性清扫 | ⛔ 待办 |
| WS-6A | `rand` 替换 | ⛔ 候选 |
| WS-6B | `crossbeam-channel` 替换 | ⛔ 候选 |
| WS-6C | `serde_json` 替换（CLI） | ⛔ 候选 |
| WS-6D | `clap`/`clap_complete` 替换 | ⛔ 候选 |
