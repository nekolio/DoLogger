# 架构演进：Sink 从插件到核心内置的根源性差异

> **版本**：v0.0.1 | **最后更新**：2026-08-15 | **目标读者**：核心开发者、架构评审者、插件作者
>
> **用途**：以设计意图为透镜（仓库内权威等价物 = [ArchitectureReference.md](ArchitectureReference.md)），对比架构大改前（`aedcd7f~1`）与大改后（`aedcd7f`）两套架构模型，回答一个问题：**「最终成果 vs 企划书」的差距，其根源性来源是什么？**
>
> 🌐 **语言 / Language**: [中文](ArchitectureEvolution.md) | [English: Architecture Evolution](../en_US/ArchitectureEvolution.md)

---

## TL;DR（结论先行）

架构大改只有**一个根源决定**：

> **输出执行（Sink）从「可插拔插件域」移动到「可信核心域」。**

这一个决定同时重构了系统的**本体论**（Sink 是什么类别）、**信任边界**（输出还被沙箱/信任闸保护吗）与**分发模型**（输出如何被驱动）。这三层不是三个独立改动，而是**同一个根源决定的三个投影**。

「最终成果 vs 企划书」的差距，主要由这一个决定塑造：企划书把 Sink 当作 10 种插件之一（可扩展、可沙箱），最终架构把它收编进核心（无 VTable、信任闸管不到），换来更简单的配置扇出与更小的安全面。

---

## 1. 根源决定

