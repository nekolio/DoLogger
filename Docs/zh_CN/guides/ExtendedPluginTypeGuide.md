# DoLogger 扩展插件类型开发指南

> 🌐 **语言 / Language**: [中文](ExtendedPluginTypeGuide.md) | [English: DoLogger Extended Plugin Type Development Guide](../../en_US/guides/ExtendedPluginTypeGuide.md)

> **版本**: v0.1.0 | **最后更新**: 2026-08-12 | **目标受众**: 高级插件开发者、核心贡献者
>
> **用途**: 本文档为 DoLogger 中全部 10 种 VTable 插件类型的实现提供高级指导。涵盖选择插件类型的设计决策、沙箱感知的 SyscallBroker 实现、自定义 PolicyProvider 模式、插件依赖管理、用于热重载的状态序列化以及多阶段插件注册。
>
> **阅读路径**: 已阅读[插件开发指南](PluginDevelopmentGuide.md)的插件开发者应从[选择合适的插件类型](#选择合适的插件类型)开始。实现安全关键插件的开发者应阅读 [SyscallBroker 实现](#syscallbroker-实现)和[多阶段插件](#多阶段插件)。插件生态维护者应审查[插件依赖管理](#插件依赖管理)。

## 目录

1. [选择合适的插件类型](#选择合适的插件类型)
2. [ConfigProvider vs KeyProvider：何时使用每个](#configprovider-vs-keyprovider何时使用每个)
3. [SyscallBroker 实现](#syscallbroker-实现)
4. [自定义 PolicyProvider 模式](#自定义-policyprovider-模式)
5. [插件依赖管理](#插件依赖管理)
6. [用于热重载的插件状态序列化](#用于热重载的插件状态序列化)
7. [多阶段插件](#多阶段插件)
8. [高级插件架构模式](#高级插件架构模式)

---

## 选择合适的插件类型

### 决策矩阵

当您有一个日志扩展想法时，使用此决策表选择合适的 VTable 插件类型。

```mermaid
flowchart TD
    Q{"您的插件需要做什么？"}
    Q -->|"决定保留或丢弃哪些记录"| A["Filter<br/>挂载阶段：filter（阶段 1）<br/>关键问题：此记录应继续通过管道吗？"]
    Q -->|"控制日志的速率或量级"| B["PolicyProvider<br/>挂载阶段：prefilter（阶段 0）<br/>关键问题：每秒应通过多少条记录？"]
    Q -->|"为每条记录添加元数据"| M{"主机/环境元数据还是应用程序/业务元数据？"}
    M -->|"主机/环境元数据（PID、主机名、容器 ID）"| C["HostInfoProvider<br/>挂载阶段：field（阶段 2）<br/>具有 Ring 1 写入权限"]
    M -->|"应用程序/业务元数据（用户 ID、会话 ID、追踪 ID）"| D["FieldProvider<br/>挂载阶段：field（阶段 2）<br/>具有 Ring 2 写入权限（Blue/Yellow）或 Ring 3（Red）"]
    Q -->|"转换或脱敏日志内容"| E["Processor<br/>挂载阶段：process（阶段 4）<br/>关键问题：记录是否需要 PII 掩码、丰富或重构？"]
    Q -->|"更改记录的序列化方式"| F["Formatter<br/>挂载阶段：format（阶段 5）<br/>关键问题：输出应为 JSON、CSV、protobuf 还是自定义二进制格式？"]
    Q -->|"将记录写入目标位置"| G["IOSink<br/>挂载阶段：sink（阶段 6）<br/>关键问题：格式化后的记录应发往哪里？文件、网络、数据库？"]
    Q -->|"从外部源加载配置"| H["ConfigProvider<br/>挂载阶段：config（加载时，不在管道中）<br/>关键问题：配置是否来自 Vault、etcd、S3 或数据库？"]
    Q -->|"管理加密密钥"| I["KeyProvider<br/>挂载阶段：key（加载时，不在管道中）<br/>关键问题：签名密钥应来自 HSM、KMS 还是文件？"]
    Q -->|"为沙箱插件中介操作系统访问"| J["SyscallBroker<br/>挂载阶段：syscall（代理，不在管道中）<br/>关键问题：沙箱插件能否安全地执行文件 I/O 或网络调用？"]
(伪代码 — 示意性 VTable 草图；v0.1.0 实际定义见 `core/include/dologger_core.h`（`dologger_config_provider_vtable_t`：`open`/`read_config`/`close`）)：
```

### 插件类型能力

**表 1：插件类型能力矩阵**

| 插件类型 | 可丢弃记录？ | 可修改记录？ | Ring 访问（写入） | 管道阶段 |
|:-:|:-:|:-::|:-:|:-:|
| `Filter` | **是** | 否 | 无（只读） | 1 |
| `PolicyProvider` | **是**（速率限制） | 否 | 无（只读） | 0 |
| `FieldProvider` | 否 | **是** | Ring 2（Blue/Yellow）或 Ring 3（Red） | 2 |
| `HostInfoProvider` | 否 | **是**（仅 Ring 1） | Ring 1 | 2 |
| `Processor` | **是** | **是** | Ring 2（Blue/Yellow）或 Ring 3（Red） | 4 |
| `Formatter` | 否 | 否 | 无（只读） | 5 |
| `IOSink` | 否 | 否 | 无（只读） | 6 |
| `ConfigProvider` | N/A | N/A | N/A | 加载时 |
| `KeyProvider` | N/A | N/A | N/A | 加载时 |
| `SyscallBroker` | N/A | N/A | N/A | 代理 |

---

## ConfigProvider vs KeyProvider：何时使用每个

### ConfigProvider

`ConfigProvider` 扩展引擎加载配置的来源。引擎有一个内置 ConfigProvider，从 TOML 文件和环境变量读取。自定义 `ConfigProvider` 添加额外的来源。

**何时使用 ConfigProvider：**
- 配置存储在 HashiCorp Vault、AWS Secrets Manager 或 Azure Key Vault 中
- 配置由 etcd 或 Consul 管理（用于动态分布式配置）
- 配置存储在数据库中，变更需要实时重载
- 配置在到达引擎前需要转换（例如解密加密值）

**何时不使用 ConfigProvider：**
- 您只需要设置几个值——使用环境变量或 `dologger.toml` 文件
- 您需要签名密钥——改用 `KeyProvider`
- 您需要从磁盘文件读取——内置 ConfigProvider 已支持

### ConfigProvider VTable

```c
typedef struct {
    // 必需：加载配置并返回 TOML 字符串
    dologger_config_load_fn_t  load_config;

    // 可选：监控变更并通知引擎
    dologger_config_watch_fn_t watch_config;

    // 可选：应用前验证配置
    dologger_config_validate_fn_t validate;
} dologger_configprovider_vtable_t;

typedef dologger_error_t (*dologger_config_load_fn_t)(
    void                  *state,
    dologger_config_buf_t *out           // 引擎从此缓冲区读取 TOML
);

typedef dologger_error_t (*dologger_config_watch_fn_t)(
    void                          *state,
    dologger_config_change_cb_t    callback,  // 变更时调用此函数
    void                          *user_data
);
```

### 示例：最小 ConfigProvider（从 etcd 读取）

```c
// 伪代码——真实实现需要 etcd 客户端库
dologger_error_t etcd_config_load(void *state, dologger_config_buf_t *out) {
    EtcdState *s = (EtcdState *)state;
    char *etcd_value = etcd_get(s->etcd_client, "/dologger/config");
    if (!etcd_value) {
        return DO_LOG_ERR_CFG_MISSING;
    }
    strncpy(out->data, etcd_value, out->capacity);
    out->length = strlen(out->data);
    free(etcd_value);
    return DO_LOG_OK;
}
(伪代码 — 示意性 VTable 草图；v0.1.0 实际定义见 `core/include/dologger_core.h`（`dologger_key_provider_vtable_t`：`open`/`get_public_key`/`sign_detached`/`close`）)：
```

### KeyProvider

`KeyProvider` 管理用于签名审计记录的 Ed25519 密钥对。默认情况下，引擎在启动时生成一个临时密钥。自定义 `KeyProvider` 以持久、安全的密钥源替换它。

**何时使用 KeyProvider：**
- 签名密钥必须在引擎重启后持久存在（您需要验证旧的日志记录）
- 密钥通过 PKCS#11 存储在 HSM（Hardware Security Module）中
- 密钥由 AWS KMS、GCP KMS 或 Azure Key Vault 管理
- 您需要密钥轮换，并带有验证旧签名的宽限期
- 法规合规要求硬件支持的密钥保护（FIPS 140-2/3）

**何时不使用 KeyProvider：**
- 您在开发模式——内置临时密钥生成器即可
- 您在生产中但不使用 Ed25519 签名——无需密钥
- 您只需要存储密码——使用 ConfigProvider 或环境变量

### KeyProvider VTable

```c
typedef struct {
    dologger_key_sign_fn_t       sign;           // 必需：签名消息
    dologger_key_public_key_fn_t public_key;     // 必需：返回公钥
    dologger_key_rotate_fn_t     rotate;         // 可选：轮换密钥
} dologger_keyprovider_vtable_t;

typedef dologger_error_t (*dologger_key_sign_fn_t)(
    void             *key_state,
    const uint8_t    *message,
    size_t            message_len,
    dologger_sig_t   *signature_out
);

typedef dologger_error_t (*dologger_key_public_key_fn_t)(
    void             *key_state,
    uint8_t          *public_key_out,     // 32 字节
    size_t           *public_key_len
);

typedef dologger_error_t (*dologger_key_rotate_fn_t)(
    void             *key_state,
    uint8_t          *new_public_key_out, // 32 字节
    uint64_t         *rotation_timestamp
);
```

### ConfigProvider + KeyProvider：组合使用

某些后端同时服务于两个目的。例如 HashiCorp Vault 可以存储配置和签名密钥：

```toml
[plugins.vault-config]
type = "config_provider"
path = "/usr/lib/dologger/plugins/libvault_config.so"
# 从 Vault KV v2 获取 dologger.toml 内容

[plugins.vault-keys]
type = "key_provider"
path = "/usr/lib/dologger/plugins/libvault_keys.so"
# 从 Vault Transit 引擎获取 Ed25519 签名密钥
```

**关键区别**：ConfigProvider 为引擎提供*设置*（日志级别、缓冲区大小、接收器配置）。KeyProvider 为引擎提供*加密身份*（签名密钥）。这些是独立的关注点，也是独立的 VTable 类型。

---

## SyscallBroker 实现

### 目的

`SyscallBroker` 是沙箱（Yellow/Red）插件执行它们无法直接执行的受限制操作的机制。它们不自己调用 `open()`（seccomp-bpf 会以 `SECCOMP_RET_KILL_PROCESS` 阻止），而是调用 `SyscallBroker`，后者在引擎的 Blue 信任上下文中代表它们执行操作。

### 架构

```mermaid
sequenceDiagram
    participant Y as Yellow 插件
    participant B as SyscallBroker（Blue 信任）
    participant K as OS 内核

    Y->>B: dologger_syscall_broker(SYS_open, "/var/log/dologger/state", O_RDONLY)
    B->>K: open("/var/log/...", ...)
    K-->>B: fd = 42
    B-->>Y: 返回 42

    Y->>B: dologger_syscall_broker(SYS_read, fd=42, buf, len)
    B->>K: read(42, buf, len)
    K-->>B: bytes_read
    B-->>Y: 返回 bytes_read
(伪代码 — 示意性 VTable 草图；v0.1.0 实际定义见 `core/include/dologger_core.h`（`dologger_syscall_broker_vtable_t`：`syscall_io`）)：
```

### SyscallBroker VTable

```c
typedef struct {
    dologger_broker_dispatch_fn_t dispatch;
} dologger_syscallbroker_vtable_t;

typedef dologger_error_t (*dologger_broker_dispatch_fn_t)(
    void            *broker_state,
    uint32_t         syscall_number,      // 例如 SYS_open、SYS_read
    const void      *args,                // 平台特定的参数块
    size_t           args_len,
    dologger_broker_result_t *result      // 返回值 + errno
);
(伪代码 — 仅示意策略执行流程；`DO_LOG_TRUST_*`、`dologger_emit_sysmon` 等符号在 v0.1.0 中不存在)：
```

### 实现 SyscallBroker

生产 `SyscallBroker` 必须强制执行策略。代理具有 Blue 信任——它可以做任何事情。其工作是决定允许调用 Yellow/Red 插件做什么。

```c
dologger_error_t my_broker_dispatch(
    void *state, uint32_t sysno, const void *args,
    size_t args_len, dologger_broker_result_t *result)
{
    BrokerPolicy *policy = (BrokerPolicy *)state;

    // 1. 识别调用插件
    const char *caller = dologger_get_calling_plugin_name();
    PluginTrustColor color = dologger_get_plugin_trust_color(caller);

    // 2. 检查策略：此插件被允许做什么？
    switch (sysno) {
    case SYS_open:
    case SYS_openat:
        if (color == DO_LOG_TRUST_RED) {
            // Red 插件：完全没有文件访问
            result->ret = -1;
            result->errno_val = EACCES;
            dologger_emit_sysmon("SANDBOX_BROKER_DENIED",
                "plugin=%s syscall=open denied: Red trust", caller);
            return DO_LOG_OK;
        }
        // Yellow 插件：允许只读，根据白名单检查路径
        if (color == DO_LOG_TRUST_YELLOW) {
            const char *path = extract_open_path(args);
            if (!is_path_allowed(policy->yellow_path_allowlist, path)) {
                result->ret = -1;
                result->errno_val = EACCES;
                return DO_LOG_OK;
            }
        }
        break;

    case SYS_socket:
    case SYS_connect:
        // Yellow 和 Red：永远不允许网络
        if (color != DO_LOG_TRUST_BLUE) {
            result->ret = -1;
            result->errno_val = EACCES;
            return DO_LOG_OK;
        }
        break;

    case SYS_fork:
    case SYS_execve:
        // Red：永远不允许进程创建
        if (color == DO_LOG_TRUST_RED) {
            result->ret = -1;
            result->errno_val = EACCES;
            return DO_LOG_OK;
        }
        break;
    }

    // 3. 代表插件执行实际的系统调用
    long sys_ret = syscall(sysno, /* 解包参数 */);
    result->ret = sys_ret;
    result->errno_val = (sys_ret < 0) ? errno : 0;

    // 4. 审计：记录每个代理的系统调用
    dologger_audit_syscall_brokered(caller, sysno, result);

    return DO_LOG_OK;
}
(伪代码 — 示意性 VTable 草图；v0.1.0 实际定义见 `core/include/dologger_core.h`（`dologger_policy_provider_vtable_t`：仅 `evaluate`）)：
```

### SyscallBroker 的安全要求

1. **永不盲目转发**。如果代理收到不理解系统调用，必须拒绝（默认拒绝）。
2. **记录所有代理的调用**。Yellow 或 Red 插件的每个代理系统调用都必须被审计。
3. **Yellow 插件的路径白名单**。Yellow 插件只能访问在 `manifest.toml` 中声明的路径。
4. **速率限制**。恶意 Red 插件不应能够利用代理进行拒绝服务攻击。将代理系统调用速率限制为每个插件 1000 次/秒。
5. **超时**。代理系统调用应在 30 秒后超时，以防止代理线程无限期阻塞。

---

## 自定义 PolicyProvider 模式

### 内置策略

引擎在 PreFilter 阶段包含内置的速率限制和级别门控。自定义 `PolicyProvider` 替换或扩展这些功能。

### PolicyProvider VTable

```c
typedef struct {
    dologger_policy_evaluate_fn_t  evaluate;
    dologger_policy_update_fn_t    update;          // 可选
} dologger_policyprovider_vtable_t;

typedef dologger_error_t (*dologger_policy_evaluate_fn_t)(
    void                       *state,
    const dologger_record_t    *record,
    dologger_policy_result_t   *result
);

// result.action：
//   DO_LOG_POLICY_ALLOW   -- 记录通过预过滤器
//   DO_LOG_POLICY_DROP    -- 记录在过滤阶段前丢弃
//   DO_LOG_POLICY_DELAY   -- 记录被保留，稍后重新评估（背压）
//   DO_LOG_POLICY_THROTTLE -- 记录通过但以较低速率
(伪代码 — 令牌桶速率限制器示例，仅示意；`dologger_policy_result_t` 等在 v0.1.0 中不存在)：
```

### 模式 1：令牌桶速率限制器

经典的速率限制模式。为每个日志级别维护一个令牌桶。

```c
typedef struct {
    // 每个日志级别一个桶（TRACE 到 AUDIT）
    TokenBucket buckets[7];
    uint64_t     last_refill_ns;
} RateLimiterState;

typedef struct {
    double   tokens;              // 桶中当前令牌数
    double   max_tokens;          // 最大令牌数（突发容量）
    double   refill_rate;         // 每秒添加的令牌数
} TokenBucket;

dologger_error_t rate_limit_evaluate(
    void *state, const dologger_record_t *record,
    dologger_policy_result_t *result)
{
    RateLimiterState *s = (RateLimiterState *)state;
    uint8_t level = record->level;

    // 补充令牌
    refill_bucket(&s->buckets[level], s);

    // 检查是否有可用令牌
    if (s->buckets[level].tokens >= 1.0) {
        s->buckets[level].tokens -= 1.0;
        result->action = DO_LOG_POLICY_ALLOW;
    } else {
        result->action = DO_LOG_POLICY_DROP;
    }

    return DO_LOG_OK;
}
(伪代码 — 错误率断路器示例，仅示意)：
```

### 模式 2：按错误率的断路器

当 ERROR+FATAL 记录速率超过阈值时触发，表明应用程序故障风暴。

```c
typedef struct {
    uint64_t error_count;        // 当前窗口中的错误数
    uint64_t total_count;        // 当前窗口中的总记录数
    uint64_t window_start_ns;
    bool     circuit_open;
    double   error_rate_threshold; // 例如 0.5 = 50% 错误率打开断路器
} CircuitBreakerState;

dologger_error_t circuit_breaker_evaluate(
    void *state, const dologger_record_t *record,
    dologger_policy_result_t *result)
{
    CircuitBreakerState *s = (CircuitBreakerState *)state;

    // 如果断路器打开，丢弃一切（AUDIT 除外）
    if (s->circuit_open && record->level != DO_LOG_AUDIT) {
        result->action = DO_LOG_POLICY_DROP;
        return DO_LOG_OK;
    }

    // 在滑动窗口中跟踪错误率
    s->total_count++;
    if (record->level >= DO_LOG_ERROR) {
        s->error_count++;
    }

    // 检查错误率是否超过阈值
    if (s->total_count > 100) {
        double error_rate = (double)s->error_count / s->total_count;
        if (error_rate > s->error_rate_threshold) {
            s->circuit_open = true;
            dologger_emit_sysmon("POLICY_CIRCUIT_OPEN",
                "error_rate=%.2f threshold=%.2f", error_rate, s->error_rate_threshold);
        }
    }

    result->action = DO_LOG_POLICY_ALLOW;
    return DO_LOG_OK;
}
(伪代码 — 每租户配额示例，仅示意)：
```

### 模式 3：配额管理（每租户）

对于多租户部署，限制每个租户的日志记录，防止一个嘈杂租户消耗所有资源。

```c
typedef struct {
    // 映射：tenant_id -> quota
    QuotaMap quotas;
    uint64_t  default_quota_per_sec;
    uint64_t  window_duration_ns;
} QuotaManagerState;

dologger_error_t quota_evaluate(
    void *state, const dologger_record_t *record,
    dologger_policy_result_t *result)
{
    QuotaManagerState *s = (QuotaManagerState *)state;

    // 从记录中提取租户 ID（由 FieldProvider 设置）
    const char *tenant_id = dologger_record_get_field(record, "verified.tenant_id");
    if (!tenant_id) {
        tenant_id = "__default__";
    }

    QuotaEntry *quota = get_or_create_quota(s, tenant_id);
    if (quota->records_in_window >= quota->limit) {
        result->action = DO_LOG_POLICY_DROP;
        dologger_emit_sysmon("POLICY_QUOTA_EXCEEDED",
            "tenant=%s limit=%lu", tenant_id, quota->limit);
    } else {
        quota->records_in_window++;
        result->action = DO_LOG_POLICY_ALLOW;
    }

    return DO_LOG_OK;
}
```

---

## 插件依赖管理

### 声明依赖

依赖其他插件的插件在 `manifest.toml` 中声明：

```toml
[dependencies]
requires_fields = ["verified.user_id", "host.name"]
requires_plugins = [
    { name = "field-container", version = ">=1.0, <2.0" },
    { name = "json-formatter", version = ">=2.0, <3.0", optional = true }
]
```
（伪代码/示意 — 依赖解析步骤概述，非命令）：


### 依赖解析

引擎在启动时将依赖解析为有向无环图（DAG）：

```
1. 解析加载插件的所有 [dependencies] 节
2. 构建依赖图：节点 = 插件，边 A->B = "A 需要 B"
3. 拓扑排序确定加载顺序
4. 检测循环——如发现则拒绝（循环依赖攻击，参见安全白皮书）
5. 按拓扑顺序加载插件（依赖先加载）
6. 按拓扑顺序初始化
7. 按反向拓扑顺序关闭（依赖者先关闭）
(伪代码 — 依赖验证器示意（`for each` 为伪语法，非可编译 C）；v0.1.0 实际实现见 `core/src/plugin/dependency.rs`）：
```

### 加载顺序保证

**表 2：插件加载顺序规则**

| 规则 | 描述 |
|:-:|:-:|
| **依赖先加载** | 如果插件 A 依赖插件 B，B 在 A 之前加载和初始化。 |
| **管道阶段顺序** | 在一个阶段内，插件按声明顺序加载（配置文件顺序，然后字母顺序）。 |
| **阶段间依赖** | `Sink`（阶段 6）可能依赖于 `Formatter`（阶段 5）。Formatter 先加载。 |
| **跨类型依赖** | `Sink` 可能依赖于 `KeyProvider`。KeyProvider 先加载。 |
| **关闭是反向的** | 插件按反向依赖顺序关闭。依赖者在其依赖项之前关闭。 |

### 循环依赖检测

```c
// 引擎的依赖验证器（简化）
dologger_error_t validate_plugin_dag(PluginRegistry *registry) {
    for each plugin P in registry:
        if detect_cycle_from(P, visited_set):
            dologger_emit_sysmon("LICENSE_POLICY_VIOLATION",
                "检测到从插件 '%s' 开始的循环依赖", P->name);
            return DO_LOG_ERR_PLUGIN_LOAD;
    return DO_LOG_OK;
}
```

循环依赖被视为安全问题——它们可能被用于在管道中创建无限递归。如果检测到循环，引擎拒绝整个配置。

### 可选依赖

可选依赖允许插件在有或没有另一个插件的情况下运行：

```toml
[dependencies]
requires_plugins = [
    { name = "json-formatter", version = ">=2.0, <3.0", optional = true }
]
```
（示意 — 依赖冲突场景描述，非命令输出）：


当可选依赖不存在时：
- 引擎正常加载插件
- 插件可在运行时检查可用性：`dologger_is_plugin_loaded("json-formatter")`
- 插件必须优雅地处理依赖缺失的情况

### 依赖版本冲突

当两个插件要求第三方插件的冲突版本时：

```
插件 A 需要 json-formatter >= 1.0, < 2.0
插件 B 需要 json-formatter >= 2.0, < 3.0

结果：冲突
```
（示意 — 规划中的错误消息格式，非实际输出）：


引擎**拒绝配置**并显示清晰的错误消息：

```
[ERROR] 依赖冲突：
        插件 'http-sink' 需要 json-formatter >= 1.0, < 2.0
        插件 'audit-exporter' 需要 json-formatter >= 2.0, < 3.0
        每个插件只能加载一个版本。
(伪代码 — 规划中的可选导出；v0.1.0 尚无 `dologger_state_buf_t`，热重载序列化未实现)：
```

---

## 用于热重载的插件状态序列化

### 何时支持热重载

并非每个插件都需要状态序列化。在以下情况下考虑支持它：
- 您的插件积累了重建成本高的状态（例如 Processor 中训练好的 ML 模型）
- 您的插件是 `KeyProvider`，其关键材料必须在重载后持久存在
- 您的插件是 `FieldProvider`，缓存了昂贵的查找
- 您的插件是 `PolicyProvider`，具有正在运行的速率限制器状态

以下情况可跳过：
- 您的插件是无状态的（检查 `record.level` 的简单 Filter）
- `plugin_init()` 上重建状态成本低于 1 ms
- 您的插件状态包含不应序列化为明文的密钥

### 状态序列化 VTable 函数

```c
// 可选导出——如果不存在，插件在热重载时重新初始化

dologger_error_t plugin_state_serialize(dologger_state_buf_t *out) {
    // 将您的状态序列化到 out->data 中
    // out->capacity 是最大缓冲区大小
    // 设置 out->length 为实际写入的字节数
    // 如果容量不足则返回 DO_LOG_ERR_BUF_TOO_SMALL
}

dologger_error_t plugin_state_deserialize(const dologger_state_buf_t *in) {
    // 从 in->data 恢复您的状态
    // in->length 字节的序列化状态
}
(伪代码 — 序列化示例，仅示意；`dologger_state_buf_t` 不存在)：
```

### 序列化格式

您可以控制序列化格式。推荐方法：

| 方法 | 优点 | 缺点 | 示例 |
|:-:|:-:|:-:|:-:|
| **MessagePack** | 快速、紧凑、无模式 | 仅 C；需要库 | `msgpack_pack(&pk, state)` |
| **FlatBuffers** | 零拷贝反序列化 | 模式定义开销 | SIF 兼容格式 |
| **自定义二进制** | 最小开销，精确实用 | 维护负担；无工具支持 | `memcpy(out, &state, sizeof(state))`——仅适用于 POD 状态 |
| **JSON** | 人类可读，可调试 | 慢，输出大 | 仅适用于小型状态（< 1 KB） |

### 示例：序列化速率限制器状态

```c
// 状态结构
typedef struct {
    double tokens[7];           // 每个日志级别的令牌桶
    uint64_t last_refill_ns;
    uint64_t total_allowed;
    uint64_t total_dropped;
} RateLimiterState;

// 序列化
dologger_error_t plugin_state_serialize(dologger_state_buf_t *out) {
    size_t needed = sizeof(RateLimiterState);
    if (out->capacity < needed) {
        return DO_LOG_ERR_BUF_TOO_SMALL;
    }
    memcpy(out->data, &g_state, needed);
    out->length = needed;
    return DO_LOG_OK;
}

// 反序列化
dologger_error_t plugin_state_deserialize(const dologger_state_buf_t *in) {
    if (in->length != sizeof(RateLimiterState)) {
        return DO_LOG_ERR_INVALID_ARG;  // 状态格式不匹配
    }
    memcpy(&g_state, in->data, sizeof(RateLimiterState));
    return DO_LOG_OK;
}
(伪代码 — 状态版本管理示例，仅示意)：
```

### 状态版本管理

如果您的状态格式在插件版本之间发生变化，请包含版本头：

```c
typedef struct {
    uint32_t state_version;       // 状态布局变更时递增
    uint32_t state_size;          // 状态 blob 的总大小
    // ... 状态字段 ...
} VersionedState;

dologger_error_t plugin_state_deserialize(const dologger_state_buf_t *in) {
    VersionedState header;
    memcpy(&header, in->data, sizeof(header));

    if (header.state_version != MY_PLUGIN_STATE_VERSION) {
        // 版本不匹配——丢弃旧状态，全新初始化
        dologger_emit_sysmon("PLUGIN_STATE_MIGRATION",
            "plugin=%s old_version=%u new_version=%u -- 重新初始化",
            g_info.name, header.state_version, MY_PLUGIN_STATE_VERSION);
        return DO_LOG_OK;  // 从头初始化
    }

    memcpy(&g_state, in->data + sizeof(header), header.state_size);
    return DO_LOG_OK;
}
```
（伪代码/示意 — 热重载生命周期步骤，规划中）：


### 热重载生命周期

```
1. 引擎检测到新插件二进制（配置变更或 SIGHUP）
2. 在旧插件上调用 plugin_state_serialize()
3. 在旧插件上调用 plugin_shutdown()
4. dlclose(旧插件)
5. dlopen(新插件)
6. 在新插件上调用 plugin_init()
7. 在新插件上调用 plugin_state_deserialize(old_state_buf)
8. 引擎释放 old_state_buf
```

在此过程中，该插件阶段的管道**暂停**。记录在环形缓冲区中排队，新插件激活后处理。对于实现良好的插件，暂停持续时间通常 < 10 ms。

---

## 多阶段插件

### 概念

单个插件二进制文件可以通过导出多个 VTable 注册到多个管道阶段。这是需要既转换记录又格式化输出，或既过滤又提供字段的插件的高级模式。

### 在 manifest.toml 中声明多阶段

```toml
[plugin]
name = "pii-guardian"
version = "1.0.0"
plugin_type = "processor"        # 主类型
mount_phase = ["process", "filter"]  # 多个阶段
(伪代码 — 多阶段插件导出示意；v0.1.0 实际 VTable 定义见 `core/include/dologger_core.h`（无 `process_batch`/`filter_batch` 成员，且无 `dologger_vtable` 符号约定）)：
```

### 导出多个 VTable

```c
// 插件为每个阶段导出一个 VTable：

// 用于 "process" 阶段：
const dologger_processor_vtable_t dologger_processor_vtable = {
    .process       = pii_mask_process,
    .process_batch = pii_mask_process_batch,
};

// 用于 "filter" 阶段：
const dologger_filter_vtable_t dologger_filter_vtable = {
    .filter       = pii_detect_filter,
    .filter_batch = NULL,
};
```

引擎通过符号查找发现额外的 VTable。主 VTable（匹配 `plugin_type`）通过标准 `dologger_vtable` 符号找到。额外 VTable 通过类型特定符号找到，如 `dologger_processor_vtable`、`dologger_filter_vtable`。

### 何时使用多阶段插件

**适当的用例：**
- 一个 PII 处理器，也在 filter 阶段过滤包含未脱敏密钥的记录（纵深防御）
- 一个 JSON 格式化器，也提供 JSON 特定字段（FieldProvider + Formatter）
- 一个审计插件，既签名记录（Processor）又导出到 WORM 接收器（IOSink）

**不当的用例：**
- 将不相关的功能塞入一个插件（违反单一职责）
- 一个也写入文件的 Filter（Filter 应过滤，IOSink 应写入）
- 一个也签名记录的 ConfigProvider（ConfigProvider 应加载配置，KeyProvider 应签名）

### 多阶段执行顺序

当插件在多个阶段注册时，每个阶段实例在其各自的管道位置独立调用：

```mermaid
flowchart LR
    A["PreFilter"] --> B["Filter"] --> C["Field"] --> D["Process"] --> E["Format"] --> F["Sink"]
    X["pii-guardian（filter 阶段）— 先调用，检查和过滤"] -.-> B
    Y["pii-guardian（process 阶段）— 后调用，掩码/脱敏"] -.-> D
(伪代码 — 多阶段线程安全示例，仅示意；`dologger_filter_result_t` 不存在)：
```

插件的 `plugin_init()` 在管道开始前调用**一次**。相同的插件状态在所有阶段间共享。这意味着：

- **共享状态**：所有阶段共享相同的 `void *state` 指针。如果阶段并行执行，请注意并发访问。
- **共享生命周期**：插件加载/卸载一次，无论它注册了多少阶段。
- **共享沙箱**：信任颜色平等地应用于所有阶段。

### 多阶段状态的线程安全

如果您的多阶段插件的状态从不同管道阶段（可能在不同线程上执行）访问，您必须同步访问：

```c
typedef struct {
    pthread_mutex_t lock;
    SharedConfig    config;      // 读写：由 filter 更新，由 process 读取
} MultiPhaseState;

dologger_error_t pii_detect_filter(dologger_record_t *record,
                                    dologger_filter_result_t *result) {
    pthread_mutex_lock(&g_state->lock);
    // 检查 PII 检测是否启用（配置可能被更新）
    bool enabled = g_state->config.pii_detection_enabled;
    pthread_mutex_unlock(&g_state->lock);

    if (!enabled) {
        result->action = DO_LOG_FILTER_PASS;
        return DO_LOG_OK;
    }
    // ... PII 检测逻辑 ...
}
(伪代码 — 插件协作示意；v0.1.0 实际字段 API 为 `dologger_field_set(record, field, value, &err)` / `dologger_field_get(...)`，返回错误码而非指针)：
```

---

## 高级插件架构模式

### 模式 1：插件链（协作处理）

同一管道阶段的插件可以通过读取彼此的输出进行协作：

```c
// 插件 A（FieldProvider）写入一个字段
dologger_record_set_field(record, "verified.user_id", user_id);

// 插件 B（Processor）读取插件 A 的字段并丰富它
const char *user_id = dologger_record_get_field(record, "verified.user_id");
if (user_id) {
    UserProfile *profile = lookup_profile(user_id);
    dologger_record_set_field(record, "verified.user_email", profile->email);
}
```

协作插件应声明它们的相互依赖：

```toml
# plugin-b/manifest.toml
[dependencies]
requires_fields = ["verified.user_id"]    # 插件 A 提供此字段
(伪代码 — 插件委托模式示意；`dologger_get_plugin()` 等注册表 API 在 v0.1.0 中不存在)：
```

### 模式 2：插件委托（代理模式）

插件通过插件注册表将工作委托给另一个插件：

```c
// 一个 Formatter，将特定记录类型委托给另一个 Formatter
dologger_error_t delegating_format(const dologger_record_t *record,
                                    dologger_buf_t *output) {
    if (dologger_record_get_field(record, "ext.output_format") == "csv") {
        // 委托给 CSV 格式化器
        dologger_plugin_handle_t *csv_fmt = dologger_get_plugin("csv-formatter");
        return dologger_delegate_format(csv_fmt, record, output);
    }
    // 默认：格式化为 JSON
    return json_format(record, output);
}
(伪代码 — 插件状态缓存模式示意；`dologger_field_set_t` 等类型不存在)：
```

### 模式 3：插件状态作为缓存

插件可以使用其持久状态作为缓存，以避免重复的昂贵操作：

```c
// 将用户 ID 解析为显示名称的 FieldProvider
typedef struct {
    CacheEntry entries[MAX_CACHE_SIZE];
    size_t     entry_count;
    uint64_t   hits;
    uint64_t   misses;
} UserCacheState;

dologger_error_t user_resolver_provide_fields(
    void *state, dologger_record_t *record,
    dologger_field_set_t *fields)
{
    UserCacheState *cache = (UserCacheState *)state;
    const char *user_id = dologger_record_get_field(record, "verified.user_id");
    if (!user_id) return DO_LOG_OK;

    // 首先检查缓存
    for (size_t i = 0; i < cache->entry_count; i++) {
        if (strcmp(cache->entries[i].user_id, user_id) == 0) {
            cache->hits++;
            dologger_record_set_field(record,
                "verified.user_display_name", cache->entries[i].display_name);
            return DO_LOG_OK;
        }
    }

    // 缓存未命中——从数据库解析
    cache->misses++;
    char *display_name = db_lookup_display_name(user_id);
    if (display_name) {
        dologger_record_set_field(record,
            "verified.user_display_name", display_name);
        add_to_cache(cache, user_id, display_name);
    }
    return DO_LOG_OK;
}
(伪代码 — 插件内 sysmon 集成示意；`dologger_emit_sysmon` 在 v0.1.0 中不存在)：
```

缓存通过状态序列化在热重载后仍然存在，避免了冷启动的性能损失。

### 模式 4：可观测性插件（Sysmon 集成）

插件可以将自己的诊断信息发送到 sysmon 事件流：

```c
// 从插件内发出自定义指标
dologger_emit_sysmon("PLUGIN_METRIC",
    "plugin=%s cache_hits=%lu cache_misses=%lu hit_rate=%.2f",
    g_info.name, cache->hits, cache->misses,
    (double)cache->hits / (cache->hits + cache->misses));
(伪代码 — 优雅降级示例，仅示意；v0.1.0 实际签名 `int plugin_init(const void *config)`，`dologger_plugin_config_t` 不存在)：
```

自定义 sysmon 事件必须遵循命名约定 `PLUGIN_<EVENT_NAME>`（社区插件）或使用插件自己的命名空间。事件格式为单行 JSON（参见[运维手册](OperationsManual.md#sysmon-事件流)）。

### 模式 5：优雅降级

当插件的依赖项或外部资源不可用时，插件应优雅降级：

```c
dologger_error_t my_plugin_init(const dologger_plugin_config_t *config) {
    // 尝试连接到外部服务
    g_state->db_conn = db_connect(config->db_url);
    if (!g_state->db_conn) {
        // 降级：没有数据库丰富功能运行
        g_state->degraded = true;
        dologger_emit_sysmon("PLUGIN_DEGRADED",
            "plugin=%s reason=database_unavailable -- 以降级模式运行",
            g_info.name);
        return DO_LOG_OK;  // 初始化成功，即使是降级模式
    }
    g_state->degraded = false;
    return DO_LOG_OK;
}

dologger_error_t my_plugin_process(dologger_record_t *record) {
    if (g_state->degraded) {
        // 跳过丰富，原样传递记录
        return DO_LOG_OK;
    }
    // 带数据库丰富的正常处理
    return enrich_from_database(g_state->db_conn, record);
}
```

降级插件必须：
1. 进入降级模式时记录 `PLUGIN_DEGRADED` sysmon 事件
2. 继续原样传递记录（不丢弃它们）
3. 尝试定期重连（每 60 秒），成功时记录 `PLUGIN_RECOVERED`
4. 绝不会因外部资源缺失而崩溃或 panic
