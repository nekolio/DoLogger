# DoLogger 官方插件

> **版本**: v0.1.0

> 🌐 **语言 / Language**: [中文](OfficialPluginRoadmap.md) | [English: DoLogger Official Plugins](../en_US/OfficialPluginRoadmap.md)

DoLogger 随附一组精选的官方插件——类似于语言标准库——覆盖最常见的
日志、格式化与可观测性需求。第三方插件在此基础上扩展领域特定功能。

**本页是当前版本随附内容的清单，不是路线图——这里没有任何对未来的承诺。**
页面随新增或变更插件的发布一同更新。

## 插件类型与管道位置

（示意性管道草图）：

```
PreFilter(0) → Filter(1) → FieldProvider(2) → Assembly(3) → Processing(4) → Formatting(5) → Sink(6)
```

| 阶段 | 插件类型 | v0.1.0 状态 |
|:-:|:-:|:-:|
| 0 | PolicyProvider | 内置核心：`rate_limiter`、`drop_level` |
| 1 | Filter | 官方插件：`filter_level` |
| 2 | FieldProvider | 内置核心：`host_info`；官方插件：`field_container` |
| 3 | Assembly | 仅核心：LSN + Ed25519 签名 |
| 4 | Processor | 内置核心：`secret_detector` |
| 5 | Formatter | 官方插件：`fmt_json`、`fmt_text` |
| 6 | Sink（核心内置） | 11 个 sink 内置核心 |
| — | KeyProvider | 未实现——签名密钥由核心自行加载 |
| — | ConfigProvider | 未实现 |
| — | SyscallBroker | 未实现 |

## 官方插件

四个官方插件位于 `plugins/official/` 下。它们是 Cargo workspace 成员
（`cargo build --workspace` 即可构建），导出 `plugin_query` /
`plugin_init` / `plugin_shutdown` C ABI 符号，并各自附带
`PluginManifest.toml`。

| 插件 | 类型 | 阶段 | 说明 |
|:-:|:-:|:-:|:-:|
| `filter_level` | Filter | Filter（1） | 按可配置的严重级别丢弃日志记录，支持按域覆盖。 |
| `fmt_json` | Formatter | Formatting（5） | 将 `Record` 字段序列化为结构化 JSON。 |
| `fmt_text` | Formatter | Formatting（5） | 人类可读的文本输出。 |
| `field_container` | FieldProvider | FieldProvider（2） | 注入容器元数据：容器 ID、Pod 名、命名空间、节点名（Docker、Kubernetes、podman）。 |

### filter_level

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Filter（1） |
| 信任级别 | Blue |
| 配置 | `min_level`（默认 `"INFO"`）、`drop_trace`、`drop_debug`，支持按域覆盖 |
| 测试 | 17 个单元测试 |

按可配置的严重级别丢弃日志记录，支持可选的按域覆盖。替代内置的
`DropLevelPolicy`，用于领域特定场景。

### fmt_json

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Formatting（5） |
| 信任级别 | Blue |
| 配置 | 尚未接线——插件以其默认行为运行 |
| 测试 | 9 个单元测试 |

将 `Record` 的字段（级别、消息、时间戳、线程、进程、源文件/函数/行号）
序列化为 JSON 对象。配置解析（`pretty`、`include_ring3`、
`timestamp_format`）尚未实现。

### fmt_text

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Formatting（5） |
| 信任级别 | Blue |
| 配置 | 尚未接线——插件以其默认行为运行 |
| 测试 | 3 个单元测试 |

人类可读的文本输出。配置解析（`color`、`show_thread`、
`show_timestamp`、`timestamp_format`）尚未实现。

### field_container

| 属性 | 值 |
|:-:|:-:|
| 阶段 | FieldProvider（2） |
| 信任级别 | Blue |
| 配置 | 尚未接线——插件以其默认行为运行（`source: auto`） |
| 测试 | 3 个单元测试 |

注入容器编排元数据：容器 ID（来自 `/proc/self/cgroup` 或 `$CONTAINER_ID`）、
Pod 名、命名空间与节点名。自动检测 Docker、Kubernetes 与 podman。
配置解析（`source`）尚未实现。

## 构建与测试

```bash
# 构建全部官方插件
cargo build --release -p dologger-plugin-filter-level \
                      -p dologger-plugin-fmt-json \
                      -p dologger-plugin-fmt-text \
                      -p dologger-plugin-field-container

# filter_level 使用全局静态变量——其测试必须单线程运行
cargo test -p dologger-plugin-filter-level -- --test-threads=1
cargo test -p dologger-plugin-fmt-json
cargo test -p dologger-plugin-fmt-text
cargo test -p dologger-plugin-field-container
```

## 尚未实现

以下内容在 v0.1.0 中刻意缺席，且没有目标版本：

- 远程插件注册表（`dologctl plugin search` / `plugin update`）——CLI 目前
  仅提供 `list`、`install <path>`、`remove`、`verify` 与 `scan`。
- 插件签名工具（`dologctl sign`）与根密钥配置。
- KeyProvider、ConfigProvider 与 SyscallBroker 插件类型。

---

*最后更新：2026-08-13*
