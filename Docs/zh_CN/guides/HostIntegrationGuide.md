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

DoLogger 是一个跨平台、高安全、插件化的日志引擎。宿主应用通过 C ABI 动态链接
`libdologger_core`（.so/.dylib/.dll）来使用日志功能。

### 关键特性

- **C ABI 接口**: 所有公共 API 使用 `dologger_*` 前缀的 C 函数
- **零依赖宿主集成**: 宿主只需链接动态库，无需 Rust 工具链
- **插件化架构**: 10 种插件 VTable 类型，按需加载
- **Ed25519 签名 + LSN 审计链**: 防篡改日志完整性保护
- **三色信任模型**: Blue/Yellow/Red 插件分级隔离

### 支持平台

| 平台 | 架构 | 动态库后缀 |
|:-:|:-:|:-:|
| Linux | x86_64, aarch64 | `.so` |
| macOS | x86_64, aarch64 | `.dylib` |
| Windows | x86_64 | `.dll` |

---

## 快速开始

### 1. 初始化引擎

```c
#include "dologger_core.h"

int main() {
    dologger_handle_t *logger = NULL;
    int rc = dologger_init(NULL, &logger);
    if (rc != DO_LOG_OK) {
        fprintf(stderr, "Failed to initialize DoLogger: %d\n", rc);
        return 1;
    }
    // ... use logger ...
    dologger_shutdown(&logger);
    return 0;
}
```

### 2. 提交日志

```c
dologger_record_params_t params = {
    .level = DO_LOG_INFO,
    .message = "Hello from host application",
    .source_file = "main.c",
    .source_function = "main",
    .source_line = 42,
};
dologger_log(logger, &params);
```

### 3. Cargo 项目集成

在 Rust 宿主中直接使用 `dologger-core` crate：

```rust
use dologger_core::{Engine, DologgerConfig, LogLevel, Record};

fn main() {
    let config = DologgerConfig::default();
    let mut engine = Engine::init(config).expect("Engine init failed");
    // ... use engine ...
    engine.shutdown();
}
```

---

## C ABI 初始化与关闭

### `dologger_init()`

```c
int dologger_init(const dologger_init_params_t *params, dologger_handle_t **handle);
```

**参数**:
- `params`: 初始化参数（可为 NULL 使用默认值）
- `handle`: 输出 — 引擎句柄指针

**返回值**: `DO_LOG_OK` (0) 成功，负数错误码

### `dologger_shutdown()`

```c
int dologger_shutdown(dologger_handle_t **handle);
```

优雅关闭引擎，等待所有 in-flight 日志完成。关闭后 `*handle` 设为 NULL。

---

## 日志提交

### 参数结构

```c
typedef struct {
    uint8_t    level;          // DO_LOG_TRACE(0) ~ DO_LOG_AUDIT(6)
    const char *message;       // UTF-8 日志消息
    const char *source_file;   // __FILE__ (可选)
    const char *source_function; // __FUNCTION__ (可选)
    uint32_t   source_line;    // __LINE__ (可选)
    uint32_t   source_column;  // 0 (可选)
    const char *request_id;    // 请求/追踪 ID (可选)
} dologger_record_params_t;
```

### 日志级别

| 常量 | 值 | 含义 |
|:-:|:-:|:-:|
| `DO_LOG_TRACE` | 0 | 追踪级调试 |
| `DO_LOG_DEBUG` | 1 | 调试信息 |
| `DO_LOG_INFO` | 2 | 一般信息 |
| `DO_LOG_WARN` | 3 | 警告 |
| `DO_LOG_ERROR` | 4 | 错误 |
| `DO_LOG_FATAL` | 5 | 致命错误 |
| `DO_LOG_AUDIT` | 6 | 审计记录（非否认） |

---

## 配置体系

### 配置优先级（低→高）

1. 硬编码默认值
2. 系统默认配置 (`/etc/dologger/` 或 `%PROGRAMDATA%\dologger\`)
3. 项目本地配置（cwd + 父目录遍历）
4. 环境变量 (`DO_LOG_LEVEL`, `DO_LOG_CONFIG_FILE` 等）
5. API 参数 (`dologger_config_load_from_string()`)
6. Record 元数据标签
7. 安全不可降级项（绝对不可绕过的硬性限制）

### 性能 Profile

| Profile | block_timeout_ms | drop_strategy | 适用场景 |
|:-:|:-:|:-:|:-:|
| `dev` | 100 | drop_newest | 开发环境 |
| `prod-performance` | 3000 | below_warn | 高性能生产 |
| `prod-audit` | 3000 | below_warn | 审计合规生产 |
| `balanced` | 2000 | oldest | 均衡模式 |

---

## Record 字段权限环

| Ring | 名称 | 写入权限 | 读取权限 |
|:-:|:-:|:-:|:-:|
| Ring 0 | 内核核心 | 仅核心引擎 | 格式化器/Sink 只读 |
| Ring 1 | 系统受信 | 核心 + HostInfoProvider | 所有插件只读 |
| Ring 2 | 已验证插件 | Blue/Yellow 插件 | 所有插件 |
| Ring 3 | 不可信扩展 | 任何插件 | 任何插件 (CRC32C) |

---

## 错误处理

所有 API 返回 `int` 错误码。零表示成功，负数表示错误。

### 获取详细错误

```c
int dologger_get_last_error(dologger_handle_t *handle, dologger_error_info_t *err_info);
```

错误码采用十六进制 nibble 分类：
- `0x01xx` — 一般错误
- `0x02xx` — 配置错误
- `0x03xx` — IO 错误
- `0x04xx` — 签名/安全错误
- `0x05xx` — 插件错误

---

## 回调 Sink 注册

宿主可注册回调接收格式化后的日志数据：

```c
typedef void (*dologger_sink_callback_t)(
    const uint8_t *data,
    size_t length,
    void *user_data
);
dologger_register_callback_sink(logger, my_callback, user_data);
```

---

## 线程安全模型

- 所有 `dologger_*` API 是线程安全的
- 环形缓冲区使用无锁 CAS 操作支持多生产者
- 后台 Pipeline 线程执行格式化和 Sink 写入
- 共享状态使用原子操作 + 细粒度锁

---

## 语言适配器

### Rust

```rust
// Cargo.toml: dologger-core = "0.1"
use dologger_core::Engine;
```

### Python (M4)

```python
import dologger
logger = dologger.Logger(config_path="/etc/dologger/default.toml")
logger.info("Hello from Python")
```

### Go (M4)

```go
import "github.com/Nekolio/DoLogger-go"
```

---

## 性能调优

| 参数 | 建议值 | 影响 |
|:-:|:-:|:-:|
| `ring_buffer_size` | 262144 | 更大的缓冲 = 更高的突发吞吐 |
| `batch_size` | 256 | 更大的批次 = 更高的吞吐，更高的延迟 |
| `enable_signature` | false | 开发环境关闭签名可降低延迟 ~150x |

---

## 故障排除

### 常见问题

1. **`dologger_init` 返回错误** → 检查 `dologger_internal.log` 诊断日志（0600 权限）
2. **日志未输出** → 检查 sysmon 输出 `stderr` 是否有 `SHM_DROP` 或 `SINK_CIRCUIT_OPEN` 事件
3. **性能不达标** → 运行 `cargo bench` 确认基线数据，检查配置中是否正确选择了性能 Profile
4. **Windows 上日志文件无法删除** → 使用 `FILE_SHARE_DELETE` 标志；滚动前先关闭文件句柄
