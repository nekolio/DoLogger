# 错误码参考

> **DoLogger 错误码的权威唯一事实来源。** 常量定义于 [`core/src/error.rs`](../../../core/src/error.rs)，并镜像到 C ABI 枚举 `dologger_error_code_t`（[`core/include/dologger_core.h`](../../../core/include/dologger_core.h)）。不要相信任何其他位置（包括更早的设计文档）出现的错误码值——本表、`error.rs` 与 C 头文件才是唯一活源。

## 设计

每个错误码均为负 `i32`；`0`（`DO_LOG_OK`）表示成功。码值采用 nibble 分段：**高字节命名故障浮出的执行阶段**，沿一条记录在引擎中的旅程展开：

```
调用方 → 配置 → 插件 → 记录 → 摄入 → 签名 → 沙箱 → Sink → 远程 → 配额 → 合规 → 时钟 → SIF
0x01     0x02    0x03    0x04   0x05    0x06   0x07    0x08    0x09   0x0A    0x0B     0x0C   0x0D
```

排序原则（为什么，而不只是是什么）：

1. **语义与情景顺序** — 码按执行流程的阶段归类，运维读到码值即可判断"往哪个环节排查"。
2. **社区约定** — 负值表示错误、`0` 表示成功、类别段 + 段内顺序码（POSIX `errno`、Linux 内核 `-EXYZ`、Win32 `HRESULT` 的 facility/code）；一经分配即稳定，新码追加在段尾。
3. **命名** — `DO_LOG_ERR_<子系统>_<条件>`，`UPPER_SNAKE_CASE`；条件描述"出了什么故障"，而非"如何恢复"。

**插件自定义码**使用高位区间 `-0x80000000` 及以下（无符号即 `0x80000000`–`0xFFFFFFFF`）。核心对它们仅透传、不做语义解释，只封装进 `DologgerDomainEvent` 供 sysmon 记录。

## 0x01xx — 一般 / API

调用方边界检查：参数校验与引擎生命周期。这是任何宿主最先遭遇的一层错误。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `0` | `DO_LOG_OK` | 成功（无错误） |
| `-0x0101` | `DO_LOG_ERR_INVALID_ARG` | 传给 API 的参数无效 |
| `-0x0102` | `DO_LOG_ERR_NOT_SUPPORTED` | 当前平台/构建不支持的操作 |
| `-0x0103` | `DO_LOG_ERR_NOT_INITIALIZED` | 核心引擎未初始化 |
| `-0x0104` | `DO_LOG_ERR_ALREADY_INITIALIZED` | 核心引擎已初始化（重复 init） |
| `-0x0105` | `DO_LOG_ERR_OUT_OF_MEMORY` | 内存分配失败 |
| `-0x0106` | `DO_LOG_ERR_BUFFER_TOO_SMALL` | 调用方提供的缓冲区不足以容纳结果 |
| `-0x0107` | `DO_LOG_ERR_TIMEOUT` | 操作超时 |
| `-0x0108` | `DO_LOG_ERR_INTERNAL` | 通用内部错误 |
| `-0x0109` | `DO_LOG_ERR_INIT_FAILED` | 引擎初始化内部致命错误 |

## 0x02xx — 配置

