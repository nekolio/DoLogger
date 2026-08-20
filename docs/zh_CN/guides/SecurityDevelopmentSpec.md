# DoLogger 安全开发规范

> 🌐 **语言 / Language**: [中文](SecurityDevelopmentSpec.md) | [English: DoLogger Security Development Specification](../../en_US/guides/SecurityDevelopmentSpec.md)

> **版本**: v0.0.1 | **最后更新**: 2026-08-12 | **目标受众**: 插件开发者、核心贡献者、安全审计人员
>
> **用途**: 本文档定义 DoLogger 插件开发的强制性安全编码标准。涵盖内存安全、输入验证、沙箱模型、密钥处理、加密指导、模糊测试要求和静态分析工具。所有插件无论信任颜色均须遵守本规范。
>
> **阅读路径**: 所有插件开发者必须阅读[内存安全规则](#内存安全规则)和[输入验证](#输入验证)。面向审计部署的插件作者还必须阅读[密钥和关键材料处理](#密钥和关键材料处理)。安全审计人员应从[插件沙箱模型](#插件沙箱模型)和[模糊测试要求](#模糊测试要求)开始。

## 目录

1. [内存安全规则](#内存安全规则)
2. [输入验证](#输入验证)
3. [插件沙箱模型](#插件沙箱模型)
4. [密钥和关键材料处理](#密钥和关键材料处理)
5. [加密规范：该做与不该做](#加密规范该做与不该做)
6. [模糊测试要求](#模糊测试要求)
7. [静态分析工具链](#静态分析工具链)
8. [安全代码审查检查清单](#安全代码审查检查清单)
9. [漏洞披露](#漏洞披露)

---

## 内存安全规则

### 核心原则

**DoLogger 插件代码绝不能成为宿主进程中内存安全违规的来源。** 由于插件在进程内运行（即使是被沙箱保护的），插件中的内存损坏可能危及整个应用程序。

### 强制规则

**表 1：内存安全规则**

| 规则 | 描述 | 执行方式 |
|:-:|:-:|:-:|
| **R1：无未审查的 unsafe 内存操作** | Rust 插件中不得使用未经明确审查和理由说明的 `unsafe` 块。C 插件不得执行无匹配 alloc/free 对且未经 Valgrind 验证的手动内存管理。 | 代码审查 + Valgrind |
| **R2：边界检查** | 每次数组访问、字符串操作和缓冲区写入必须进行边界检查。C 插件：使用 `snprintf` 而非 `sprintf`，`strncpy` 而非 `strcpy`，大小跟踪缓冲区而非原始指针。 | 静态分析（参见[静态分析工具链](#静态分析工具链)） |
| **R3：无 use-after-free** | 不得保留指向已释放内存的指针。释放后将指针设为 `NULL`。在 Rust 中，这由借用检查器强制执行——只有 `unsafe` 代码可能违反此规则。 | Valgrind / AddressSanitizer |
| **R4：无 double-free** | 每个分配恰好释放一次。在调试构建中使用分配跟踪检测 double-free。 | 调试分配器 + `dologger_internal.log` |
| **R5：无缓冲区溢出** | 所有 VTable 函数输出缓冲区由引擎调用方分配。插件**不得**写入超过提供的 `length` 参数的范围。如果缓冲区不足，返回 `DO_LOG_ERR_BUFFER_TOO_SMALL`。 | 模糊测试（参见[模糊测试要求](#模糊测试要求)） |
| **R6：栈保护** | C 插件：使用 `-fstack-protector-strong` 编译。Rust 插件：通过 LLVM 自动提供。 | 编译器标志 |
| **R7：整数溢出** | 对所有大小计算（尤其是缓冲区大小计算）使用检查算术。Rust：使用 `checked_add`、`saturating_add`，或在 release 中启用 `overflow-checks = true`。C：使用 `__builtin_add_overflow`。 | 静态分析 + 模糊测试 |

### 规则 R1 详解：Unsafe 块

Rust 插件必须将每个 `unsafe` 块视为安全责任：

(伪代码 — 教学片段（`record_ptr` 未定义，非完整可编译代码），仅演示注释规范)：

```rust
// 必须：每个 unsafe 块必须附有 SAFETY 注释，
// 解释为什么它是安全的，而不仅仅是它做什么。

// 好的写法：
// SAFETY: 引擎保证 record_ptr 在此 VTable 调用期间
// 是一个有效的非空指针。我们只读取它，从不修改。
let record = unsafe { &*record_ptr };

// 不好的写法：
let record = unsafe { &*record_ptr }; // 无解释
```

违反 R1 的后果：
- **Blue 插件**：代码审查拒绝；必须在合并前修复
- **Yellow 插件**：如果 `unsafe` 数量超过 5 且无理由，插件加载被拒绝
- **Red 插件**：在默认安全策略下，插件完全不能包含 `unsafe` 块

### 内存所有权规则

不要释放引擎拥有的内存，也不要假设引擎会释放您分配的内存。完整的所有权矩阵请参见[插件开发指南](PluginDevelopmentGuide.md#内存所有权规则)。

---

## 输入验证

### 必须验证的内容

从插件自身代码之外接收的所有数据必须被视为**不受信任的**并在使用前验证。

**表 2：输入验证要求**

| 输入来源 | 所需验证 | 理由 |
|:-:|:-:|:-:|
| `dologger_record_t *` 字段 | 解引用前对所有指针字段进行空检查。验证 `level` 在 0-6 范围内。 | 插件从异步管道接收记录；损坏的指针是灾难性的。 |
| `dologger_plugin_config_t *` | 使用前验证所有配置值。检查字符串长度、数值范围、枚举值。 | 配置从可能被用户编辑或损坏的 TOML 文件加载。 |
| 记录中的字符串 | 将所有字符串视为可能包含空字节、控制字符或过长值。在合理最大值处截断。 | 日志注入攻击（CRLF 注入、终端转义序列）源于未验证的字符串。 |
| 数值字段 | 用作数组索引或大小参数前进行边界检查。 | 整数溢出或越界访问。 |
| 批量数组（`records`、`count`） | 迭代前验证 `count > 0`。验证每个数组元素非空。 | 针对引擎 Bug 或恶意插件链的防御性编程。 |

### 验证模式（C）

(伪代码 — 教学示例，仅示意验证模式；`dologger_filter_result_t`、`DO_LOG_MAX_MESSAGE_LEN` 等符号在 v0.0.1 中不存在)：

```c
dologger_error_t my_filter(dologger_record_t *record,
                           dologger_filter_result_t *result) {
    // 规则 1：对所有指针参数进行空检查
    if (record == NULL || result == NULL) {
        return DO_LOG_ERR_INVALID_ARG;
    }

    // 规则 2：使用前验证记录字段
    if (record->level > DO_LOG_AUDIT) {
        // 无效级别——丢弃可疑记录，不崩溃
        result->action = DO_LOG_FILTER_DROP;
        return DO_LOG_OK;  // 返回 OK 以使管道继续
    }

    // 规则 3：验证字符串指针和长度
    if (record->message != NULL) {
        // 强制最大消息长度以防止内存耗尽
        size_t msg_len = strnlen(record->message, DO_LOG_MAX_MESSAGE_LEN);
        if (msg_len >= DO_LOG_MAX_MESSAGE_LEN) {
            // 消息过长——丢弃，不崩溃
            result->action = DO_LOG_FILTER_DROP;
            return DO_LOG_OK;
        }
    }

    // ... 业务逻辑 ...

    return DO_LOG_OK;
}
```

### 验证模式（Rust）

(伪代码 — 教学示例，仅示意验证模式；core 中无 `FilterResult`/`FilterAction`/`DoLogError` 类型)：

```rust
fn my_filter(record: &Record, result: &mut FilterResult) -> DoLogError {
    // 规则 1：验证级别
    if record.level > LogLevel::Audit as u8 {
        result.action = FilterAction::Drop;
        return Ok(());
    }

    // 规则 2：验证消息
    if let Some(msg) = record.message() {
        if msg.len() > DO_LOG_MAX_MESSAGE_LEN {
            result.action = FilterAction::Drop;
            return Ok(());
        }
    }

    // ... 业务逻辑 ...

    Ok(())
}
```

### 原则

1. **安全失败，而非开放失败**：当验证失败时，默认操作必须是丢弃/舍弃，而非放行。
2. **永不崩溃**：不要在 VTable 函数中 `panic!()`、`abort()` 或 `exit()`。返回错误码。
3. **记录违规**：通过引擎的诊断日志报告验证失败，以便运维人员检测攻击。
4. **纵深防御**：在插件边界进行验证，即使引擎也进行验证。插件可能以意外顺序或组合加载。

---

## 插件沙箱模型

### 沙箱的作用

沙箱限制插件可以执行哪些操作系统操作。它在 `dlopen()` **之后**、`plugin_init()` **之前**应用。一旦沙箱激活，就不能放宽——只能收紧。

**表 3：按信任颜色的沙箱能力**

| 能力 | Blue | Yellow | Red |
|:-:|:-:|:-:|:-:|
| 内存分配（`mmap`、`munmap`、`brk`） | 是 | 是 | 是 |
| 线程操作（`clone`、`futex`） | 是 | 是 | 是 |
| 时间函数（`clock_gettime`） | 是 | 是 | 是 |
| 文件 I/O（`open`、`read`、`write`、`close`） | 是 | 是 | **否** |
| 网络（`socket`、`connect`、`sendto`） | 是 | **否** | **否** |
| 进程创建（`fork`、`execve`） | 是 | **否** | **否** |
| 信号处理（`sigaction`、`tgkill`） | 是 | 是 | **否** |

### 沙箱对插件开发者的意义

**如果您在开发 Blue 插件**：沙箱不应用。您拥有对操作系统的完全访问权限。但您应负责任地使用该访问权限——您以宿主应用程序的权限运行，插件中的漏洞就是应用程序中的漏洞。

**如果您在开发 Yellow 插件**：沙箱部分应用。

- 您**可以**读写文件（用于配置、状态持久化、临时数据）。
- 您**不能**打开网络连接。如果您的插件需要网络访问（例如从远程 URL 获取配置的 `ConfigProvider`），您必须请求 Blue 信任并提供适当的理由。
- 您**不能**创建子进程。使用引擎内置的并行机制——不要 fork。
- 尝试禁止的系统调用导致**立即线程终止**（Linux 上为 `SECCOMP_RET_KILL_PROCESS`）。没有错误码，没有恢复——插件线程死亡。

**如果您在开发 Red 插件**：沙箱最大程度限制。

- 您**不能**访问文件系统、网络或创建进程。
- 您**可以**分配内存、使用线程和查询时间。这对无状态或纯计算插件（例如检查记录字段的 Filter、编辑文本的 Processor）足够。
- 所有输出进入 `ext.*` 字段命名空间（Ring 3，仅 CRC32C 完整性）。您不能写入 `verified.*` 命名空间。
- Red 插件**默认禁用**。宿主运维人员必须显式设置 `allow_red_plugins = true`。

### 在沙箱约束下开发

(伪代码 — 教学示例，仅示意沙箱约束；v0.0.1 实际插件入口为 `int plugin_init(const void *config)`，`dologger_plugin_config_t` 不存在)：

```c
// YELLOW 插件：不要这样做——网络被拒绝
dologger_error_t my_plugin_init(const dologger_plugin_config_t *config) {
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    // 这将在 Linux 上触发 SECCOMP_RET_KILL_PROCESS。
    //   您的插件线程死亡。没有错误码。没有恢复。
}

// YELLOW 插件：改为这样做——使用引擎的 ConfigProvider 链：
dologger_error_t my_plugin_init(const dologger_plugin_config_t *config) {
    // 引擎提供配置。使用它。
    const char *remote_url = dologger_config_get(config, "remote_url");
    // 如果远程获取是必需的，请升级到 Blue 信任。
}
```

### 测试沙箱合规性

发布前在沙箱约束下测试您的插件：

```bash
# Linux：在 strace 下运行以审计系统调用使用
sudo strace -f -e trace=file,network,process \
    ./target/debug/examples/simple_logger 2>&1 | grep -v ENOENT

# 检查：是否有意外的 open()、socket()、fork() 调用？

# 强制 Yellow 沙箱以测试 Blue 插件
# （编辑 dologger.toml：trust.color = "yellow" 用于测试运行）
```

---

## 密钥和关键材料处理

### 首要指令

**永远不要记录密钥。** 日志引擎是泄漏凭据的最糟糕地方——日志被持久化、复制、发送到集中平台，并可能保留多年。

### 规则

**表 4：密钥处理规则**

| 规则 | 描述 |
|:-:|:-:|
| **S1：永不记录原始密钥** | 不要将 API 密钥、密码、令牌、私钥、会话 cookie 或 PII 写入任何 `DO_LOG_*` 调用或任何 `record.message` 字段。 |
| **S2：永不将密钥存储在插件状态中** | 插件状态在热重载时被序列化。`dologger_state_buf_t` 以明文存储。不要将关键材料放在序列化状态中。 |
| **S3：使用 SecretDetector API** | 如果您的插件处理可能包含密钥的文本，在记录或格式化之前调用引擎的 `dologger_secret_scan()` API。 |
| **S4：传输前脱敏** | 如果 `Processor` 插件使用敏感上下文（例如用户 PII）丰富记录，请确保下游 Processor 或 Filter 在记录到达网络接收器之前对其进行脱敏或掩码。 |
| **S5：KeyProvider 用于密钥** | 如果您的插件需要签名或加密密钥，不要硬编码、从配置读取或存储在插件内存中。使用带 HSM/KMS 支持的 `KeyProvider` 插件。 |
| **S6：审计所有密钥访问** | 每次您的插件访问密钥（读取密钥、解密数据）时，发出 `DO_LOG_AUDIT` 记录记录该访问。 |

### 使用 SecretDetector API

(伪代码 — v0.0.1 无 `dologger_secret_scan()` C 导出；核心内的 SecretDetector 为 Rust 内部 API（`core/src/security/secret_detector.rs`）)：

```c
// 在记录可能包含密钥的文本之前，扫描它
dologger_secret_scan_result_t scan_result;
dologger_error_t rc = dologger_secret_scan(
    untrusted_input_text,
    strlen(untrusted_input_text),
    &scan_result
);

if (scan_result.secret_detected) {
    // 用占位符替换检测到的密钥
    // 例如 "api_key=sk-abc123" -> "api_key=<REDACTED>"
    memset(scan_result.secret_start, '*', scan_result.secret_length);
    // 发出审计记录记录脱敏操作
    DO_LOG_AUDIT(logger, "SecretDetector: redacted %zu bytes at offset %zu",
                 scan_result.secret_length, scan_result.secret_offset);
}
```

### SecretDetector 检测的内容

内置 `SecretDetector` 扫描以下模式：

| 模式 | 示例 | 正则表达式 |
|:-:|:-:|:-:|
| AWS 访问密钥 | `AKIAIOSFODNN7EXAMPLE` | `AKIA[0-9A-Z]{16}` |
| Stripe API 密钥 | `sk_live_example_placeholder_key` | `sk_live_[0-9a-zA-Z]{24}` |
| GitHub Token | `ghp_example_placeholder_token` | `ghp_[0-9a-zA-Z]{36}` |
| JWT Token | `eyJhbGciOiJIUzI1NiIs...` | `eyJ[0-9a-zA-Z_-]+` |
| 私钥（PEM） | `-----BEGIN RSA PRIVATE KEY-----` | `-----BEGIN .* PRIVATE KEY-----` |
| URL 中的密码 | `postgres://user:secret@host/db` | 带密码组件的 URI |
| Base64 高熵值 | 40+ 字符 base64，熵 > 4.5 bits/char | Shannon 熵检查 |

可通过 `SecretDetector` Processor 插件配置添加自定义模式。

---

## 加密规范：该做与不该做

### 批准的算法

**表 5：加密算法策略**

| 操作 | 使用 | 不要使用 |
|:-:|:-:|:-:|
| 签名 | Ed25519（通过 `KeyProvider`） | RSA、DSA、ECDSA |
| 哈希 | SHA-256、SHA-512 | MD5、SHA-1 |
| 完整性（非加密） | CRC32C（硬件加速） | CRC32、Adler-32 |
| 静态加密 | AES-256-GCM | AES-ECB、DES、3DES |
| 密钥交换 | X25519（如需插件间通信） | DH-1024、静态 RSA |
| 随机数 | OS CSPRNG（`getrandom` 系统调用，`/dev/urandom`） | `rand()`、`srand()`、`drand48()` |

### 加密该做的事

1. **要做**：将签名委托给引擎的 `KeyProvider` 链。不要自己实现签名。
2. **要做**：对所有安全敏感数据使用常量时间比较（C 中使用 `CRYPTO_memcmp`，Rust 中使用 `subtle` crate）。
3. **要做**：使用后清零关键材料（`explicit_bzero` / `zeroize` crate）。
4. **要做**：将密钥存储在 HSM/KMS 支持的 KeyProvider 插件中，永远不存储在插件状态或配置文件中。
5. **要做**：在依赖审计记录内容之前验证其 Ed25519 签名（如果您的插件消费审计数据）。

### 加密不该做的事

1. **不要**：实现自己的加密算法。使用引擎的 API 或经过良好审计的库（`ring`、`ed25519-dalek`、`rustls`）。
2. **不要**：使用可预测或低熵种子的随机数。始终从 `/dev/urandom` 或 `getrandom()` 获取种子。
3. **不要**：对签名和加密重复使用相同的密钥。不同用途使用不同密钥。
4. **不要**：将已废弃的哈希函数（MD5、SHA-1）用于任何安全目的。CRC32C 仅可用于 Ring 3 数据的非安全完整性检查。
5. **不要**：硬编码加密密钥、IV 或 nonce。每个密钥必须来自 `KeyProvider`。每个 nonce 必须新鲜生成。

### 处理 Ed25519 签名

如果您的插件处理或验证 Ed25519 签名：

(伪代码 — v0.0.1 无 `dologger_verify_record_signature()` C 导出，该接口为规划中)：

```c
// 要做：使用引擎的验证 API
dologger_error_t rc = dologger_verify_record_signature(
    engine_handle,
    record,
    &verification_result    // -> DO_LOG_SIG_VALID / INVALID / NOT_SIGNED
);

// 不要：自己重新实现签名验证
// ed25519_dalek_verify(record->signature, ...)  <-- 不要！
```

引擎管理公钥分发、密钥轮换和 CRL 检查。您的插件不应重复这些基础设施。

---

## 模糊测试要求

### 何时需要模糊测试

**表 6：按插件类型的模糊测试要求**

| 插件类型 | 需要模糊测试？ | 理由 |
|:-:|:-:|:-:|
| `Filter` | 否 | 仅读取记录字段；无解析 |
| `PolicyProvider` | 否 | 仅读取指标计数器 |
| `FieldProvider` | 否 | 仅写入字段；无解析 |
| `HostInfoProvider` | 否 | 读取操作系统 API；无外部输入 |
| `Processor` | **是** | 转换记录内容——可能解析结构化数据 |
| `Formatter` | **是** | 序列化记录——必须处理所有字段值和格式错误的 UTF-8 |
| `ConfigProvider` | **是** | 解析外部配置格式（TOML、JSON、YAML） |
| `KeyProvider` | **是** | 处理加密关键材料和签名操作 |
| `SyscallBroker` | **是** | 拦截并代理任意系统调用参数 |

### 模糊测试目标

对于每个需要模糊测试的插件，提供至少一个模糊测试目标：

(伪代码 — 模糊测试目标模板（`my_formatter`/`mock_record_from_bytes` 为占位符；引擎的真实 fuzz 目标位于 `core/fuzz/fuzz_targets/`：`fuzz_ring_buffer`、`fuzz_sif_record`、`fuzz_toml_config`）：

```rust
// fuzz/fuzz_targets/format_json.rs
#![no_main]

use libfuzzer_sys::fuzz_target;
use my_formatter::format_record;

fuzz_target!(|data: &[u8]| {
    // 从模糊测试器输入构造模拟记录
    if let Ok(record) = mock_record_from_bytes(data) {
        let mut output = vec![0u8; 4096];
        // 格式化器绝不能 panic 或损坏内存
        let _ = format_record(&record, &mut output);
    }
});
```

### 模糊测试要求检查清单

- [ ] 表 6 中标记为"是"的每种插件类型至少一个模糊测试目标
- [ ] 模糊测试目标通过 `cargo fuzz` 链接到 CI
- [ ] 插件发布前 24 小时零崩溃模糊测试
- [ ] 模糊测试期间启用 AddressSanitizer（`-Z sanitizer=address`）
- [ ] 模糊测试语料库签入仓库（`fuzz/corpus/`）
- [ ] OSS-Fuzz 集成（规划中）

### 本地运行模糊测试

(伪代码/示意 — cargo-fuzz 命令语法正确；`format_json` 为占位目标名，请替换为 `core/fuzz/fuzz_targets/` 中的真实目标名)：

```bash
# 安装 cargo-fuzz
cargo install cargo-fuzz

# 运行特定模糊测试目标 60 秒
cargo fuzz run format_json -- -max_total_time=60

# 使用 AddressSanitizer 运行
RUSTFLAGS="-Z sanitizer=address" cargo +nightly fuzz run format_json

# 最小化崩溃输入
cargo fuzz tmin format_json fuzz/artifacts/format_json/crash-xxxxx

# 重放崩溃
cargo fuzz run format_json fuzz/artifacts/format_json/crash-xxxxx
```

### 什么构成模糊测试失败

- **崩溃**：段错误、断言失败、`panic!()`——**阻塞级**。必须在发布前修复。
- **超时**：4 KB 输入下函数耗时 > 5 秒——**警告**。审查算法复杂度攻击。
- **OOM**：分配 > 1 GB——**警告**。审查内存耗尽 DoS。
- **错误输出**：产生未通过插件自身往返测试的输出——**阻塞级**。表明逻辑错误。

---

## 静态分析工具链

### 必需工具和配置

所有 DoLogger 插件必须在 CI 中通过以下静态分析检查：

**表 7：静态分析工具链**

| 工具 | 命令 | 检查内容 | 失败严重性 |
|:-:|:-:|:-:|:-:|
| **cargo audit** | `cargo audit` | 依赖树中的已知漏洞（CVE） | **阻塞级** |
| **cargo deny** | `cargo deny check advisories` | RustSec 安全通告数据库 | **阻塞级** |
| **cargo deny** | `cargo deny check licenses` | 许可证合规（参见[插件开发指南](PluginDevelopmentGuide.md#许可证合规)） | **阻塞级** |
| **cargo deny** | `cargo deny check bans` | 重复 crate 版本、通配符依赖 | **警告** |
| **cargo deny** | `cargo deny check sources` | 未知或不受信任的 crate 源 | **阻塞级** |
| **clippy** | `cargo clippy -- -D warnings` | Rust 惯用写法、正确性 lint、性能 lint | **阻塞级** |
| **rustfmt** | `cargo fmt --check` | 代码格式一致性 | **警告** |

### Cargo Audit 配置

```bash
# 运行 cargo audit——在任何漏洞上失败
cargo audit

# 使用特定严重性阈值运行
cargo audit --deny unsound --deny warnings

# 忽略特定安全通告（需要在 audit.toml 中提供理由）
cargo audit --ignore RUSTSEC-2024-XXXX
```

### Cargo Deny 配置

项目根目录的 `deny.toml` 包含规范的 deny 配置（以下摘录与仓库实际文件一致）：

```toml
# deny.toml（摘录）
[graph]
all-features = true

[licenses]
version = 2
private = { ignore = true }

[licenses.allow]
mit = "allow"
apache-2.0 = "allow"
bsd-2-clause = "allow"
bsd-3-clause = "allow"
isc = "allow"
zlib = "allow"
# ...（详见仓库根目录 deny.toml）...

[licenses.deny]
gpl-2.0-only = "deny"
gpl-2.0-or-later = "deny"
gpl-3.0-only = "deny"
gpl-3.0-or-later = "deny"
agpl-3.0-only = "deny"
agpl-3.0-or-later = "deny"
# ...（详见仓库根目录 deny.toml）...

[bans]
multiple-versions = "warn"
wildcards = "deny"           # 拒绝通配符依赖

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

（注：该文件使用 cargo-deny `version = 2` 的映射格式，需 cargo-deny 1.x+；cargo-deny 0.x 会报 "expected an array" 解析错误。仓库当前未包含 `[advisories]` 节）

### Clippy 配置

```bash
# 运行 clippy 并启用所有 lint
cargo clippy --all-targets --all-features -- -D warnings

# 额外的安全关键 lint
cargo clippy -- -W clippy::unwrap_used \
                 -W clippy::expect_used \
                 -W clippy::integer_arithmetic \
                 -W clippy::cast_possible_truncation \
                 -W clippy::cast_possible_wrap \
                 -W clippy::indexing_slicing
```
### CI 集成

（示例 CI 配置 — YAML 语法有效；仓库当前实际文件为 `.github/workflows/security.yml`）：

```yaml
# .github/workflows/security-checks.yml
name: Security Checks

on: [push, pull_request]

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: cargo install cargo-audit cargo-deny
      - run: cargo audit --deny warnings
      - run: cargo deny check advisories
      - run: cargo deny check licenses
      - run: cargo deny check bans

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: cargo clippy --all-targets --all-features -- -D warnings

  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: cargo fmt --all -- --check
```

### C 插件

C 插件开发者应添加：

| 工具 | 命令 | 检查内容 |
|:-:|:-:|:-:|
| **Valgrind** | `valgrind --leak-check=full --show-leak-kinds=all` | 内存泄漏、use-after-free、double-free、未初始化读取 |
| **AddressSanitizer** | `-fsanitize=address` | 缓冲区溢出、use-after-free、栈溢出 |
| **UndefinedBehaviorSanitizer** | `-fsanitize=undefined` | 整数溢出、空指针解引用、对齐违规 |
| **Coverity / CodeQL** | CI 集成 | 全面的跨过程分析 |

```bash
# C 插件安全构建标志
cc -shared -fPIC \
   -fstack-protector-strong \
   -D_FORTIFY_SOURCE=2 \
   -fsanitize=address \
   -fsanitize=undefined \
   -O2 -g \
   -o my_plugin.so my_plugin.c
```

---

## 安全代码审查检查清单

每个插件代码审查必须在合并前验证以下项目。

### 内存安全

- [ ] 无未附 `// SAFETY:` 理由注释的 unsafe 块
- [ ] 所有指针参数在解引用前进行空检查
- [ ] 所有数组访问进行边界检查
- [ ] 所有缓冲区写入遵守提供的大小限制
- [ ] 大小计算使用检查算术运算
- [ ] 插件不释放引擎拥有的内存

### 输入验证

- [ ] 所有配置值已验证（范围、枚举、长度）
- [ ] 所有记录字段使用前已验证
- [ ] 字符串长度在拷贝前已检查
- [ ] 批量计数在迭代前已验证
- [ ] 无效输入导致 `Drop` 或返回错误，绝不崩溃

### 沙箱合规性

- [ ] 插件的 `manifest.toml` `[capabilities]` 与实际系统调用使用匹配
- [ ] Yellow/Red 插件中无网络操作
- [ ] Yellow/Red 插件中无进程创建
- [ ] Red 插件中无文件 I/O
- [ ] 插件已在沙箱约束下测试

### 密钥与加密

- [ ] 无硬编码密钥、令牌或密码
- [ ] 日志消息中无密钥
- [ ] 序列化插件状态中无密钥
- [ ] 记录不受信任文本前使用 `SecretDetector`
- [ ] 加密操作委托给引擎 API
- [ ] 未使用过时算法（MD5、SHA-1、DES）

### 静态分析

- [ ] `cargo audit` 以零漏洞通过
- [ ] `cargo deny check` 通过（安全通告、许可证、禁用、源）
- [ ] `cargo clippy -- -D warnings` 通过
- [ ] `cargo fmt --check` 通过
- [ ] （C 插件）Valgrind 报告零错误
- [ ] （C 插件）ASan + UBSan 报告零错误

### 模糊测试（如适用）

- [ ] 插件类型存在模糊测试目标
- [ ] 最终提交上 24 小时零崩溃
- [ ] 模糊测试语料库已提交到仓库

---

## 漏洞披露

### 报告漏洞

如果您发现 DoLogger 或任何插件中的安全漏洞：

1. **不要**提交公开 Issue。
2. 发送邮件至 `nekoliowork+DoLogger@gmail.com`，附：
   - 漏洞描述
   - 复现步骤
   - 受影响版本（引擎、插件、平台）
   - 任何概念验证代码
3. 允许最多 72 小时获得初始回复。

### 披露时间线

| 严重性 | 补丁时间线 | 披露方式 |
|:-:|:-:|:-:|
| **严重**（RCE、沙箱逃逸、签名绕过） | 7 天 | 与报告者协调 |
| **高危**（信息泄露、权限提升） | 14 天 | 与报告者协调 |
| **中危**（DoS、轻微数据泄露） | 30 天 | 发布说明中公开披露 |
| **低危**（纵深防御改进） | 下一个发布 | 发布说明中公开披露 |

### 安全通告

已发布的安全通告可在以下位置获取：

```
https://github.com/Nekolio/DoLogger/security/advisories
```

每个通告包含：
- CVE 标识符（如已分配）
- 受影响版本范围
- 已修补版本
- 严重性评级（CVSS v3.1 评分）
- 无法立即升级的用户的缓解措施

### 插件开发者责任

如果在**您的**插件中发现安全漏洞：

1. 您将通过 `manifest.toml` 中的联系邮箱收到通知。
2. 您应在与严重性相应的时限内发布补丁。
3. 如果漏洞严重且在 14 天后仍未修补，插件将从官方插件仓库中移除，并被引擎的安全通告检查列入黑名单。
4. DoLogger 自身的 `cargo audit` / `cargo deny` 管道将在您的插件依赖于带有已知 CVE 的 crate 时标记其存在漏洞。请保持依赖项更新。
