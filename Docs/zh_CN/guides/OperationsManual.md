# DoLogger 运维手册 (Operations Manual)

> 🌐 **语言 / Language**: [中文](OperationsManual.md) | [English: Operations Manual](../../en_US/guides/OperationsManual.md)

> **版本**: v0.1.0 | **最后更新**: 2026-08-12 | **目标受众**: SRE/运维工程师

## 目录

1. [部署架构](#部署架构)
2. [配置管理](#配置管理)
3. [性能 Profile 选择](#性能-profile-选择)
4. [监控与告警](#监控与告警)
5. [控制面运维](#控制面运维)
6. [日志生命周期管理](#日志生命周期管理)
7. [备份与灾难恢复](#备份与灾难恢复)
8. [安全运维](#安全运维)
9. [故障处理流程](#故障处理流程)

---

## 部署架构

### 部署模式

| 模式 | 说明 | 适用场景 |
|:-:|:-:|:-:|
| **嵌入式** | 动态库直接链接到宿主进程 | 低延迟，单进程服务 |
| **边车 (Sidecar)** | 独立进程通过 sink_shm 接收日志 | 多语言微服务，运维隔离 |
| **守护进程** | 系统级日志收集服务 | 传统 syslog 替代 |

### 文件布局

（目录结构示意 — 非命令输出）：

```
Linux:
  /etc/dologger/default.toml        # 系统默认配置
  /usr/lib/dologger/plugins/        # 系统插件目录
  /var/log/dologger/                # 日志输出目录
  /var/lib/dologger/audit/          # WORM 审计日志
  /dev/shm/dologger_*.shm           # 共享内存 (sink_shm)

Windows:
  %PROGRAMDATA%\dologger\default.toml
  %PROGRAMFILES%\dologger\plugins\
  %LOCALAPPDATA%\dologger\logs\
```

---

## 配置管理

### 核心配置项

```toml
[dologger]
level = "INFO"
performance_profile = "prod-performance"    # dev/prod-performance/prod-audit/balanced
ring_buffer_size = 262144                   # 必须是 2 的幂
batch_size = 256
enable_signature = false                    # 生产审计环境设为 true
shutdown_policy = "graceful"
shutdown_timeout_ms = 5000
```

### 性能 Profile 速查

| Profile | 适用场景 | AUDIT 签名 | 背压策略 |
|:-:|:-:|:-:|:-:|
| `dev` | 开发调试 | 关闭 | 100ms 超时 → drop_newest |
| `prod-performance` | 高吞吐生产 | 可选 | 3s 超时 → below_warn |
| `prod-audit` | 审计合规 | 强制开启 | 3s 超时 → below_warn |
| `balanced` | 通用均衡 | 可选 | 2s 超时 → oldest |

### 环境变量

| 变量 | 说明 |
|:-:|:-:|
| `DO_LOG_LEVEL` | 覆盖日志级别 |
| `DO_LOG_BUF_SIZE` | 覆盖环形缓冲区大小 |
| `DO_LOG_PERF_PROFILE` | 覆盖性能 Profile |
| `DO_LOG_CONFIG_FILE` | 指定配置文件路径 |
| `DO_LOG_CONFIG_LOCK` | 禁止回退配置搜索（要求 `DO_LOG_CONFIG_FILE` 存在） |

### 配置热重载

修改配置文件后，引擎自动检测（轮询间隔 1s，防抖 500ms）——规划中的行为：

```bash
# 伪代码/示意 — ConfigWatcher（core/src/config/watcher.rs）在 v0.1.0 尚未接入 Engine::init，
# 引擎不会自动重载配置；需重启或（M3+）通过控制面触发
# sed -i 's/level = "INFO"/level = "DEBUG"/' dologger.toml
```

也可通过控制面 API 触发：

```bash
# 伪代码/示意 — 控制面（POST /reload）在 v0.1.0 尚未随引擎启动（M3+）
# curl -X POST http://127.0.0.1:9090/reload
```

---

## 监控与告警

### Sysmon 事件类型

| 事件码 | 严重等级 | 含义 | 处置 |
|:-:|:-:|:-:|:-:|
| `PIPELINE_BACKLOG` | WARN | 环形缓冲区占用 >50% | 检查消费者线程状态 |
| `SHM_DROP` | WARN | sink_shm 丢弃记录 | 检查消费者进程是否存活 |
| `SINK_CIRCUIT_OPEN` | ERROR | Sink 熔断器打开 | 检查下游服务 |
| `EMERGENCY_BUFFER` | WARN | 紧急缓冲区激活 | 立即排查背压原因 |
| `SANDBOX_VIOLATION` | CRITICAL | 沙箱违规 | 插件尝试禁用系统调用 |
| `SIGNATURE_FAILURE` | CRITICAL | 签名验证失败 | 日志可能被篡改 |

（注：v0.1.0 实际 sysmon 行格式为 `{"sysmon_version":"1.0","error_code":0,"category":"...","description":"...","timestamp_ms":...,"severity":1}`）

### 控制面状态查询

```bash
# 伪代码/示意 — 控制面在 v0.1.0 尚未随引擎启动（M3+）；
# 下方响应格式与 core/src/sys/control_plane.rs 的 /status 处理器一致
# curl http://127.0.0.1:9090/status
# {"status":"ok","level":"INFO","profile":"prod-performance","plugins":0,"signature_enabled":false}
```

### 关键指标

| 指标 | 基线 (P50) | 告警阈值 |
|:-:|:-:|:-:|
| 单条提交延迟 | < 102ns | > 500ns |
| 环形缓冲区占用 | < 70% | > 90% |
| 丢弃率 | 0% | > 0.1% |
| Sink 写入延迟 | < 1ms | > 100ms |
| 熔断器打开次数 | 0 | > 3/小时 |

---

## 控制面运维

### HTTP API 端点

| 方法 | 路径 | 功能 |
|:-:|:-:|:-:|
| GET | `/status` | 引擎状态 + 指标 |
| GET | `/health` | 存活检查（规划中 — v0.1.0 未实现） |
| POST | `/level` | 动态设置日志级别 |
| POST | `/reload` | 触发配置重载 |

### 示例

```bash
# 伪代码/示意 — 控制面（POST /level、POST /reload）在 v0.1.0 尚未随引擎启动（M3+）
# curl -X POST http://127.0.0.1:9090/level -d '{"level":"DEBUG"}'
# curl -X POST http://127.0.0.1:9090/level -d '{"level":"INFO"}'
# curl -X POST http://127.0.0.1:9090/reload
```

### 安全注意事项

- 控制面默认监听 127.0.0.1:9090（仅本地；规划中 — v0.1.0 未随引擎启动，M3+）
- M4 阶段支持 mTLS + JWT 认证
- 生产环境建议配合防火墙限制访问

---

## 日志生命周期管理

### 滚动策略

```toml
# （示意 — v0.1.0 的 FileSinkConfig 仅含：path、max_size（字节）、fsync_on_write、
# durability_level、buffer_size；按时间滚动、压缩与文件数保留均为规划中）
[sinks.file]
type = "sink_file"
max_size = "100MB"              # 按大小滚动
rotation_interval = "24h"       # 按时间滚动
compression = "zstd"            # gzip / zstd
```

### 保留策略

```toml
# （示意 — 保留策略键为规划中，v0.1.0 未解析）
[sinks.file]
retention_days = 90
retention_total_size = "10GB"
```

### 冷热分层

| 层级 | 存储 | 保留期 | 格式 |
|:-:|:-:|:-:|:-:|
| 热层 | 本地 NVMe | 0-7 天 | 当前写入（未压缩） |
| 温层 | 本地 HDD | 7-90 天 | 压缩归档 |
| 冷层 | S3 对象存储 | 90+ 天 | Parquet 列存 |

---

## 备份与灾难恢复

### WORM 审计日志备份

```bash
# 验证审计链完整性（verify-log 接受单个 SIF/WORM 文件路径）
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm

# 外部锚定（M4；verify-anchor 接受锚定 JSON 文件路径 + --pubkey）
dologctl verify-anchor anchors/2026-08.json --pubkey "$(cat pubkey.hex)"
```

### 紧急缓冲恢复

当环形缓冲区溢出时，记录自动溢出到紧急文件（位于系统临时目录 — 见 `core/src/buffer/emergency_buffer.rs`）。恢复正常后自动恢复：

（伪代码/示意 — 恢复流程，非命令）：

```
dologger_emergency_<pid>_<spill_id>.buf  →  引擎自动读取 → 注入主管线
```

---

## 安全运维

### 不可降级项

以下配置在子域中只能收紧，不能放宽：
- `enable_signature`
- `escape_html`
- `worm_enabled`
- `fsync_on_write`
- `require_tls`
- `sign_ring2`

### 密钥管理

- Ed25519 密钥对由 `KeyProvider` 管理
- 默认内置提供者生成临时密钥，永不落盘
- 生产环境建议配置外部 KeyProvider（HSM/SSM）

### 审计日志防篡改

审计链结构：每条记录通过 `prev_hash` 链接到下一条，所有记录均由 Ed25519 签名保护。

- `LSN(N)` -- `prev_hash` --> `LSN(N+1)` -- `prev_hash` --> `LSN(N+2)`
- 每条记录均通过 Ed25519 签名独立保护

---

## 故障处理流程

### 日志丢失排查

1. 检查 sysmon 输出 `stderr` 中的 `PIPELINE_DROP` / `SHM_DROP` 事件
2. 检查 `dologger_internal.log` 诊断日志
3. 确认 `enable_signature` 状态与期望一致
4. 检查 Sink 熔断器状态（`SINK_CIRCUIT_OPEN`）
5. 验证消费者进程存活（sink_shm）

### 性能下降排查

1. 运行 `cargo bench` 获取当前基线
2. 检查 `performance_profile` 配置是否正确
3. 检查 sysmon 中的 `PIPELINE_BACKLOG` 频率
4. 确认 ring_buffer_size 是否被意外覆盖
5. 检查磁盘 I/O 延迟（WORM Sink fsync）

### 沙箱违规

（伪代码/示意 — 沙箱违规事件格式示例）：

```
[SANDBOX_VIOLATION] plugin='untrusted-plugin' syscall='fork' action='KILL'
→ 插件已被沙箱终止，检查插件 trust color 和 capabilities 声明
```
