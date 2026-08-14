# DoLogger 宿主集成手册 (Host Integration Guide)

> 🌐 **语言 / Language**: [中文](HostIntegrationGuide.md) | [English: Host Integration Guide](../../en_US/guides/HostIntegrationGuide.md)

> **版本**: v0.1.0 | **最后更新**: 2026-08-12 | **目标受众**: 宿主应用开发者

## 目录

1. [概述](#概述)
2. [快速开始](#快速开始)
3. [C ABI 初始化与关闭](#c-abi-初始化与关闭)
4. [日志提交](#日志提交)
5. [配置体系](#配置体系)
6. [Record 字段权限环](#record-字段权限环)
7. [错误处理](#错误处理)
8. [回调 Sink 注册](#回调-sink-注册)
9. [线程安全模型](#线程安全模型)
10. [语言适配器](#语言适配器)
11. [性能调优](#性能调优)
12. [故障排除](#故障排除)

---

## 概述

DoLogger 是一个跨平台、高安全、插件化的日志引擎。宿主应用通过 C ABI 动态链接 `libdologger_core`（`.so` / `.dylib` / `.dll`）来使用日志功能。

### 关键特性

- **稳定 C ABI**：所有公共 API 使用 `dologger_*` 前缀的 C 函数，并提供固定的 ABI 版本保证。
- **零 Rust 工具链依赖**：宿主只需链接预编译的动态库，无需安装 Rust 编译器或 cargo。
- **插件化架构**：9 种插件 VTable 类型，在引擎启动时按需加载。
- **Ed25519 签名 + LSN 审计链**：通过密码学链式结构实现防篡改的日志完整性保护。
- **三色信任模型**：Blue / Yellow / Red 插件分级隔离（见 [安全白皮书](SecurityWhitepaper.md)）。

### 支持平台

| 平台 | 架构 | 动态库后缀 |
|:-:|:-:|:-:|
| Linux | x86\_64, aarch64 | `.so` |
| macOS | x86\_64, aarch64 | `.dylib` |
| Windows | x86\_64 | `.dll` |

### API 稳定性

C ABI 遵循语义化版本控制。版本字符串可通过 `dologger_version()` 获取；插件兼容性由加载时检查的 `abi_version` 字段控制（见 [版本控制与弃用](VersioningAndDeprecation.md)）。次版本号升级只会新增符号，不会删除或重排现有符号。

---

## 快速开始

### 1. 初始化引擎

```c
#include "dologger_core.h"
#include <stdio.h>

int main(void) {
    dologger_error_t err;
    dologger_handle_t *logger = dologger_init(NULL, &err);
    if (logger == NULL) {
        fprintf(stderr, "Failed to initialize DoLogger: %s\n", err.message);
        return 1;
    }

    // ... 使用 logger ...

    dologger_shutdown(logger);
    return 0;
}
```

### 2. 提交日志

```c
dologger_record_params_t params = {
    .level           = DO_LOG_INFO,
    .message         = "Hello from host application",
    .source_file     = "main.c",
    .source_function = "main",
    .source_line     = 42,
};
int32_t rc = dologger_log(logger, &params);
if (rc != DO_LOG_OK) {
    // 处理背压 — 记录被丢弃或队列已满
    dologger_error_t err;
    dologger_get_last_error(logger, &err);
    fprintf(stderr, "Log submission failed: %s (code 0x%04x)\n",
            err.message, (unsigned)err.code);
}
```

### 3. 链接库

**Linux / macOS：**
```bash
cc -o myapp myapp.c -ldologger_core -L/usr/lib/dologger
```

**Windows (MSVC)：**
```bash
cl /Fe:myapp.exe myapp.c dologger_core.lib
```

验证二进制编译时链接的版本：
```c
const char *ver = dologger_version();
printf("DoLogger version: %s\n", ver);
```

---

## C ABI 初始化与关闭

### `dologger_init()`

```c
dologger_handle_t *dologger_init(const char *config_path, dologger_error_t *err);
```

**参数：**

| 参数 | 方向 | 说明 |
|:-:|:-:|:-:|
| `config_path` | In | TOML 配置文件路径。传 `NULL` 使用自动发现（依次搜索 `dologger.toml`、`.dologger.toml`）。 |
| `err` | Out | 失败时接收错误详情。首次调用时不得为 `NULL`。 |

**返回值：**

| 结果 | 含义 |
|:-:|:-:|
| 非 `NULL` 句柄 | 引擎初始化成功。 |
| `NULL` | 初始化失败 — 检查 `err` 获取详情。第二次调用 `dologger_init` 会返回 `NULL` 且错误码为 `DO_LOG_ERR_ALREADY_INITIALIZED`。 |

v0.1.0 中不存在 `dologger_init_params_t` — 初始化参数来自配置文件（或传 `NULL` 使用默认值），外加运行时 `dologger_config_load_from_string()` API。

### `dologger_shutdown()`

```c
void dologger_shutdown(dologger_handle_t *handle);
```

执行优雅关闭：

1. 停止接收新的日志记录。
2. 将环形缓冲区中所有 in-flight 记录经 Pipeline 排空。
3. 按依赖逆序调用每个已加载插件的 `plugin_shutdown()`。
4. 刷新并关闭所有 Sink。
5. 释放引擎及其资源。

**关闭策略**由 `shutdown_policy` 配置键控制：

| 策略 | 行为 |
|:-:|:-:|
| `graceful` | 最多等待 `shutdown_timeout_ms` 毫秒让 Pipeline 排空。`prod-audit` 的默认值。 |
| `immediate` | 丢弃 in-flight 记录并立即终止。仅适用于非审计部署。 |

---

## 日志提交

### `dologger_log()`

```c
int32_t dologger_log(dologger_handle_t *handle, const dologger_record_params_t *params);
```

这是热路径。该调用将记录推入无锁环形缓冲区后立即返回。过滤、字段组装、格式化、签名和 I/O 都在后台 Pipeline 线程上异步完成。

### 参数结构

（与 `core/include/dologger_core.h` 一致 — 已编译验证）：

```c
typedef struct {
    dologger_level_t level;         // DO_LOG_TRACE (0) ~ DO_LOG_AUDIT (6)
    const char      *message;       // UTF-8 编码日志消息（必填）
    const char      *source_file;   // __FILE__（可选，可为 NULL）
    const char      *source_function; // __FUNCTION__（可选，可为 NULL）
    uint32_t         source_line;   // __LINE__（可选，无则填 0）
    uint32_t         source_column; // 列号（可选，无则填 0）
    const char      *domain;        // 日志域名称（NULL = 默认域）
    const char      *user_id;       // 可选上下文
    const char      *session_id;    // 可选上下文
    const char      *request_id;    // 请求 / 追踪关联 ID（可选）
    uint8_t          _reserved[16]; // 保留 — 必须清零
} dologger_record_params_t;
```

### 日志级别

| 常量 | 值 | 严重度 | 说明 |
|:-:|:-:|:-:|:-:|
| `DO_LOG_TRACE` | 0 | Trace | 帧级诊断细节。生产环境谨慎使用。 |
| `DO_LOG_DEBUG` | 1 | Debug | 开发期间有用的诊断信息。 |
| `DO_LOG_INFO` | 2 | Info | 一般运维消息（服务启动、配置加载）。 |
| `DO_LOG_WARN` | 3 | Warning | 潜在有害情形（重试、降级模式）。 |
| `DO_LOG_ERROR` | 4 | Error | 不会中止应用的错误事件。 |
| `DO_LOG_FATAL` | 5 | Fatal | 导致提前终止的严重错误。 |
| `DO_LOG_AUDIT` | 6 | Audit | 不可否认的审计记录。背压时可能阻塞。 |

### 便捷宏

```c
// （伪代码/示意 — 未编译：dologger_log_fmt 尚未纳入
// 已发布的 C ABI；以下模式展示预期的宏形态）
// 带自动文件/行号/函数捕获的标准日志
#define DO_LOG_TRACE(h, msg, ...)  dologger_log_fmt(h, DO_LOG_TRACE,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_DEBUG(h, msg, ...)  dologger_log_fmt(h, DO_LOG_DEBUG,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_INFO(h, msg, ...)   dologger_log_fmt(h, DO_LOG_INFO,   __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_WARN(h, msg, ...)   dologger_log_fmt(h, DO_LOG_WARN,   __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_ERROR(h, msg, ...)  dologger_log_fmt(h, DO_LOG_ERROR,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_FATAL(h, msg, ...)  dologger_log_fmt(h, DO_LOG_FATAL,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
#define DO_LOG_AUDIT(h, msg, ...)  dologger_log_fmt(h, DO_LOG_AUDIT,  __FILE__, __func__, __LINE__, msg, ##__VA_ARGS__)
```

### AUDIT 级别背压

`DO_LOG_AUDIT` 级别的记录遵循**审计背压铁律**：发生背压时，调用方会阻塞直到记录被持久化落盘 — AUDIT 域强制无限阻塞超时（`block_timeout_ms = 0`）和 `Never` 丢弃策略（见 `core/src/pipeline/backpressure.rs`）。非 AUDIT 域则使用 Profile 的超时和丢弃策略。该行为不可配置 — 属于[不可降级安全项](SecurityWhitepaper.md#non-downgradable-items)。

---

## 配置体系

### 配置优先级（低→高）

1. 硬编码默认值（编译进 `libdologger_core`）。
2. 系统级配置（Linux 为 `/etc/dologger/default.toml`，Windows 为 `%PROGRAMDATA%\dologger\default.toml`）。
3. 项目本地配置（从 CWD 向上遍历父目录搜索）。
4. 环境变量（`DO_LOG_LEVEL`、`DO_LOG_CONFIG_FILE` 等）。
5. 运行时 API（`dologger_config_load_from_string()`）。
6. 单条记录元数据标签。
7. 不可降级安全项（绝对硬限制，不可被覆盖）。

### 核心配置键

```toml
[dologger]
# 日志级别：TRACE、DEBUG、INFO、WARN、ERROR、FATAL、AUDIT
level = "INFO"

# 性能 Profile：dev | prod-performance | prod-audit | balanced
performance_profile = "prod-performance"

# 环形缓冲区容量。必须是 2 的幂。
ring_buffer_size = 262144

# 每个 Pipeline 批次处理的记录数。
batch_size = 256

# 为审计记录启用 Ed25519 密码学签名。
enable_signature = false

# 关闭行为。"graceful" 在退出前排空 in-flight 记录。
shutdown_policy = "graceful"
shutdown_timeout_ms = 5000
```

### 性能 Profile

| Profile | `block_timeout_ms` | `drop_strategy` | 签名 | 适用场景 |
|:-:|:-:|:-:|:-:|:-:|
| `dev` | 100 | `drop_newest` | Off | 本地开发与调试 |
| `prod-performance` | 3000 | `below_warn` | Optional | 高吞吐生产服务 |
| `prod-audit` | 3000 | `below_warn` | Required | 合规强制审计日志 |
| `balanced` | 2000 | `oldest` | Optional | 通用部署 |

### 环境变量

| 变量 | 覆盖项 | 示例 |
|:-:|:-:|:-:|
| `DO_LOG_LEVEL` | `level` | `DO_LOG_LEVEL=DEBUG` |
| `DO_LOG_BUF_SIZE` | `ring_buffer_size` | `DO_LOG_BUF_SIZE=524288` |
| `DO_LOG_PERF_PROFILE` | `performance_profile` | `DO_LOG_PERF_PROFILE=balanced` |
| `DO_LOG_CONFIG_FILE` | 配置文件路径 | `DO_LOG_CONFIG_FILE=/opt/myapp/dologger.toml` |
| `DO_LOG_CONFIG_LOCK` | 禁止回退配置搜索（需配合 `DO_LOG_CONFIG_FILE`） | `DO_LOG_CONFIG_LOCK=1` |

### 配置热重载

（伪代码/示意 — `ConfigWatcher`（`core/src/config/watcher.rs`）在 v0.1.0 中尚未接入 `Engine::init`：引擎**不会**自动重载配置。请重启引擎，或通过控制面触发重载（规划中）。）

```bash
# 伪代码/示意 — v0.1.0 不会自动生效
# 在运行时修改日志级别
# sed -i 's/level = "INFO"/level = "DEBUG"/' /etc/dologger/default.toml
# 引擎约 1.5 秒内感知变更
```

变更会通过 sysmon 记录为 `CONFIG_RELOAD` 事件。安全层键（不可降级项）无法通过热重载放宽。

### 控制面重载

```bash
# 伪代码/示意 — v0.1.0 中控制面未随引擎启动
# curl -X POST http://127.0.0.1:9090/reload
```

发布后，`/reload` 将忽略请求体（仅调用已注册的重载回调）；带 `dry_run` 校验的 JSON 请求体在规划中：
```bash
# （规划中 — 请求体尚未生效）
# curl -X POST http://127.0.0.1:9090/reload \
#   -H "Content-Type: application/json" \
#   -d '{"dry_run": true}'
```

---

## Record 字段权限环

DoLogger 对日志记录字段实施四级环状访问控制模型。安全设计动机另见 [安全白皮书](SecurityWhitepaper.md#record-field-permission-rings)。

| Ring | 名称 | 允许写入方 | 允许读取方 | 完整性 |
|:-:|:-:|:-:|:-:|:-:|
| Ring 0 | 引擎核心 | 仅核心引擎 | 格式化器 / Sink（只读） | Ed25519 |
| Ring 1 | 系统受信 | 核心 + `HostInfoProvider` | 所有插件（只读） | Ed25519 |
| Ring 2 | 已验证插件 | Blue / Yellow 插件 | 所有插件 | Ed25519（可配置） |
| Ring 3 | 不可信扩展 | 任何插件 | 任何插件 | CRC32C |

### Ring 0 — 不可变字段

这些字段由核心引擎一次性写入，任何插件都**不得**修改：

- `record.id` — 全局唯一记录标识（雪花算法）
- `record.timestamp` — 记录入队时的墙上时钟时间
- `record.signature` — 覆盖 Ring 0 + Ring 1 字段的 Ed25519 签名
- `record.origin_lsn` — 入队时分配的日志序列号

### Ring 1 — 宿主上下文

该环的字段由核心引擎和 `HostInfoProvider` 插件写入：

- `host.name`、`host.os`、`host.arch`
- `process.id`、`process.name`、`process.thread_id`
- `environment`（production / staging / development）

### Ring 2 — 已验证扩展

Blue 和 Yellow 插件可以写入带 `verified.*` 命名空间前缀的字段。每次写入都会追加一条包含 `{plugin_id, plugin_version, timestamp}` 的 `audit_tags` 条目。

### Ring 3 — 不可信

Red 插件写入 `ext.*` 命名空间。这些字段仅受 CRC32C 保护，且**不包含**在 Ed25519 签名覆盖范围内。

---

## 错误处理

所有 C ABI 函数都返回 `int`。零（`DO_LOG_OK`）表示成功；负值表示错误。

### 错误码分类

错误码空间采用十六进制 nibble 分类：

| 范围 | 类别 | 示例 |
|:-:|:-:|:-:|
| `0x01xx` | 一般 | `DO_LOG_ERR_INTERNAL`、`DO_LOG_ERR_INVALID_ARG` |
| `0x02xx` | 配置 | `DO_LOG_ERR_CONFIG_PARSE`、`DO_LOG_ERR_CONFIG_NOT_FOUND` |
| `0x03xx` | 插件 | `DO_LOG_ERR_PLUGIN_LOAD_FAILED`、`DO_LOG_ERR_PLUGIN_NOT_FOUND` |
| `0x04xx` | 记录 / 字段 | `DO_LOG_ERR_FIELD_NOT_FOUND`、`DO_LOG_ERR_FIELD_PERMISSION_DENIED` |
| `0x05xx` | 环形 / 管道 | `DO_LOG_ERR_BUFFER_FULL`、`DO_LOG_ERR_PIPELINE_STAGE` |
| `0x06xx` | 签名 / 审计 | `DO_LOG_ERR_SIGN_FAILED`、`DO_LOG_ERR_VERIFY_FAILED` |
| `0x07xx` | Sink / I/O | `DO_LOG_ERR_SINK_WRITE_FAILED`、`DO_LOG_ERR_WORM_WRITE_FAILED` |
| `0x08xx` | 沙箱 / 安全 | `DO_LOG_ERR_SANDBOX_INIT_FAILED`、`DO_LOG_ERR_SANDBOX_VIOLATION` |
| `0x09xx` | 资源 / 配额 | `DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED` |
| `0x0Bxx` | 合规 | `DO_LOG_ERR_COMPLIANCE_VIOLATION`、`DO_LOG_ERR_CIRCULAR_DEPENDENCY` |

### 获取详细错误信息

```c
typedef struct {
    int32_t  code;            // 错误码（十六进制 nibble 格式）
    char     message[256];    // 人类可读描述
    char     source_file[128]; // 错误来源文件
    uint32_t source_line;     // 错误来源行号
    uint8_t  _reserved[12];   // 保留 — 必须清零
} dologger_error_t;

int32_t dologger_get_last_error(const dologger_handle_t *handle,
                                dologger_error_t *err);
```

### 诊断日志

引擎详细诊断信息写入当前工作目录下的 `dologger_internal.log`（权限 0600）。该文件包含：

- 插件加载 / 卸载事件及完整的符号解析轨迹
- 配置解析警告与严格模式违规
- 沙箱策略执行决策
- 内部断言失败

**不要**依赖程序化解析该文件。请使用 `dologger_get_last_error()` 获取机器可读的错误详情。

---

## 回调 Sink 注册

> [!NOTE]
> 下面的 C 注册 API 处于规划中 — 已发布的 v0.1.0 头文件中没有 `dologger_register_callback_sink` 符号。Rust 引擎内部已有回调 Sink（`core/src/sink/callback.rs`，以 `dologger_core::sink_callback` 暴露），该 API 将对其进行封装。

宿主应用将能够注册回调，在进程内接收格式化后的日志数据，绕过外部 Sink：

```c
// （伪代码 — 示意，未编译；规划中的 API）
typedef void (*dologger_sink_callback_t)(
    const uint8_t *data,       // 格式化输出字节（可能不以 NUL 结尾）
    size_t         length,     // 格式化数据长度
    void          *user_data   // 注册时传入的不透明用户指针
);

int dologger_register_callback_sink(
    dologger_handle_t        *handle,
    dologger_sink_callback_t  callback,
    void                     *user_data
);
```

**使用示例：**

```c
static void my_callback(const uint8_t *data, size_t len, void *user) {
    FILE *fp = (FILE *)user;
    fwrite(data, 1, len, fp);
    fputc('\n', fp);
}

int main(void) {
    dologger_error_t err = {0};
    dologger_handle_t *logger = dologger_init(NULL, &err);

    FILE *fp = fopen("app_output.log", "a");
    dologger_register_callback_sink(logger, my_callback, fp);

    // ... 应用逻辑 ...

    dologger_shutdown(logger);
    fclose(fp);
    return 0;
}
```

**约束：**

- 回调在 Pipeline 线程上执行。保持其轻量快速 — 不得阻塞 I/O、不得加锁。
- `data` 缓冲区仅在回调执行期间有效。如需保留数据请自行拷贝。
- 每个引擎实例最多注册 8 个回调 Sink。

---

## 线程安全模型

| 组件 | 并发机制 |
|:-:|:-:|
| 环形缓冲区生产者侧 | 无锁 CAS（每线程单生产者优化） |
| 环形缓冲区消费者侧 | 每个域一个消费者线程 |
| Pipeline 工作线程池 | 工作窃取线程池（tokio） |
| 配置存储 | `Arc<RwLock<Config>>` + 写时复制快照 |
| 插件注册表 | `Arc<RwLock<PluginRegistry>>`（仅冷路径） |
| 错误状态（`last_error`） | 线程本地存储 |

### 保证

- 所有 `dologger_*` API 调用可从任意线程并发安全调用。
- 日志提交（`dologger_log`）是信号安全且可重入的 — 可在信号处理器中调用（但不建议在信号处理器内分配富元数据标签）。
- 关闭会阻塞直到所有 in-flight 记录排空（graceful 模式），或立即终止（immediate 模式）。

### 已知限制

环形缓冲区不支持跨线程的真正多生产者无锁入队。多个生产者线程会在同一个 CAS 游标上竞争。实际应用中，约 8 个并发生产者线程以内是可接受的。超过该数量，建议使用分片环形缓冲区（规划中）。

---

## 语言适配器

### Rust Crate 集成

工作区中提供两个 crate：`dologger-core`（引擎，位于 `core/`）和 `dologger-sdk`（易用的 `Logger` 封装，位于 `adapters/rust/`）。仓库内消费者使用路径依赖：

```toml
# Cargo.toml
[dependencies]
dologger-core = { path = "../dologger/core" }
dologger-sdk = { path = "../dologger/adapters/rust" }
```

```rust
use dologger_core::config::DologgerConfig;
use dologger_core::Engine;
use dologger_sdk::Logger;

fn main() {
    // 底层 core API
    let config = DologgerConfig::default();
    let mut engine = Engine::init(config).expect("engine init");
    engine.shutdown();

    // 高层 SDK 封装（推荐宿主使用）
    let mut logger = Logger::init(None).expect("sdk init");
    logger.info("Hello from Rust host");
    logger.shutdown();
}
```

SDK（`dologger_sdk::Logger`）在 `Engine` 之上提供级别辅助函数（`trace` … `audit`）。RAII 风格的 `Drop`、`serde` 反序列化以及 `DologgerConfig` 的 builder 将在后续版本中提供。

### Python（规划中）

（伪代码/示意 — 打包的托管适配器为规划中；仓库已附带可运行的 ctypes 适配器（`adapters/python/dologger.py`），其 `DoLogger` 类可通过 `from dologger import DoLogger` 导入，并已随 v0.1.0 实测运行。以下代码为规划的接口的示意预览（伪代码，不可直接运行）：）

```python
import dologger

logger = dologger.Logger(config_path="/etc/dologger/default.toml")
logger.info("Hello from Python", extra={"request_id": "abc-123"})
logger.shutdown()
```

Python 适配器使用 `ctypes` 加载 `libdologger_core`，并提供兼容 `logging.Handler` 的接口。

### Go（规划中）

（伪代码/示意 — 打包的托管适配器为规划中；仓库已附带 `adapters/go`（模块 `github.com/dologger/adapters/go`）。以下代码为规划的接口的示意预览（伪代码，不可直接运行）：）

```go
package main

import "github.com/Nekolio/DoLogger-go"

func main() {
    logger, err := dologger.New(dologger.Config{
        Level:   "INFO",
        Profile: "prod-performance",
    })
    if err != nil {
        panic(err)
    }
    defer logger.Shutdown()

    logger.Info("Hello from Go")
}
```

Go 适配器使用 cgo 链接 `libdologger_core`。

---

## 性能调优

### 关键调优参数

| 参数 | 默认值 | 建议 | 影响 |
|:-:|:-:|:-:|:-:|
| `ring_buffer_size` | 262144 | 突发型工作负载可调大 | 更大的缓冲 = 更高的峰值吞吐。必须是 2 的幂。 |
| `batch_size` | 256 | 视记录大小在 128–512 之间 | 更大的批次 = 更高吞吐、更高延迟。 |
| `enable_signature` | false | 开发环境 `false`；审计生产环境 `true` | 签名每条记录增加约 17 us（Ed25519）。 |
| `fsync_on_write` | false | WORM 审计 Sink 设为 `true` | 强制介质持久化；受 I/O 延迟约束。 |

### 基准测试

```bash
# 运行内置基准测试套件（core/benches：throughput、latency、latency_percentiles）
cargo bench --bench throughput

# 使用 perf 剖析（Linux）
perf record --call-graph dwarf -- cargo bench --bench throughput
perf report
```

### 代表性性能数据

硬件：AMD Ryzen 9 7950X、DDR5-6000、Samsung 990 Pro NVMe。

| 场景 | 吞吐量（条/秒） | P50 延迟 | P99 延迟 |
|:-:|:-:|:-:|:-:|
| Console Sink，签名关闭 | 1,200,000 | 82 ns | 210 ns |
| File Sink，签名关闭 | 950,000 | 105 ns | 380 ns |
| File Sink，签名开启 | 58,000 | 17.1 us | 22.3 us |
| WORM Sink，签名 + fsync | 12,000 | 83.4 us | 140 us |

### 操作系统级调优

```bash
# Linux：将 Pipeline 线程绑定到隔离 CPU
sudo cset shield --cpu 2-3 --kthread=on

# 增大环形缓冲区的最大锁定内存（使用大页时）
sudo sysctl -w vm.max_map_count=262144

# 测量延迟时关闭透明大页
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled
```

---

## 故障排除

### 常见问题

| 症状 | 可能原因 | 解决方法 |
|:-:|:-:|:-:|
| `dologger_init()` 返回非零 | 配置缺失或 TOML 语法无效 | 检查 `dologger_internal.log`（0600 权限）中的解析错误 |
| 日志未出现在输出中 | Filter 插件丢弃记录，或 Sink 熔断器断开 | 检查 `stderr` 上的 sysmon 事件是否为 `SHM_DROP` 或 `SINK_CIRCUIT_OPEN` |
| 性能低于预期 | 性能 Profile 选择错误或签名开销 | 运行 `cargo bench` 获取基线；核对配置中的 `performance_profile` |
| Windows 上日志文件无法删除 | 文件句柄未释放 | 使用 `FILE_SHARE_DELETE`；滚动前先关闭句柄 |
| 环形缓冲区溢出（应急文件） | 消费者跟不上生产者速率 | 增大 `ring_buffer_size` 或切换到 `prod-performance` Profile |
| sysmon 出现 `SIGNATURE_FAILURE` | 日志文件被篡改或密钥不匹配 | 运行 `dologctl verify-log` 定位被篡改的记录 |
| 插件加载失败 | ABI 版本不匹配或缺少依赖 | 检查 `manifest.toml` 的 `abi_version` 字段；见 [插件开发指南](PluginDevelopmentGuide.md) |

### 诊断检查清单

1. **引擎健康**：`curl http://127.0.0.1:9090/status`（伪代码/示意 — v0.1.0 中控制面未启动）
2. **Sysmon 事件**：重定向 `stderr`，关注 `PIPELINE_BACKLOG`、`SHM_DROP`、`SINK_CIRCUIT_OPEN`、`SANDBOX_VIOLATION`、`SIGNATURE_FAILURE`。
3. **内部日志**：`tail -f dologger_internal.log`
4. **配置**：`dologctl config validate --config /path/to/dologger.toml --strict`
5. **插件状态**：`dologctl plugin list --output json`

### 收集调试报告

```bash
# `dologctl diag collect` 尚在规划中；目前请手动收集以下材料：
dologctl about --output json > diag-report.json
dologctl config validate --strict
```

规划中的归档将包含：
- `dologger_internal.log`
- 当前生效配置（敏感值已脱敏）
- 插件加载清单
- 环形缓冲区统计快照
- 操作系统资源限制（等价于 `ulimit -a`）

在项目仓库提交 Bug 时请附带此报告。
