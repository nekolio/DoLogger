# DoLogger 安全白皮书 (Security Whitepaper)

> 🌐 **语言 / Language**: [中文](SecurityWhitepaper.md) | [English: Security Whitepaper](../../en_US/guides/SecurityWhitepaper.md)

> **版本**: v0.1.0 | **最后更新**: 2026-08-12 | **目标受众**: 安全/合规工程师

## 目录

1. [安全模型概述](#安全模型概述)
2. [威胁模型 (STRIDE)](#威胁模型-stride)
3. [Record 字段权限环](#record-字段权限环)
4. [Ed25519 签名与 LSN 审计链](#ed25519-签名与-lsn-审计链)
5. [插件信任模型与沙箱隔离](#插件信任模型与沙箱隔离)
6. [配置安全与不可降级项](#配置安全与不可降级项)
7. [数据完整性保护](#数据完整性保护)
8. [供应链安全](#供应链安全)
9. [网络安全](#网络安全)
10. [合规性映射](#合规性映射)
11. [已知限制与待改进项](#已知限制与待改进项)

---

## 安全模型概述

### 核心安全原则

1. **深度防御**: 多层安全机制叠加（字段环 → 签名链 → 沙箱 → 配置不可降级）
2. **最小权限**: 插件按信任级别获得最小必要权限
3. **不可否认性**: 审计日志 Ed25519 签名 + LSN 区块链式验证
4. **完整性优先**: 安全 > Linux 锚点性能 > 热路径蓝色优先 > 生态安全

### 信任边界

**宿主应用架构（自上而下）：**

1. **插件层**（按信任级别分类）：
   - **Blue 插件**（全信任）
   - **Yellow 插件**（部分信任）
   - **Red 插件**（零信任）
2. **DoLogger 核心引擎**：
   - 环形缓冲区、管线调度
   - Ed25519 签名、LSN 审计链
   - 配置管理、诊断/监控
3. **Sink 输出层**：文件 / 网络 / 共享内存

---

## 威胁模型 (STRIDE)

对 DoLogger 设计实施 STRIDE 威胁建模：

| 威胁类别 | 威胁描述 | 缓解措施 | 状态 |
|:-:|:-:|:-:|:-:|
| **Spoofing** | 伪造日志记录/插件身份 | Ed25519 签名验证 + 插件证书 | ✅ |
| **Tampering** | 篡改已写入日志 | LSN 链 + prev_hash + WORM 文件 | ✅ |
| **Repudiation** | 否认日志写入 | Ed25519 不可否认签名 | ✅ |
| **Info Disclosure** | 敏感字段泄露 | Ring 0-3 权限控制 + 秘密检测 | ⚠️ 秘密检测 M4 |
| **DoS** | 日志洪泛致服务不可用 | 令牌桶 RateLimiter + 背压控制 | ✅ |
| **Elevation** | 红色插件提权 | 沙箱隔离 + seccomp/AppContainer | ✅ M3 框架 |

### 攻击向量分析

| 攻击向量 | 风险等级 | 缓解 |
|:-:|:-:|:-:|
| 红色插件尝试 fork() | CRITICAL | seccomp-bpf 拦截，返回 EPERM |
| 伪造审计签名 | CRITICAL | Ed25519 + HSM KeyProvider |
| 日志注入 (CRLF) | HIGH | 自动转义 HTML/控制字符 |
| 环形缓冲区溢出 | MEDIUM | 紧急 mmap 缓冲区 + 丢弃策略 |
| 配置文件篡改 | HIGH | config_lock + 不可降级项 |
| Sink 中间人攻击 | HIGH | require_tls + 证书固定 |

---

## Record 字段权限环

| Ring | 写入权限 | 读取权限 | 完整性保护 |
|:-:|:-:|:-:|:-:|
| Ring 0 | 核心引擎 | Sink 只读 API | Ed25519 签名 |
| Ring 1 | 核心 + HostInfoProvider | 所有插件只读 | Ed25519 签名 |
| Ring 2 | Blue/Yellow 插件 | 所有插件 | audit_tags + 签名扩展 |
| Ring 3 | 任何插件 | 任何插件 | CRC32C (硬件加速) |

### Ring 0 字段（不可变）

- `record.id`, `record.timestamp`, `record.signature`, `record.origin_lsn`
- 一旦写入，任何插件不得修改

### Ring 2 字段（已验证）

- `verified.*` 前缀命名空间
- 修改自动追加 `audit_tags`（plugin_id + version + timestamp；示意 — 实际字段为名为 `security.audit_tags` 的 `RecordString`）

### Ring 3 字段（不可信）

- `ext.*` 前缀命名空间
- CRC32C 完整性校验，不在 Ed25519 签名覆盖范围内

---

## Ed25519 签名与 LSN 审计链

### 签名覆盖范围

Ed25519 签名覆盖：
- 所有 Ring 0 字段
- 所有 Ring 1 字段 (包括 LSN 和 prev_hash)
- 可配置 `sign_ring2 = true` 扩展至 Ring 2 字段

签名不包括 Ring 3 字段（由 CRC32C 单独保护）。

### LSN 区块链式验证

（伪代码 — 链式验证算法示意，非可执行代码）：

```
Record(N):
  lsn = N
  prev_hash = SHA-256(Record(N-1).signature || Record(N-1).lsn)
  signature = Ed25519_Sign(Ring0_fields || Ring1_fields)

验证 Record(N):
  1. 验证 Ed25519 签名 ✓
  2. 计算 prev_hash 匹配 Record(N+1).prev_hash ✓
  3. LSN 单调递增 ✓
```

### WORM 文件保护

审计日志写入 WORM Sink：
- `fsync` (MEDIA durability) 每次写入后同步
- 关闭/滚动时设置只读权限 (chmod 0400 / FILE_ATTRIBUTE_READONLY)
- LSN 乱序重排窗口 (200ms) + 间隙标记

---

## 插件信任模型与沙箱隔离

### 三色分级

| 颜色 | 信任度 | 沙箱 | 文件 I/O | 网络 | 进程创建 |
|:-:|:-:|:-:|:-:|:-:|:-:|
| Blue | 完全 | 无 | ✅ | ✅ | ✅ |
| Yellow | 部分 | 受限 | ✅ | ❌ | ❌ |
| Red | 无 | 最大 | ❌ | ❌ | ❌ |

### seccomp-bpf 实现 (Linux)

黄色插件允许的系统调用类别：Memory, FileIO, Threading, Time, Signal, SystemInfo
红色插件允许的系统调用类别：Memory, Threading, Time (最少)

反例：红色插件尝试 `fork()` → seccomp-bpf 返回 `SECCOMP_RET_KILL_PROCESS` → 进程终止 + sysmon CRITICAL

### 已实现的安全测试 (15 项)

1. ✅ Ring 0 写入被 Blue 插件阻止
2. ✅ Ring 1 写入被不可信插件阻止
3. ✅ 签名篡改检测
4. ✅ LSN 字段篡改检测
5. ✅ LSN 链断裂检测
6. ✅ 审计背压铁律
7. ✅ 不可降级项绕过阻止
8. ✅ 背压丢弃策略正确性
9. ✅ 速率限制器阻断超额
10. ✅ 环形缓冲区并发安全
11. ✅ WORM 空隙检测 + 标记
12. ✅ 空隙标记超时处理
13. ✅ 循环依赖攻击阻止
14. ✅ Ring 3 ext 不在签名覆盖内
15. ✅ 所有不可降级项已定义

---

## 配置安全与不可降级项

### 不可降级项

以下 6 项在子域继承中只能收紧，不能放宽：

| 配置项 | 含义 | 安全影响 |
|:-:|:-:|:-:|
| `enable_signature` | Ed25519 签名 | 关闭后日志不可验证 |
| `escape_html` | HTML 转义 | 关闭后日志注入风险 |
| `worm_enabled` | WORM 强制 | 关闭后审计日志可修改 |
| `fsync_on_write` | 同步写入 | 关闭后崩溃不持久 |
| `require_tls` | 强制 TLS | 关闭后中间人攻击 |
| `sign_ring2` | Ring 2 签名 | 关闭后已验证字段无签名 |

### 合规模板 (M4)

预定义模板自动激活所有安全配置：
- `compliance/gdpr.toml` — 个人数据保护
- `compliance/hipaa.toml` — 医疗数据保护
- `compliance/pci-dss.toml` — 支付卡行业

（示意 — `config merge` 子命令与 `--compliance` 选项为规划中，v0.1.0 未提供；今天需手动合并 TOML 的 `[dologger]` 节，然后运行 `dologctl config validate --strict`）

---

## 数据完整性保护

### 多层次校验

| 层次 | 机制 | 性能开销 |
|:-:|:-:|:-:|
| Ring 3 字段 | CRC32C (SSE 4.2: ~0.5 cycles/B) | 极低 |
| Ring 0/1 字段 | Ed25519 签名 (~16.96μs) | 中等 |
| 审计链 | SHA-256 prev_hash | 低 |
| WORM 文件 | fsync + 只读锁定 | 中等 (I/O 绑定) |

### 篡改检测流程

1. 外部验证工具 `dologctl verify-log` 读取 WORM 文件（接受单个文件路径）
2. 逐条验证 Ed25519 签名
3. 验证 LSN 连续性 + prev_hash 链
4. 检测 LSN 间隙 → 生成间隙报告
5. 外部锚定验证（M4 规划；v0.1.0 的 `dologctl verify-anchor` 接受锚定 JSON 文件路径 + `--pubkey`）

---

## 供应链安全

### 插件签名验证

- Blue 插件: 必须由 DoLogger 团队 Ed25519 公钥签名
- 公钥通过 KeyProvider 获取，支持离线导入
- 插件版本绑定：`plugin_query` 返回 ABI 版本号

### 依赖许可证合规

- `deny.toml` 自动执行 SPDX 兼容性检查
- A/B 类许可证 (MIT/Apache/BSD/MPL) 允许
- C/D/E 类许可证 (GPL/SSPL/专有) 拒绝
- 红色插件可被例外放行（仅限社区分发）

### 第三方漏洞扫描

- `cargo audit` 扫描已知 CVE
- `cargo deny check advisories` 检查安全公告
- OSS-Fuzz 持续模糊测试 (M4)

---

## 网络安全

### Sink 安全传输

| Sink | 传输安全 | 认证 |
|:-:|:-:|:-:|
| Kafka | TLS + SASL/SCRAM-SHA-256 | 用户名/密码 + 证书 |
| Syslog | TCP/TLS (RFC 5425) | 可选 mTLS |
| Webhook | HTTPS | Bearer Token |
| OTel | HTTPS (OTLP/HTTP) | Bearer Token |

（注：v0.1.0 实际 Kafka sink 配置使用逗号分隔的 `brokers` 字符串 + `enable_tls`/`sasl_username`/`sasl_password`，见 `core/src/sink/kafka.rs`）

### 控制面安全

- M3: HTTP 本地监听 (127.0.0.1)
- M4: gRPC + mTLS + JWT

---

## 合规性映射

| 框架 | 要求 | DoLogger 实现 |
|:-:|:-:|:-:|
| GDPR Art. 30 | 处理活动记录 | WORM 审计日志 + 不可否认 |
| HIPAA §164.312(b) | 审计控制 | 审计域隔离 + Ed25519 |
| PCI DSS 10.2 | 自动化审计追踪 | LSN 链 + WORM |
| SOC 2 CC7.2 | 监控异常活动 | sysmon + 安全告警 |
| ISO 27001 A.12.4 | 日志记录与保护 | 签名 + 加密 + WORM |

---

## 已知限制与待改进项

### 当前限制 (M3)

1. **SIF 格式**: 使用简化二进制帧，完整 FlatBuffers SIF 待 M4
2. **进程隔离**: 沙箱提供策略框架，完整子进程隔离在 M4
3. **外部锚定**: M4 实现 S3/HTTP 锚定证明
4. **秘密检测**: M4 实现自动脱敏 Processor
5. **密钥轮换**: M4 实现 CRL + 多密钥并行验证

### 安全审计路线图

- [ ] 第三方安全审计 (M4-15)
- [ ] OSS-Fuzz 24h 无崩溃 (M4-12)
- [ ] 沙箱逃逸测试套件 × 3 平台 (M4-13)
- [ ] 渗透测试：签名绕过、LSN 注入、环形缓冲区竞态
- [ ] 形式化验证：LSN 链的加密强度分析
