# DoLogger 快速开始指南

> **版本**: v0.2.0 | **最后更新**: 2026-08-12 | **目标受众**: 新用户
>
> **用途**: 5 分钟上手 DoLogger。无需任何先验知识。
>
> 🌐 **语言 / Language**: [中文](QuickStart.md) | [English: DoLogger Quick Start Guide](../en_US/QuickStart.md)
>
> **阅读路径**: 从头到尾阅读本文，准备就绪后跟随[集成指南](IntegrationGuide.md)的链接将 DoLogger 嵌入到您的应用程序中。

---

## 目录

1. [开始之前](#开始之前)
2. [5 分钟快速设置](#5-分钟快速设置)
3. [配置走读](#配置走读)
4. [下一步](#下一步)

---

## 开始之前

### 前提条件

| 工具 | 最低版本 | 检查命令 |
|:-:|:-:|:-:|
| Rust | 1.70 | `rustc --version` |
| CMake | 3.20 | `cmake --version` |
| Git | 任意 | `git --version` |

### 平台支持

| 平台 | 架构 | 状态 |
|:-:|:-:|:-:|
| Linux | x86_64、aarch64 | 完全支持 |
| macOS | x86_64、aarch64 | 完全支持 |
| Windows | x86_64 | 完全支持 |

---

## 5 分钟快速设置

### 步骤 1：克隆并构建（60 秒）

```bash
git clone https://github.com/Nekolio/DoLogger.git
cd dologger
cargo build --release
```

预期输出：`target/release/dologctl`（CLI 工具）和 `target/release/dologger_core`（引擎库）。

### 步骤 2：生成配置文件（30 秒）

```bash
./target/release/dologctl init --template dev
```

这将在当前目录创建具有合理开发默认值的 `dologger.toml`：

```toml
[dologger]
level = "DEBUG"
performance_profile = "dev"
ring_buffer_size = 65536
batch_size = 32
enable_signature = false
```

### 步骤 3：开始记录日志（10 秒）

```bash
./target/release/dologctl run
```

您应看到引擎横幅和随后的日志输出：

```text
   ___       __
  / _ \___  / /  ___  ___ ____ ____ ____
 / // / _ \/ /__/ _ \/ _ `/ _ `/ -_) __/
/____/\___/____/\___/\_, /\_, /\__/_/
                    /___//___/

[2026-08-12T14:30:00.123Z] INFO  DoLogger engine started (profile: dev, level: DEBUG)
```

### 步骤 4：运行示例（可选）

要查看 DoLogger 处理真实应用程序日志，使用内置示例：

```bash
cargo run --example simple_logger -- --config dologger.toml
```

输出：

```text
[2026-08-12T14:30:01.000Z] INFO  Hello from DoLogger example application
[2026-08-12T14:30:01.001Z] WARN  This is a warning message
[2026-08-12T14:30:01.002Z] ERROR An error occurred: simulated failure
```

### 步骤 5：验证日志文件

```bash
cat dologger_output.log
```

记录由 File Sink 写入到 `dologger_output.log`（每条记录一行 JSON）。

---

## 配置走读

您最常接触的五个选项：

### 1. 日志级别

```toml
[dologger]
level = "INFO"          # TRACE | DEBUG | INFO | WARN | ERROR | FATAL
```

设置写入输出的最低严重级别。低于此级别的记录将被丢弃。

### 2. 性能配置文件

```toml
[dologger]
performance_profile = "prod-performance"
```

| 配置文件 | 描述 | 何时使用 |
|:-:|:-:|:-:|
| `dev` | 小缓冲区，签名关闭，快速启动 | 本地开发 |
| `balanced` | 中等吞吐量，基本保护 | 通用工作负载 |
| `prod-performance` | 最大吞吐量，背压控制 | 高吞吐量服务 |
| `prod-audit` | 每条记录 Ed25519 签名，WORM 存储 | 合规强制审计 |

### 3. 环形缓冲区大小

```toml
[dologger]
ring_buffer_size = 262144   # 必须是 2 的幂（65536、131072、262144、524288）
```

更大的缓冲区以内存为代价更好地处理突发工作负载。每个槽是一个记录指针（64 位下 8 字节），因此 262144 个槽使用约 2 MB。

### 4. 输出接收器

```toml
[sinks.console]
type = "sink_console"
enabled = true

[sinks.file]
type = "sink_file"
enabled = true
path = "/var/log/dologger/app.log"
rotation_interval = "24h"
compression = "zstd"
```

DoLogger 有 11 个内置接收器：console、file、callback、Kafka、syslog、webhook、SQLite、WORM、security file、shared memory 和 OpenTelemetry。按需启用任意数量——输出同时发送到所有启用的接收器。

### 5. 插件

```toml
[plugins.json-formatter]
type = "formatter"
path = "/usr/lib/dologger/plugins/libjson_formatter.so"

[plugins.drop-debug]
type = "filter"
path = "/usr/lib/dologger/plugins/libdrop_debug.so"
```

插件无需修改引擎即可扩展 DoLogger。包含推荐的完整列表参见[集成指南](IntegrationGuide.md#插件选择指南)。

---

## 下一步

| 您想要... | 阅读 |
|:-:|:-:|
| 将 DoLogger 嵌入到 C 应用程序中 | [集成指南](IntegrationGuide.md) -- C API 章节 |
| 为 Rust 项目添加日志记录 | [集成指南](IntegrationGuide.md) -- Rust 适配器章节 |
| 了解引擎内部工作原理 | [架构参考](ArchitectureReference.md) |
| 在生产环境中部署 DoLogger | [运维与安全指南](OperationsAndSecurity.md) |
| 编写自定义插件 | [插件开发指南](guides/PluginDevelopmentGuide.md) |
| 验证审计日志完整性 | [运维与安全指南](OperationsAndSecurity.md#审计验证) |

### 快速参考

```bash
# 验证您的配置
dologctl config validate --config dologger.toml --strict

# 列出已加载的插件
dologctl plugin list

# 检查引擎健康状态（需要运行中的引擎）
curl http://127.0.0.1:9090/status

# 验证 Ed25519 审计链
dologctl verify-log --path /var/lib/dologger/audit/

# 收集诊断报告
dologctl diag collect --output diag-report.tar.gz
```

### 故障排查

| 症状 | 解决方案 |
|:-:|:-:|
| 构建失败，提示"CMake not found" | 安装 CMake 3.20+：`apt install cmake` / `brew install cmake` |
| `dologctl run` 立即退出 | 使用 `dologctl config validate` 检查 `dologger.toml` 语法 |
| 无输出出现 | 验证至少有一个接收器 `enabled = true` |
| 插件加载失败 | 检查 `dologger_internal.log` 以获取 ABI 不匹配详情 |

---

## 完整规范

关于每个架构决策、API 和安全属性的权威设计文档，请参阅 [架构参考](ArchitectureReference.md)。
