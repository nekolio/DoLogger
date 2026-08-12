# DoLogger 官方插件路线图

> 🌐 **语言 / Language**: [中文](OfficialPluginRoadmap.md) | [English: DoLogger Official Plugin Roadmap](../en_US/OfficialPluginRoadmap.md)

DoLogger 引擎附带一组精选的官方插件——类似于语言的标准库——覆盖最常见的日志记录、格式化、安全和可观测性需求。第三方插件在此基础上扩展以支持特定领域需求。

## 插件类型与管道位置

```
PreFilter(0) → Filter(1) → FieldProvider(2) → Assembly(3) → Processing(4) → Formatting(5) → Sink(6)
```

| 阶段 | 插件类型 | 官方数量 | 状态 |
|:-:|:-:|:-:|:-:|
| 0 | PolicyProvider | 2 | 内置（rate_limiter、drop_level） |
| 1 | Filter | 3 | 计划中 |
| 2 | FieldProvider | 3 | 1 个部分完成（host_info 内置） |
| 3 | Assembly | 0 | 仅核心（LSN + Ed25519 签名） |
| 4 | Processor | 3 | 1 个完成（secret_detector） |
| 5 | Formatter | 3 | 计划中 |
| 6 | IOSink | 11 | 全部内置 |
| — | KeyProvider | 2 | 计划中 |
| — | ConfigProvider | 0 | 延期（远程配置中心） |
| — | SyscallBroker | 0 | 延期（平台特定） |

---

## Tier 1 — 基础（v0.2.0 目标）

这些插件覆盖每个生产部署的基本需求。

### Filter：`filter_level`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Filter（1） |
| 信任 | Blue |
| 描述 | 丢弃低于可配置日志级别的记录（支持每域覆盖）。 |
| 配置 | `min_level: "INFO"`、`drop_below: true` |
| 理由 | 将日志级别过滤从核心引擎解耦；允许在不触及全局速率限制器的情况下进行每域规则设置。替代内置 `DropLevelPolicy` 用于特定域。 |

### Formatter：`fmt_json`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Formatting（5） |
| 信任 | Blue |
| 描述 | 将 Record 字段序列化为结构化 JSON，支持可配置的字段包含。 |
| 配置 | `pretty: false`、`include_ring3: false`、`timestamp_format: "rfc3339"` |
| 理由 | JSON 是日志聚合系统（ELK、Loki、Datadog）的通用交换格式。每次部署都需要此插件。 |

### Formatter：`fmt_text`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Formatting（5） |
| 信任 | Blue |
| 描述 | 人类可读的彩色文本输出，支持可配置的字段列（与 ConsoleSink 格式匹配但作为可加载插件）。 |
| 配置 | `color: true`、`show_thread: true`、`show_timestamp: true`、`timestamp_format: "elapsed"` |
| 理由 | 开发与调试。将 ConsoleSink 格式化逻辑移入可替换插件，以便其他接收器可重用它。 |

### FieldProvider：`field_container`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | FieldProvider（2） |
| 信任 | Blue |
| 描述 | 注入容器编排元数据：容器 ID（来自 `/proc/self/cgroup` 或 `$CONTAINER_ID`）、Pod 名称、命名空间、节点名称（来自 Kubernetes downward API）。 |
| 配置 | `source: "auto"`（自动检测 Docker/Kubernetes/podman） |
| 理由 | 到 2026 年，大多数生产工作负载运行在容器中。自动容器上下文注入是基本要求。 |

---

## Tier 2 — 生产（v0.3.0 目标）

这些插件解决安全、合规和运维需求。

### Processor：`proc_pii_mask`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Processing（4） |
| 信任 | Blue |
| 描述 | 在日志消息到达任何接收器之前掩码/替换其中的 PII 模式。 |
| 模式 | 邮件地址、信用卡号（Luhn 检查）、SSN（美国）、电话号码（E.164）、IBAN（欧盟）、IP 地址（可选） |
| 配置 | `mode: "mask"`（替换中间字符）或 `mode: "hash"`（SHA-256 伪匿名）、`custom_patterns: []` |
| 理由 | GDPR/CCPA/HIPAA 合规关卡。在格式化之前运行，确保掩码数据永不落盘或通过网络。补充现有的 `secret_detector`（处理 API 密钥/令牌）。 |

