# 命名规范

> 适用于本仓库的所有源文件：Rust、C、C++、Go、Python 以及构建/CI 脚本。目标是建立一套单一、可校验的语法，让读者能从职责推断文件名——也能从文件名推断职责。

## 1. 目录树即命名空间

层级结构与模块图承载「层/模块」信息。叶子文件以**其主导出的概念**命名——绝不与所在模块名重复：

```
core/src/buffer/object_pool.rs     # object_pool，位于 buffer 模块内
core/src/sink/shared_memory.rs     # shared_memory，位于 sink 模块内
```

不要在 `buffer/` 里写 `buffer_object_pool.rs`——路径已经说明它是 `buffer`。重复会带来三重冗余，并拉长所有 import。

## 2. 叶子语法

文件名要么是裸对象，要么是对象 + 角色后缀：

```
{对象}             →  以主导出项命名的 snake_case 名词
{对象}_{角色}       →  对象 + 批准的词表后缀（见 §3）
```

示例：

| 模式 | 示例 |
|:-:|:-:|
| `{对象}` | `record`、`audit`、`domain`、`phase`、`time` |
| `{对象}_{角色}` | `key_provider`、`secret_detector`、`ring_buffer`、`control_plane` |

规则：

- snake_case，ASCII 小写；无空格、无 CamelCase 文件名（类型可以用 PascalCase，文件名绝不可以）。
- 一个文件一个主概念。定义 `Foo` 加一个极小的辅助结构体的文件仍是 `foo.rs`；把两个无关概念塞进一个文件的应当拆分。
- 同目录兄弟文件遵循同一语法：`sink/` 用一个 sink 一文件的裸名词（`console`、`file`、`syslog`），基础设施用 `{对象}`（`ring_buffer`）。
- 除 §4 所列，文件名中不得使用缩写。

### Shell 脚本（`scripts/`、`peripheral/github/scripts/`）

可执行脚本采用 PowerShell 式「动词-名词」命名：

```
{动词}-{宾语}.sh        # build-all.sh、setup-conan.sh、check-env.sh
```

- `{动词}` 取自下表；`{宾语}` 为小写目标，多词用连字符（`release-notes`）。
- `.sh` 后缀**强制**——每个 Bash 脚本都必须带。不允许无后缀、`dologger-` 前缀的名字：目录已表明项目身份，脚本以 `bash scripts/<名>.sh` 调用。
- 只用全词——`generate-release-notes.sh`，绝不写 `gen-release-notes.sh`。

批准动词（新动词须加入下表并评审，同 §3）：

| 动词 | 含义 |
|:-:|:-:|
| `build` | 编译产物 |
| `setup` | 安装/探测前置条件 |
| `check` | 校验环境或输出 |
| `sync` | 将内容镜像到目标 |
| `generate` | 从 git 状态生成文档/正文 |

## 3. 批准的角色后缀词表

角色后缀是 PowerShell「动词-名词」规则在代码层面的对应物：「动词」即文件的角色，取自一份固定词表。新角色须加入词表（并评审），而非临时自造。

| 角色 | 含义 |
|:-:|:-:|
| `manager` | 掌管一个或多个对象的生命周期 |
| `provider` | 按需提供对象（类似工厂） |
| `dispatcher` | 将工作/记录路由给处理器 |
| `validator` | 在接受前强制不变量 |
| `loader` | 从存储/配置实例化对象 |
| `writer` | 持久化输出 |
| `reader` | 读取输入 |
| `watcher` | 观察状态并作出反应 |
| `scheduler` | 决定执行顺序/时机 |
| `detector` | 识别某条件（如密钥、异常） |
| `rotator` | 周期轮换凭据/密钥 |
| `builder` | 逐步构建复杂对象 |
| `parser` | 将文本/字节转换为结构化形式 |
| `store` | 封装持久集合 |
| `registry` | 将键映射到已注册项 |
| `service` | 长期运行、对外可见的能力 |
| `policy` | 封装决策规则 |
| `facade` | 用一个入口简化子系统 |
| `adapter` | 将一个接口适配为另一个 |
| `layer` | 对接日志前端 |
| `encoder` / `decoder` | 序列化 / 反序列化 |
| `reporter` | 输出指标/遥测 |
| `handler` | 响应事件或回调 |
| `engine` | 驱动有状态计算循环 |

并非每个文件都需要角色。没有合适角色时，用裸 `{对象}` 形式。

## 4. 允许的缩写

缩写被禁止，除非是 (a) 行业通用术语，或 (b) 冻结公共 ABI 的一部分。下表逐项说明豁免原因。

| 名称 | 保留原因 |
|:-:|:-:|
| `ffi` | 通用术语；`dologger_core::ffi` 是插件面向的 API 面，已冻结 |
| `io` | 输入/输出的通用术语 |
| `shm` | 与 POSIX `shm_open` 及冻结的 C ABI（`dologger_shm.h`、`dologger_shm_*`）一致 |
| `crc32c` | 算法规范名（Castagnoli CRC-32C） |
| `sif` | 项目术语：**S**tandard **I**ntermediate **F**ormat |
| `perf` | CLI 子命令动词；与 `perf` 工具名一致 |

其余一律拼全——`diag` → `diagnostics`、`sysmon` → `system_monitor`、`otel` → `open_telemetry`。

## 5. 禁止模式

- **叶子中重复模块名**（`sink/sink_file.rs`）。
- **无对象的模糊角色名词**：crate 根部不允许裸 `manager.rs`、`handler.rs`、`service.rs`——必须指明对象。
- **同一概念的混用词汇**：若某概念叫 `record`（而非 `log`）、叫 `sink`（而非 `output`），则该概念的所有文件都用同一词。当两个词确实含义不同（`internal_log` vs `syslog`）时，保持区分并记录差异。
- **特性名与文件名漂移**：被特性 `foo` 门控的文件必须命名为 `foo.rs`，或拼全其对象（`open_telemetry.rs` 由 `sink-otel` 门控）。

## 6. 改名记录

| 原名 | 现名 | 原因 |
|:-:|:-:|:-:|
| `core/src/sys/diag.rs` | `core/src/sys/diagnostics.rs` | 缩写 → 全词 |
| `core/src/sink/otel.rs` | `core/src/sink/open_telemetry.rs` | 缩写 → 全词；同时修复特性门控（`sink-webhook` → `sink-otel`） |
| `core/src/sys/sysmon.rs` | `core/src/sys/system_monitor.rs` | 融合缩写 → 全词 |

向后兼容的 re-export 保留在 `core/src/lib.rs`（`diag`、`sysmon`、`sink_otel`），既有 `dologger_core` 路径继续可用。

## 7. 检查

`cargo fmt` 机械强制 snake_case。角色后缀与缩写规则在代码评审中把关；未来可用 `cargo xtask lint-naming` 机械校验缩写清单。