配置文件加载、解析、校验、合并（域继承）与热重载。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0201` | `DO_LOG_ERR_CONFIG_NOT_FOUND` | 配置文件不存在 |
| `-0x0202` | `DO_LOG_ERR_CONFIG_PERMISSION` | 配置文件无读取权限 |
| `-0x0203` | `DO_LOG_ERR_CONFIG_PARSE` | 配置解析（TOML 语法）错误 |
| `-0x0204` | `DO_LOG_ERR_CONFIG_VALIDATION` | 配置语义校验失败 |
| `-0x0205` | `DO_LOG_ERR_CONFIG_MERGE` | 配置合并冲突（域继承） |
| `-0x0206` | `DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED` | 热重载失败；沿用旧配置 |
| `-0x0207` | `DO_LOG_ERR_CONFIG_HASH_MISMATCH` | 热重载配置哈希不匹配（校验途中文件被改） |
| `-0x0208` | `DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID` | 提交热重载的新配置校验失败 |
| `-0x0209` | `DO_LOG_ERR_CONFIG_RESTART_REQUIRED` | 其他字段已热加载，但受保护的编码变更需要重启 |

## 0x03xx — 插件

插件注册表与运行时：加载、manifest、ABI、依赖、状态与跨插件调用。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0301` | `DO_LOG_ERR_PLUGIN_NOT_FOUND` | 搜索路径中未找到插件 |
| `-0x0302` | `DO_LOG_ERR_PLUGIN_LOAD_FAILED` | 动态库加载失败（链接、符号缺失、平台不匹配） |
| `-0x0303` | `DO_LOG_ERR_PLUGIN_MANIFEST_INVALID` | 插件 manifest 校验失败 |
| `-0x0304` | `DO_LOG_ERR_PLUGIN_VERSION_MISMATCH` | 插件版本与核心 ABI 不兼容 |
| `-0x0305` | `DO_LOG_ERR_PLUGIN_ABI` | 插件 ABI 与核心不兼容 |
| `-0x0306` | `DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING` | 插件依赖未被满足 |
| `-0x0307` | `DO_LOG_ERR_PLUGIN_LOCK_MISMATCH` | 插件锁文件不匹配（确定性加载） |
| `-0x0308` | `DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID` | 插件签名验证失败 |
| `-0x0309` | `DO_LOG_ERR_MISSING_CAPABILITY` | 插件依赖的能力无任何提供者 |
| `-0x030A` | `DO_LOG_ERR_CIRCULAR_DEPENDENCY` | 插件图检测到循环依赖 |
| `-0x030B` | `DO_LOG_ERR_TOKEN_EXCEEDED_DEPTH` | 跨插件调用能力令牌链深度超限 |
| `-0x030C` | `DO_LOG_ERR_CALL_DEADLOCK` | 跨插件调用检测到死锁（循环等待） |
| `-0x030D` | `DO_LOG_ERR_STATE_FORMAT_UNSUPPORTED` | 插件状态格式版本不受支持 |
| `-0x030E` | `DO_LOG_ERR_STATE_ROLLBACK_REJECTED` | 插件状态迁移拒绝回滚（epoch 防回滚） |
| `-0x030F` | `DO_LOG_ERR_STATE_MIGRATE_FAILED` | 重载期间插件状态序列化/反序列化迁移失败 |

## 0x04xx — 记录 / 字段

记录不变量与字段访问。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0401` | `DO_LOG_ERR_RECORD_INVALID` | 记录处于非法状态 |
| `-0x0402` | `DO_LOG_ERR_FIELD_NOT_FOUND` | 记录中不存在该字段 |
| `-0x0403` | `DO_LOG_ERR_FIELD_PERMISSION_DENIED` | 字段访问被拒（权限环违规） |
| `-0x0404` | `DO_LOG_ERR_FIELD_TYPE_MISMATCH` | 字段类型不匹配 |
| `-0x0405` | `DO_LOG_ERR_FIELD_DEPENDENCY_NOT_MET` | 插件要求的字段未由前序管线阶段提供 |
| `-0x0406` | `DO_LOG_ERR_RECORD_INVALID_ENCODING` | 旧文本 ABI 输入不是有效 UTF-8 |

## 0x05xx — 缓冲 / 管线

摄入、背压与管线阶段。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0501` | `DO_LOG_ERR_BUFFER_FULL` | 环形缓冲已满且配置策略禁止无阻塞丢弃 |
| `-0x0502` | `DO_LOG_ERR_PIPELINE_STAGE` | 管线阶段错误 |
| `-0x0503` | `DO_LOG_ERR_AUDIT_QUEUE_FULL` | 审计域队列已满且策略不允许丢弃 |

## 0x06xx — 签名 / 审计链

密钥服务、签名、验签、LSN 链与审计域策略。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0601` | `DO_LOG_ERR_SIGN_FAILED` | 签名计算失败（Assembly 阶段） |
| `-0x0602` | `DO_LOG_ERR_VERIFY_FAILED` | 签名验证失败（可能为篡改） |
| `-0x0603` | `DO_LOG_ERR_LSN_CHAIN_BROKEN` | LSN 链断裂（检测到篡改） |
| `-0x0604` | `DO_LOG_ERR_LSN_GAP_DETECTED` | 检测到 LSN 跳跃（乱序窗口超限） |
| `-0x0605` | `DO_LOG_ERR_KEY_NOT_AVAILABLE` | 所需密钥不可用于签名 |
| `-0x0606` | `DO_LOG_ERR_KEY_PROVIDER_FAILED` | KeyProvider 插件打开/读取/签名操作失败 |
| `-0x0607` | `DO_LOG_ERR_AUDIT_DROP_FORBIDDEN` | AUDIT 域配置了丢弃策略 |
| `-0x0608` | `DO_LOG_ERR_AUDIT_CALLBACK_ONLY` | AUDIT 域仅配置了回调 Sink |
| `-0x0609` | `DO_LOG_ERR_AUDIT_NO_PERSISTENT_SINK` | AUDIT 域无持久化主 Sink |

## 0x07xx — 安全 / 沙箱

插件执行保护。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0701` | `DO_LOG_ERR_SANDBOX_INIT_FAILED` | 沙箱初始化失败 |
| `-0x0702` | `DO_LOG_ERR_SANDBOX_VIOLATION` | 沙箱策略违规（禁止的系统调用被拦截） |
| `-0x0703` | `DO_LOG_ERR_UNTRUSTED_PLUGIN` | 生产模式下尝试加载未签名（红色）插件 |