### Processor：`proc_field_enrich`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Processing（4） |
| 信任 | Blue |
| 描述 | 为用户定义的静态或计算键值字段添加到通过管道的每条记录。 |
| 配置 | `fields: { "datacenter": "us-east-1", "team": "payments" }`、`env_inherit: ["DEPLOY_VERSION", "REGION"]` |
| 理由 | 常见的运维需求——使用部署元数据标记记录，无需更改应用代码。 |

### FieldProvider：`field_cloud`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | FieldProvider（2） |
| 信任 | Blue |
| 描述 | 注入云提供商元数据：实例 ID、区域、可用区、账户 ID（AWS IMDSv2 / GCP metadata server / Azure IMDS）。 |
| 配置 | `provider: "auto"`、`timeout_ms: 100` |
| 理由 | 多云/混合云部署的关键需求。避免将云特定信息嵌入应用配置。 |

### Filter：`filter_sampling`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | Filter（1） |
| 信任 | Blue |
| 描述 | 概率记录采样——确定性地（按 trace_id 哈希）或随机地保留 1/N 条记录。 |
| 配置 | `rate: 0.01`（保留 1%）、`key: "trace_id"`（按字段确定性）、`min_level: "WARN"`（始终保留 WARN+） |
| 理由 | 高吞吐量系统无法承受存储每条 DEBUG/TRACE 记录。确定性采样保留追踪连续性。 |

### KeyProvider：`key_file`

| 属性 | 值 |
|:-:|:-:|
| 阶段 | KeyProvider（加载时） |
| 信任 | Blue |
| 描述 | 从文件系统读取 Ed25519 签名密钥，进行权限检查（必须为 0600，仅所有者可访问）。 |
| 配置 | `path: "/etc/dologger/signing_key"`、`require_owner: true` |
| 理由 | 生产部署不能将密钥嵌入配置 TOML。这是基础的外部密钥提供程序。 |

---

## Tier 3 — 扩展（v0.4.0+）

这些插件解决高级或专业用例。

| 插件 | 阶段 | 描述 | 优先级 |
|:-:|:-:|:-:|:-:|
| `fmt_csv` | Formatting | RFC 4180 CSV 输出，用于分析/仓库导入 | 中 |
| `filter_regex` | Filter | 丢弃匹配 `message` 或命名字段的正则表达式模式的记录 | 中 |
| `proc_geoip` | Processing | 从 MaxMind GeoLite2 数据库添加 `geo.country`、`geo.city` | 低 |
| `field_process` | FieldProvider | 进程统计：PID、父 PID、命令行、运行时间、RSS | 中 |
| `key_env` | KeyProvider | 从环境变量读取签名密钥（CI/CD、短期令牌） | 中 |
| `key_hsm` | KeyProvider | 到硬件安全模块的 PKCS#11 接口（YubiHSM、AWS CloudHSM） | 低 |
| `policy_quota` | PolicyProvider | 每域记录配额（每秒计数 + 字节预算） | 中 |

---

## 开发策略

### 阶段 1：脚手架（现在）
1. 创建 `plugins/official/` 目录结构
2. 每个插件获得一个 Cargo workspace 成员 crate
3. 标准化的 `Cargo.toml` 模板，使用 `license.workspace = true`
4. 每个插件的 `PluginManifest.toml` 包含元数据

### 阶段 2：Tier 1 实现（v0.2.0）
1. `fmt_json`——影响最大，为所有接收器启用结构化日志
2. `field_container`——通用容器元数据
3. `filter_level` + `fmt_text`——与内置行为对等，但以插件形式

### 阶段 3：Tier 2 实现（v0.3.0）
1. `proc_pii_mask`——合规关卡
2. `key_file`——生产密钥管理
3. `proc_field_enrich` + `field_cloud` + `filter_sampling`

### 阶段 4：Tier 3（v0.4.0+）
社区驱动，配官方参考实现。

### 插件 Crate 模板

```text
plugins/official/fmt_json/
├── Cargo.toml
├── PluginManifest.toml
└── src/
    └── lib.rs
```

每个官方插件：
- 导出 `plugin_query`、`plugin_init`、`plugin_shutdown` C ABI 符号
- 声明 `license.workspace = true`
- 包含插件索引的 `PluginManifest.toml`
- 具有覆盖 VTable 契约的单元测试
- 使用 DoLogger 根密钥签名（Blue 信任级别）

---

*最后更新：2026-08-12*