提交 [`aedcd7f`](https://github.com/nekolio/DoLogger/commit/aedcd7f) "refactor(core): Sink is a core built-in, not a plugin type" 是本次架构大改的锚点。

| | 大改前（`aedcd7f~1`） | 大改后（`aedcd7f`） |
|:-:|:-:|:-:|
| **Sink 的类别** | 插件类型 #5（`dologger_iosink_vtable_t`） | 核心内置输出执行器（阶段 6） |
| **阶段位** | `DO_LOG_PHASE_SINK = 0x0020u` | 已删除 |
| **插件 VTable 类型数** | 10 种 | 9 种 |
| **文档固化** | 插件 VTable 规范含 Sink | [ArchitectureReference.md](ArchitectureReference.md#插件-vtable-规范)：「Sink 不是插件类型……11 种内置接收器由核心直接驱动」 |

一个决定，为什么能产生如此大的连锁反应？因为 Sink 恰好落在两个域的**交叠面**上：它是**输出**（可信核心必须保证不丢、防篡改），又曾是可**扩展**面（第三方想自定义输出）。之前把这两个属性绑定在同一个类型上，大改把它们拆开了。

---

## 2. 三个投影

### 投影 A —— 本体论（Ontology）：Sink 的类别变了

**前**：Sink = 插件类型 #5，拥有自己的 C ABI VTable。`core/include/dologger_core.h` 中定义：

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

它遵守与其它插件完全相同的生命周期：加载、验证、挂载、卸载——受信任闸与沙箱约束的**不可信扩展**。

**后**：Sink 是核心内置输出执行器，**没有 VTable**、没有插件生命周期、不可按插件加载/卸载。文档固化（[ArchitectureReference.md](ArchitectureReference.md)）：「Sink 不是插件类型：它是核心内置的输出执行器（阶段 6），没有 VTable。11 种内置接收器由核心直接驱动。」

**本质**：Sink 从「**引擎之外的扩展**」变成「**引擎本身的一部分**」。

### 投影 B —— 信任边界（Trust boundary）：输出还被保护吗

**前**：Sink 在沙箱 whitelist 里是**可被沙箱化引擎加载的插件类型**。Red（`SandboxLevel::Isolated`）级允许类型数组明确包含 `"IOSink"`（`core/src/plugin/sandbox.rs:281` 注释「Red can only be: Filter, FieldProvider, Processor, Formatter, IOSink」）。也就是说，第三方写自定义 sink 是**设计允许的扩展面**——且该扩展被沙箱保护。

**后**：sink 属于**可信核心**，不再进入插件沙箱的词汇表。`aedcd7f` 同时做了三处收口：
- `sandbox.rs`：Red/Isolated 允许类型数组删除 `"IOSink"`（注释改为「Filter, FieldProvider, Processor, Formatter only」）。
- 沙箱测试 `tests/security/sandbox_escape/mod.rs`：allowed-type 数组与 README 同步删除 IOSink。
- 测试函数重命名：`red_allows_only_render_transform_types` → `red_allows_only_transform_plugin_types`——语义从「渲染/转换输出类」收敛为「转换插件类」。

**含义**：**输出不再被信任闸/沙箱保护**——它被假定为引擎自身。第三方输出扩展的通道从「插件 vtable」移到「配置 + Callback」。这是一次安全面的**收窄**：少了一条可被恶意插件冒充的扩展路径。

### 投影 C —— 分发模型（Dispatch）：输出如何被驱动

**前**：Sink 按 phase 位 `0x0020` 挂载到阶段 6，与其它插件同走 `resolve_dispatch` 的 vtable 分发——运行期动态解引用函数指针调用。

**后**：sink 由 `[sinks.*]` TOML 注册表（`type` 标签）+ `FanoutSink`（M4+M5）驱动（`core/src/sink/registry.rs`）。插件分发只剩 Formatter / FieldProvider（M6）。

```toml
[sinks.stdout]
type = "console"

[sinks.applog]
type = "file"
path = "/var/log/app.log"
```

**含义**：**输出路径从「多态 vtable 调用」变成「配置驱动的核心扇出」**——配置即插即用，但放弃了运行期动态加载第三方 sink。

### 三个投影的关系

```
          根源决定：输出执行（Sink）从插件域 → 核心域
                        │
        ┌───────────────┼───────────────┐
        │               │               │
     投影A            投影B           投影C
     本体论           信任边界        分发模型
   Sink 类别变了    输出不再被沙箱     输出变成配置扇出
   插件#5 → 内置    保护（安全面收窄）   vtable → [sinks.*]
```

---

## 3. 「前 / 后」对照表

| 维度 | 大改前 | 大改后 | 差异类别 |
|:-:|:-:|:-:|:-:|
| Sink 类别 | 插件类型 #5 | 核心内置（阶段 6） | 根源决定 |
| C ABI | `dologger_iosink_vtable_t` | 删除 | 根源决定 |
| 阶段位 | `DO_LOG_PHASE_SINK = 0x0020u` | 删除 | 根源决定 |
| 插件 VTable 类型数 | 10 | 9 | 根源决定 |
| 沙箱 whitelist | Red 允许 IOSink | 删除 IOSink | 根源决定（信任投影） |
| 分发方式 | vtable 动态分发 | `[sinks.*]` + `FanoutSink` | 根源决定（分发投影） |
| 沙箱测试命名 | `red_allows_only_render_transform_types` | `red_allows_only_transform_plugin_types` | 根源决定 |
| 第三方输出扩展通道 | 插件 vtable | 配置 + Callback | 根源决定 |

---

## 4. 派生差距（标注类别）

> 此处区分两类差距：**「前后架构」差异**（由根源决定直接产生）与**「文档设计 vs 当前实现」差异**（设计意图已定但未实现/未完全实现）。后者**不属于**本次大改的范围，标注出来供下一阶段规划。

### 4.1 「前后架构」差异（本文档核心）

1. **插件类型 10 → 9**：`PHASE_SINK` 位删除，`dologger_iosink_vtable_t` 从 C ABI 消失。→ 直接由根源决定产生。
2. **目前仅 Formatter / FieldProvider 两类被真正分发**：`Filter`/`Processor`/`ConfigProvider`/`KeyProvider`/`PolicyProvider`/`HostInfoProvider`/`SyscallBroker` 七类 vtable 存在（`dologger_core.h` + ArchitectureReference 9 种类型表），但未接线到管道阶段。→ 这是**设计意图已定、实现未跟上**的差距（见 4.2）。

### 4.2 「文档设计 vs 当前实现」差异（下一阶段范围）

> 这些是设计文档（ArchitectureReference / 企划书意图）承诺、但当前实现尚未达成的点。**不是**本次 Sink 大改引入的回归，而是待补的缺失核心项。

1. **七类插件未分发**：Filter(Stage1)、Processor(Stage4)、Config/Key/Policy/HostInfo/SyscallBroker 各阶段钩子未接线。→ 下一步 **Batch A** 范围。
2. **并行 io_pool 扇出 + 回退链 + 熔断**：文档描述的并行扇出与回退链，在当前 `FanoutSink` 中为顺序实现（或部分未实现）。
3. **沙箱（seccomp-bpf/AppContainer）未实现**：当前仅有 Ed25519 签名信任闸。Blue/Yellow/Red 信任色属于信任闸概念，保留为设计意图。
4. **Formatting→Sink 的 SIF 交接处于规划中**（[ArchitectureReference.md](ArchitectureReference.md)）。

---

## 5. 为什么这决定「最终成果 vs 企划书」的差距

企划书的原设计把 Sink 当作 10 种插件之一——**可扩展、可沙箱、运行期可加载**。最终架构把它收编进核心——**不可扩展 vtable、信任闸管不到、配置驱动**。

这个收敛方向的**取舍**是：

| 企划书设计的 Sink | 最终架构的 Sink | 得 | 失 |
|:-:|:-:|:-:|:-:|
| 插件化、可沙箱 | 核心内置、配置驱动 | 输出路径简化、安全面收窄 | 放弃运行期第三方 sink |
| 10 种插件 | 9 种插件 | 本体更干净（输出≠扩展） | 多一个核心内置概念 |

**关键洞察**：这不是「实现没做完」导致的差距，而是**架构意图本身主动做出的选择**。企划书想要「可扩展的输出」，最终架构选择「可信的输出」。这个决定是**根源性的**——它不再是一个可后补的 feature，而是重塑了整个系统的信任模型与扩展模型。

后续要弥合与企划书的差距，正确的方向不是「把 Sink 改回插件」，而是在**核心内置**的前提下补齐：配置扇出的完备性、回退链、熔断、以及真正分发其余七类插件。

---

## 参考

- 本文件英文版：[Architecture Evolution](../en_US/ArchitectureEvolution.md)
- 设计意图权威文档：[ArchitectureReference.md](ArchitectureReference.md)
- 大改提交：`aedcd7f` "refactor(core): Sink is a core built-in, not a plugin type"
- 相关实现记忆：[[plugin-m6-dispatch]]、[[ffi-field-access-ring3-only]]