## 0x08xx — Sink / IO

本地与共享内存输出：File、WORM、Callback 与 `sink_shm`。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0801` | `DO_LOG_ERR_SINK_WRITE_FAILED` | Sink 写入失败（完整或部分写入） |
| `-0x0802` | `DO_LOG_ERR_SINK_CONNECTION_FAILED` | Sink 连接目标失败（文件、网络、broker） |
| `-0x0803` | `DO_LOG_ERR_SINK_CONNECTION_LOST` | 建立后 Sink 连接丢失 |
| `-0x0804` | `DO_LOG_ERR_SINK_FORMAT_INVALID` | Sink 输出格式配置无效或不支持 |
| `-0x0805` | `DO_LOG_ERR_SINK_CONFIG_INVALID` | Sink 配置被拒（如 `sink_shm` 的 `full_policy = "block"`） |
| `-0x0806` | `DO_LOG_ERR_SINK_NO_FALLBACK` | 该 Sink 不支持 fallback 链（如 `sink_shm`） |
| `-0x0807` | `DO_LOG_ERR_CALLBACK_TIMEOUT` | 回调 Sink 调用宿主的超时 |
| `-0x0808` | `DO_LOG_ERR_WORM_WRITE_FAILED` | WORM 写入失败（磁盘满、权限） |
| `-0x0809` | `DO_LOG_ERR_SHM_INIT_FAILED` | 共享内存创建/映射失败（权限、空间不足） |
| `-0x080A` | `DO_LOG_ERR_SHM_RING_FULL` | 共享内存环形缓冲已满（仅 block 策略下才会浮出） |
| `-0x080B` | `DO_LOG_ERR_AUDIT_SHM_FORBIDDEN` | `sink_shm` 配置给 AUDIT 域—被禁止 |

## 0x09xx — 网络 / 远程

远程 Sink（Kafka / Syslog / Webhook）：连接、TLS、SASL、熔断器。

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0901` | `DO_LOG_ERR_CIRCUIT_OPEN` | 远程 Sink 熔断器处于 OPEN；写入被拒 |
| `-0x0902` | `DO_LOG_ERR_TLS_FAILED` | TLS 握手 / 证书失败 |
| `-0x0903` | `DO_LOG_ERR_SASL_FAILED` | SASL 认证失败 |
| `-0x0904` | `DO_LOG_ERR_REMOTE_TIMEOUT` | 远程 Sink 操作超时（发送、批量 ack） |

## 0x0Axx — 资源 / 配额

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0A01` | `DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED` | 插件内存使用超过配置配额 |
| `-0x0A02` | `DO_LOG_ERR_QUOTA_CPU_EXCEEDED` | 插件 CPU 使用超过配置配额 |
| `-0x0A03` | `DO_LOG_ERR_RECURSION_DEPTH_EXCEEDED` | 日志自指涉递归深度超限 |

## 0x0Bxx — 合规

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0B01` | `DO_LOG_ERR_COMPLIANCE_VIOLATION` | 合规违规（模板 vs 手动配置，或不可降级项被放宽） |
| `-0x0B02` | `DO_LOG_ERR_AUDIT_DURABILITY_INSUFFICIENT` | AUDIT 域 Sink 持久化等级低于要求的 MEDIA |

## 0x0Cxx — 时钟 / 时间安全

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0C01` | `DO_LOG_ERR_TIME_BACKWARD` | 单调钟向后跳跃；审计域冻结 |

## 0x0Dxx — SIF / 序列化

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0D01` | `DO_LOG_ERR_SIF_INVALID` | SIF 帧损坏或未通过 FlatBuffer 结构校验 |
| `-0x0D02` | `DO_LOG_ERR_SIF_VERSION_UNSUPPORTED` | 插件声明的 SIF schema 版本不被核心支持 |

## 0x0Exx — 内部 / 致命

| 值 | 名称 | 说明 |
|:-:|:-:|:-:|
| `-0x0E01` | `DO_LOG_ERR_FATAL` | 引擎致命条件（插件被卸载；Sink 触发 `SINK_CIRCUIT_OPEN`） |

## 0x0Fxx — 保留

保留给未来核心扩展。插件自定义码必须使用高位区间 `-0x80000000` 及以下；核心码永不进入该空间。

## 相关链接

- Rust 常量与分类理由：`core/src/error.rs`
- C ABI 枚举：`core/include/dologger_core.h`
- 码的承载结构：`DologgerError` / `DologgerDomainEvent`（参见[宿主集成手册](HostIntegrationGuide.md)）
- 测试期望：[测试规范](TestingConvention.md)
