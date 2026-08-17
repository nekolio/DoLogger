# 测试规范

> DoLogger 测试**测什么、放哪里、怎么写**的活事实来源。适用于每一次代码变更：没有本规范要求的测试，任何功能、修复或迁移都不准落地。
>
> 四象限模型（单元 / 集成 / 基准 / 安全-压力）沿袭更早草案中的设计意图，并结合本文档仓库的物理布局落定。

## 1. 测试分类与物理布局

| 象限 | 物理位置 | 放什么 |
|:-:|:-:|:-:|
| **单元** | 代码旁的 `#[cfg(test)] mod tests`（`core/src/**`） | 单函数 / 单算法 / 单数据结构的行为；错误路径；不变量 |
| **集成** | [core/tests/](../../../core/tests/)（`{subject}.rs`，Cargo 自动发现） | 跨模块行为：配置 → 注册表 → Sink、插件 bundle、沙箱、安全、扇出 |
| **集成（进程级）** | [tests/](../../../tests/)（`common/`、`smoke/`、`release-smoke/`） | C ABI 冒烟、平台冒烟 runner、发布门禁 |
| **基准** | [core/benches/](../../../core/benches/)（Criterion） | 延迟、吞吐、百分位；各 Sink 投递延迟 |
| **模糊** | [core/fuzz/](../../../core/fuzz/)（cargo-fuzz） | 不可信输入解析器：SIF 帧、TOML 配置、环形缓冲操作 |
| **性能（C ABI）** | [tests/perf/](../../../tests/perf/)（CMake） | 宿主语言 / C ABI 吞吐 harness |

## 2. 什么必须写测试——无例外

1. **每个新公共 API**（`pub fn` / C ABI 函数 / 配置键）：正常路径 + 每一条 `Result::Err`/错误码路径。一条代码路径如果能失败，就必须有一条测试把它驱动到失败。
2. **每个新错误码**（见[错误码参考](ErrorCodesReference.md)）：该码必须被产生它的失败路径实际触发。
3. **每个配置面**（新 TOML 字段、新 `[shm]`/`[dologger]` 键）：合法配置可解析且生效；缺失字段回退默认；非法值以正确错误码/警告被拒；跨平台路径转义（如 Windows 路径中的 `\`）被覆盖。
4. **每个 Sink**（含 `sink_shm` 与远程 Sink）：生命周期（open→write→flush→close）、故障模式（环满 / 连接丢失 / 超时）、清理（无泄漏的描述符 / 共享内存对象）。
5. **跨平台守卫**：任何 `#[cfg(...)]` 保护的代码，在每个受影响的平台都有测试（CI 覆盖 Linux；矩阵有 Windows/macOS 则各跑一遍）。平台特有行为绝不能只由单一平台覆盖。
6. **确定性**：任何标注为确定性的东西（hero 美术、发布工具、配置加载）必须有"跑两遍逐字节一致"的测试。

## 3. 怎么写测试——仓库风格

- **放置**：单元测试内联（`#[cfg(test)] mod tests`）；集成测试作为 `core/tests/` 下的独立文件。效仿 [fanout_sinks.rs](../../../core/tests/fanout_sinks.rs) 的现有风格：文档注释写明被测性质、小型聚焦测试、无共享全局状态。
- **唯一临时产物**：创建临时文件/shm 名时必须用「每进程原子计数器 + `process::id()`」（见 fanout_sinks 的 `temp_path()`），避免并行测试互撞。测试体内务必清理。
- **字符串内 TOML**：用 `DologgerConfig::parse` 内联构建配置，而非脆弱的文件 fixture；Windows 路径转义 `\`。
- **错误码断言**：断言符号常量，绝不硬编码字面值。
- **确定性优先于易碎**：优先有界的进程内测试而非计时断言；对并发结构使用 `Ordering` 感知的期望；若测试设计上就易碎（如规模相关），用环境变量门控而不是删除。
- **Rust 最佳实践**：`#[should_panic]` 仅用于已文档化的不变量；解析器输入用 `proptest`/`arbitrary`；`loom` 是无锁结构的*未来*工具——只有当真在追某个特定交织 bug 时才引入，不提前加。

## 4. 验收门禁（每次提交前 / CI 中执行）

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dologger-core
cargo test -p dologctl
cargo test (workspace, default members)
```

基准在本机对 release 构建跑（`cargo bench`），结果进 `benchmark-results.json` 与发布说明。硬性按 PR 级别基准门禁（吞吐 −5%、P99 +10%、RSS +5% 对比基线）是文档化目标——待稳定的 CI 基线 runner 就绪后启用；在此之前：记录并发布，不设门禁。

## 5. Sink 专项测试清单（适用于 `sink_shm` 与远程 Sink）

- **单元**：配置校验（每个非法值 → 对应错误码）、header 布局尺寸断言、环满检测、drop/overwrite 计数器。
- **集成（shm 需跨进程）**：生产者写入 N 条 SIF 记录；独立消费者进程只读挂载并验证每条解码记录一致；`drop_oldest`/`drop_newest` 的环满行为；生产者崩溃 → 消费者检测到 `FLAG_PRODUCER_DEAD` 后正常退出、无悬挂指针；AUDIT 域配置被拒。
- **模糊**：写入环的畸形 SIF 字节不得使消费者解析器崩溃（`shm_parser` 目标）。
- **压力（等 CI 容量就绪）**：共享水位线语义下多消费者并发挂载无数据竞争；长跑泄漏检查共享内存对象尺寸恒定。

## 6. 自查清单

变更标记完成前，按清单过一遍：

- [ ] 新增/变更的错误路径都有测试命中？
- [ ] 新配置键有解析 / 默认 / 拒绝三类测试？
- [ ] 新 Sink 覆盖生命周期 AND 故障 AND 清理？
- [ ] 平台特有路径在各平台各测一遍？
- [ ] `cargo test` + `clippy -D warnings` 通过？
- [ ] 变更对应的文档（中英 1:1）与行为保持一致？

## 相关链接

- [错误码参考](ErrorCodesReference.md) — 测试必须断言的那些码
- [tests/README.md](../../../tests/README.md) — 规范类别 → 物理位置的权威映射
- `core/tests/` 集成套件 — 供效仿的风格范例