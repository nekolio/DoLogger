# DoLogger 运维手册 (Operations Manual)

> 🌐 **语言 / Language**: [中文](OperationsManual.md) | [English: Operations Manual](../../en_US/guides/OperationsManual.md)

> **版本**: v0.0.1 | **最后更新**: 2026-08-12 | **目标受众**: SRE/运维工程师

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

**表 1：部署模式对比**

| 模式 | 说明 | 适用场景 |
|:-:|:-:|:-:|
| **嵌入式** | 动态库直接链接到宿主进程。单一地址空间，延迟最低。 | 低延迟、单进程服务（如 Rust 微服务）。 |
| **边车 (Sidecar)** | 独立进程，通过 `sink_shm` 共享内存接收一个或多个宿主进程的日志。 | 需要在应用与日志组件之间做运维隔离的多语言微服务。 |
| **守护进程** | 系统级日志收集服务，通过本地套接字或共享内存接收日志。 | 传统 syslog 替代方案，适用于遗留应用。 |

### 如何选择部署模式

- **嵌入式**：当你能控制宿主进程二进制、且无法容忍 IPC 开销时使用。适用于 Rust 与 C 应用。
- **边车 (Sidecar)**：当宿主应用使用的语言没有原生 DoLogger 适配器，或需要在应用与日志基础设施之间做进程级故障隔离时使用。
- **守护进程**：用于跨多个应用的全系统日志收集，尤其在容器主机或裸金属服务器上。

### 文件布局

**Linux：**

```text
（布局示意）
/etc/dologger/
  default.toml                  # 系统全局默认配置
  conf.d/                       # 追加配置片段
    10-sinks.toml
    20-plugins.toml

/usr/lib/dologger/
  plugins/                      # 系统插件目录
  libdologger_core.so           # 核心引擎共享库

/var/log/dologger/              # 日志输出目录
  app.log                       # 当前日志文件
  app.2026-08-12.log.zst        # 已滚动并压缩

/var/lib/dologger/
  audit/                        # WORM 审计日志存储
    audit-000001.worm
    audit-000002.worm
  state/                        # 引擎状态（LSN 游标等）

/dev/shm/
  dologger_<name>.shm           # 共享内存段（sink_shm 模式）

/run/dologger/
  dologger.pid                  # PID 文件（守护进程模式）
  control.sock                  # Unix 域套接字（守护进程模式）
```

**Windows：**

```text
（布局示意）
%PROGRAMDATA%\dologger\
  default.toml
  conf.d\

%PROGRAMFILES%\dologger\
  plugins\
  dologger_core.dll

%LOCALAPPDATA%\dologger\
  logs\
  audit\
```

### 安装

> [!NOTE]
> 操作系统软件包尚未发布 — 以下命令为示意（打包规划中）。当前请从源码构建（`cargo build --release`）并手动复制产物。

**Linux（APT）：**
```bash
# （示意 — 软件包尚未发布）
sudo apt install dologger-core dologger-cli
```

**Linux（RPM）：**
```bash
# （示意 — 软件包尚未发布）
sudo dnf install dologger-core dologger-cli
```

**Linux（手动 tar 包）：**
```bash
# （示意 — 当前请从源码构建：cargo build --release）
tar xzf dologger-0.0.1-linux-x86_64.tar.gz
cd dologger-0.0.1-linux-x86_64
sudo cp libdologger_core.so /usr/lib/dologger/
sudo cp dologctl /usr/local/bin/
sudo mkdir -p /etc/dologger /var/log/dologger /var/lib/dologger/audit
sudo cp default.toml /etc/dologger/
```

**macOS（Homebrew）：**
```bash
# （示意 — formula 尚未发布）
brew install dologger/tap/dologger
```

---

## 配置管理

### 核心配置文件

```toml
# /etc/dologger/default.toml — 生产基线
# 校验：dologctl config validate --config /etc/dologger/default.toml --strict
# （该文件可通过宽松校验；在签名启用前 --strict 会失败）

[dologger]
level = "INFO"
performance_profile = "prod-performance"
ring_buffer_size = 65536        # 默认值；必须是 2 的幂
batch_size = 256
enable_audit = false           # 审计部署时设为 true
enable_signature = false        # 需要逐条签名时设为 true
# 以下五项域级不可降级项由 DomainManager 强制执行，
# 在 v0.0.1 中不读取自 [dologger] 段 — 此处仅为完整性列出：
escape_html = true              # 防止 CRLF / 日志注入
fsync_on_write = false          # 崩溃安全持久化设为 true
require_tls = true              # 强制所有网络 Sink 使用 TLS
sign_ring2 = false              # 设为 true 以签名受验证的扩展字段
shutdown_policy = "graceful"
shutdown_timeout_ms = 5000

# --- Sink 定义 ---
# （示意 — v0.0.1 的配置解析仅覆盖 [dologger] 键；Sink 段在代码中按管线接线，
# 参见 core/src/sink/）

# 旧 schema 中通过 enabled = false 禁用的 Sink 在此省略——某个 Sink 存在与否仅取决于
# 其表是否定义；不存在启用标志。

[sinks.file]
type = "file"
path = "/var/log/dologger/app.log"
max_size = 104857600
durability_level = "os_cache"

# --- 插件定义 ---

[plugins.json-formatter]
type = "formatter"
path = "/usr/lib/dologger/plugins/libjson_formatter.so"

[plugins.drop-debug]
type = "filter"
path = "/usr/lib/dologger/plugins/libdrop_debug.so"
```

### 配置优先级（从低到高）

1. **硬编码默认值** — 编译进 `libdologger_core`。
2. **系统配置** — `/etc/dologger/default.toml`
3. **追加配置片段** — `/etc/dologger/conf.d/*.toml`（按字母顺序合并）
4. **项目本地配置** — 当前工作目录下的 `dologger.toml`，向上逐级查找
5. **环境变量** — `DO_LOG_LEVEL`、`DO_LOG_CONFIG_FILE` 等
6. **运行时 API** — `dologger_config_load_from_string()`
7. **不可降级项** — 绝对硬限制（任何更低层都无法放宽）

### 环境变量

| 变量 | 覆盖项 | 示例 |
|:-:|:-:|:-:|
| `DO_LOG_LEVEL` | `level` | `DO_LOG_LEVEL=DEBUG` |
| `DO_LOG_BUF_SIZE` | `ring_buffer_size` | `DO_LOG_BUF_SIZE=524288` |
| `DO_LOG_PERF_PROFILE` | `performance_profile` | `DO_LOG_PERF_PROFILE=prod-audit` |
| `DO_LOG_CONFIG_FILE` | 配置文件路径 | `DO_LOG_CONFIG_FILE=/opt/myapp/dologger.toml` |
| `DO_LOG_PLUGIN_DIR` | 插件目录 | `DO_LOG_PLUGIN_DIR=/opt/myapp/plugins` |
| `DO_LOG_CONFIG_LOCK` | 禁止回退配置搜索（要求 `DO_LOG_CONFIG_FILE` 存在） | `DO_LOG_CONFIG_LOCK=1` |

### 配置校验

使用 `dologctl` 在应用配置前进行校验：

```bash
# 严格校验（强制不可降级的安全不变量）
dologctl config validate --config /etc/dologger/default.toml --strict

# （规划中 — v0.0.1 未发布 --compliance 参数）
# 按合规 Profile 校验
dologctl config validate \
    --config /etc/dologger/default.toml \
    --compliance gdpr

# （规划中 — v0.0.1 未发布 config show / config diff 子命令）
# 干跑：显示合并后的生效配置
dologctl config show --effective

# 对比两份配置
dologctl config diff /etc/dologger/default.toml /etc/dologger/staging.toml
```

### 热重载

DoLogger 支持在 `dologctl run` 运行期间热重载配置文件。它是**选择性启用**的：在配置文件中添加 `[watcher]` 段即可开启。默认情况下监视器处于关闭状态，因此除非加入 `[watcher]` 段，否则现有部署不受影响。

```toml
[dologger]
level = "INFO"

[watcher]
enabled = true
poll_interval_ms = 1000   # 仅 polling 的轮询间隔
debounce_ms = 500         # 最后一次变更后的稳定等待时间
backend = "auto"          # auto | polling | inotify | read-directory-changes | fsevents
```

- 当 `enabled` 为 `true` 时，`dologctl run` 会监视当前配置文件，并在检测到变更时调用 `Engine::reload_config`。
- 原生后端会自动检测：Linux 用 **inotify**，Windows 用 **ReadDirectoryChangesW**，macOS 用 polling（FSEvents 已延迟）。`backend` 可覆盖自动检测结果。
- 解析失败或无法构建/打开其 sink 的重载会被**拒绝**：先前配置保持生效，并记录一条 sysmon 错误（错误 `-0x0206` `CONFIG_RELOAD_FAILED` / `-0x0208` `CONFIG_RELOAD_INVALID`）。一次临时的错误编辑不会终止引擎。
- 活动 sink 通过共享 `SinkRef` 原子交换：在旧 sink 关闭前，进行中的写入会在同一次加锁期间完成。
- 插件变更仍需要重启引擎（重载不会在运行时重新加载插件）。
- 对重载值的完整安全级 / 不可降级校验已在规划中，但此版本的重载路径尚未强制实施。

```bash
# 无需重启修改日志级别
sed -i 's/level = "INFO"/level = "DEBUG"/' /etc/dologger/default.toml
# `dologctl run` 会检测到变更并自动重载。
```

### 合规模板

DoLogger 为受监管环境提供预构建的配置模板：

| 模板 | 路径 | 启用内容 |
|:-:|:-:|:-:|
| GDPR | `compliance/gdpr.toml` | 全部不可降级安全项 |
| HIPAA | `compliance/hipaa.toml` | 全部不可降级安全项 |
| PCI DSS | `compliance/pci-dss.toml` | 全部不可降级安全项 |

应用合规模板（示意 — `config merge` 为规划中；当前请自行合并 TOML 的 `[dologger]` 段，然后运行 `dologctl config validate --strict`）：

```bash
dologctl config merge \
    --base /etc/dologger/default.toml \
    --overlay compliance/gdpr.toml \
    --output /etc/dologger/gdpr-production.toml
```

---

## 性能 Profile 选择

**表 2：性能 Profile 参考**

| 属性 | `dev` | `balanced` | `prod-performance` | `prod-audit` |
|:-:|:-:|:-:|:-:|:-:|
| 阻塞超时 | 100 ms | 2000 ms | 3000 ms | 3000 ms |
| 丢弃策略 | `drop_newest` | `oldest` | `below_warn` | `below_warn` |
| Ed25519 签名 | 关闭 | 可选 | 可选 | **必选** |
| WORM 强制 | 关闭 | 可选 | 可选 | **必选** |
| 批大小 | 32 | 128 | 256 | 128 |
| 环形缓冲区大小 | 65536 | 131072 | 262144 | 262144 |

> [!NOTE]
> 阻塞超时与丢弃策略的值由 `core/src/pipeline/backpressure.rs` 强制执行。dev / prod-performance / prod-audit 的批大小与环形缓冲区大小和随附的配置模板一致；`balanced` 的值为示意（v0.0.1 未随附 `balanced` 模板）。

| Escape HTML | 可选 | 开启 | 开启 | **开启** |
| fsync on write | 关闭 | 关闭 | 可选 | **开启** |
| Require TLS | 关闭 | 仅告警 | 开启 | **开启** |

### 选择 Profile

```toml
# dologger.toml 中：
[dologger]
performance_profile = "prod-performance"
```

```bash
# 或通过环境变量：
export DO_LOG_PERF_PROFILE=prod-audit
```

你可以覆盖单个 Profile 值：

```toml
[dologger]
performance_profile = "prod-performance"
ring_buffer_size = 524288       # 覆盖 65536 默认值
```

覆盖值在 Profile 默认值之上合并。不可降级项不能通过覆盖放宽。

### 丢弃策略

| 策略 | 行为 |
|:-:|:-:|
| `drop_newest` | 环形缓冲区满时丢弃最新记录。防止阻塞生产者。 |
| `oldest` | 环形缓冲区满时丢弃最旧未处理记录。保持新鲜度。 |
| `below_warn` | 环形缓冲区满时仅丢弃 WARN 级别以下的记录。WARN 及以上保留。 |
| `block` | 环形缓冲区满时阻塞生产者直至有空间。风险：可能拖停宿主应用。 |

---

## 共享内存 Sink（sink_shm）

`sink_shm` 通过跨进程的共享内存环形缓冲区，将 KVF1 记录以零拷贝方式投递给外部消费进程，同时保留旧 SIF 的兼容读取。它与 `[sinks.*]` **分开接线**，不属于 sink 注册表。通过顶层 `[shm]` 表启用：

```toml
[shm]
path = "/dologger_default.shm"   # Unix 为 POSIX 名；Windows 为映射名
buffer_size_mb = 64              # 2 的幂，>= 8
slot_size_kb = 64                # 每槽最大容量，>= 64
full_policy = "drop_newest"      # drop_newest | drop_oldest
permissions = 0o660              # 仅 Unix
auto_cleanup = true              # 引擎关闭时 unlink 该区域
allowed_consumers = []           # 空 = 允许所有
```

`sink_shm` **非持久化**——`durability_level` 被强制为 `UNSAFE`。因此它在 AUDIT 模式（`enable_audit = true` / `prod-audit`）下被禁止，该模式需要持久化 WORM 存储；引擎以 `DO_LOG_ERR_AUDIT_SHM_FORBIDDEN` 拒绝此组合。

### 通过 CLI 启用

`dologctl run --shm <path>` 启用 `sink_shm` 并覆盖共享内存路径，同时保留配置中其他 `[shm]` 字段（或使用默认值）：

```bash
dologctl run --shm /dologger_default.shm
```

### 共享水位线语义

环形缓冲区头包含两个序号：

| 字段 | 持有者 | 含义 |
|:-:|:-:|-|
| `producer_seq` | DoLogger（生产者） | 下一个写入槽位；每接受一条记录递增 |
| `consumer_seq` | 消费者（共享） | 回收水位线——其下槽位可安全覆盖 |

`consumer_seq` 是**单一共享水位线**，由消费者通过 `compare_exchange` 协作推进。只有一个水位线，因此慢消费者可能触发 `drop_oldest`/`drop_newest`——DoLogger 在共享内存路径上从不阻塞生产者。仍在读取已被回收槽位的消费者必须预期 `overwritten_count` 增加并重新读取区域。

### 检查区域

```bash
dologctl shm status /dologger_default.shm          # 人类可读
dologctl shm status /dologger_default.shm --output json
dologctl shm clear /dologger_default.shm           # 需生产者 DEAD 或 --force
```

`dologctl shm status` 与 `clear` 通过核心 `dologger_core::sink::shm::read_status` API 读取头部——头部布局的唯一事实源（消费者 ABI 见 `core/include/dologger_shm.h`）。

---

## 监控与告警

### Sysmon 事件流

系统监视器（`sysmon`）默认向 `stderr` 输出结构化事件。每个事件是一行 JSON：

```json
（示意 — 实际 sysmon 行格式为：
 {"sysmon_version":"1.0","error_code":0,"category":"engine","description":"...","timestamp_ms":...,"severity":1}）
{"ts":"2026-08-12T14:30:00.123Z","level":"WARN","event":"PIPELINE_BACKLOG","pct":72,"buf_name":"main"}
```

**表 3：Sysmon 事件类型**

| 事件 | 严重等级 | 含义 | 立即处置 |
|:-:|:-:|:-:|:-:|
| `PIPELINE_BACKLOG` | WARN | 环形缓冲区占用超过 50% | 检查消费者线程健康；考虑增大 `ring_buffer_size` |
| `PIPELINE_DROP` | WARN | 缓冲区满导致记录被丢弃 | 增大容量或切换到 `prod-performance` Profile |
| `SHM_DROP` | WARN | 共享内存 Sink 丢弃记录 | 确认消费者进程存活并正常消费 |
| `SINK_CIRCUIT_OPEN` | ERROR | Sink 熔断器跳闸 | 检查下游服务健康；熔断器 30 秒后自动复位 |
| `SINK_CIRCUIT_CLOSED` | INFO | Sink 熔断器复位 | 下游已恢复 |
| `EMERGENCY_BUFFER` | WARN | 紧急溢出缓冲区激活 | 环形缓冲区溢出；记录正在溢出到磁盘 |
| `EMERGENCY_RECOVERED` | INFO | 溢出缓冲区已回灌管线 | 系统已从溢出中恢复 |
| `SANDBOX_VIOLATION` | CRITICAL | 插件尝试调用不允许的系统调用 | 插件线程已终止；复核插件信任色 |
| `SIGNATURE_FAILURE` | CRITICAL | Ed25519 签名验证失败 | 日志记录可能被篡改；启动事件响应 |
| `LSN_GAP_DETECTED` | ERROR | 发现 LSN 序列缺口 | 记录可能缺失；运行 `dologctl verify-log` |
| `CONFIG_RELOAD` | INFO | 配置已重载 | 验证 — 确认预期变更已生效 |
| `CONFIG_RELOAD_DENIED` | WARN | 配置重载被拒绝 | 尝试放宽某个不可降级项 |
| `LICENSE_POLICY_VIOLATION` | ERROR | 插件因许可证不兼容被拒绝 | 复核插件 SPDX 标识符 |

### 控制面状态端点

```bash
# 伪代码/示意 — v0.0.1 中控制面尚未随引擎启动；
# 下方响应格式与 core/src/sys/control_plane.rs 的 /status 处理器一致
# curl -s http://127.0.0.1:9090/status | jq .
```

```json
（示意 — /status 处理器的实际响应更小：
 {"status":"ok","level":"INFO","profile":"prod-performance","plugins":0,"signature_enabled":false}；
 下方丰富的指标体为规划中）
{
  "status": "ok",
  "uptime_seconds": 86412,
  "level": "INFO",
  "profile": "prod-performance",
  "plugins_loaded": 3,
  "plugins_failed": 0,
  "signature_enabled": false,
  "ring_buffer": {
    "capacity": 262144,
    "used": 8192,
    "pct_used": 3.1,
    "drops_total": 0,
    "emergency_spills": 0
  },
  "sinks": {
    "file": {"status": "healthy", "bytes_written": 1073741824},
    "kafka": {"status": "healthy", "messages_sent": 5000000}
  },
  "pipeline": {
    "records_processed": 10000000,
    "records_dropped": 0,
    "avg_latency_us": 82
  }
}
```

### 关键指标与告警阈值

**表 4：运维指标**

| 指标 | 基线 (P50) | 告警阈值 | 严重阈值 | 来源 |
|:-:|:-:|:-:|:-:|:-:|
| 记录提交延迟 | < 102 ns | > 500 ns | > 2 us | `/status` |
| 环形缓冲区占用 | < 70% | > 80% | > 90% | `/status` |
| 丢弃率 | 0% | > 0.01% | > 0.1% | `/status` |
| Sink 写入延迟 | < 1 ms | > 10 ms | > 100 ms | `/status` |
| 熔断器每小时跳闸次数 | 0 | > 1 | > 3 | sysmon `SINK_CIRCUIT_OPEN` 计数 |
| 签名失败 | 0 | > 0 | > 0（任意） | sysmon `SIGNATURE_FAILURE` |
| 沙箱违规 | 0 | > 0 | > 0（任意） | sysmon `SANDBOX_VIOLATION` |
| LSN 缺口 | 0 | > 0 | > 0（任意） | sysmon `LSN_GAP_DETECTED` |

### Prometheus 集成（规划中）

```yaml
# prometheus.yml 抓取配置（示意 — 规划中）
scrape_configs:
  - job_name: 'dologger'
    static_configs:
      - targets: ['localhost:9090']
    metrics_path: '/metrics'
```

### 基于日志的告警

将 sysmon 事件接入你的集中式日志平台（Elasticsearch、Loki、Splunk）并配置告警：

```text
（示意告警规则草图，非字面查询语法）
# Elasticsearch 告警查询示例
event: "SIGNATURE_FAILURE" OR event: "SANDBOX_VIOLATION"
  → PagerDuty：critical（严重）
  → Slack：#incident-response

event: "SINK_CIRCUIT_OPEN"
  → PagerDuty：warning（5 分钟后升级为 critical）

event: "PIPELINE_BACKLOG" AND pct > 90
  → Slack：#sre
```

---

## 控制面运维

### HTTP API 端点

**表 5：控制面 API（规划中）** — v0.0.1 中这些端点均未随引擎启动。

| 方法 | 路径 | 认证 | 说明 |
|:-:|:-:|:-:|:-:|
| GET | `/status` | 无 | 引擎状态与指标（见上文） |
| GET | `/health` | 无 | 存活检查（规划中 — v0.0.1 未实现） |
| POST | `/level` | 无 | 动态设置日志级别 |
| POST | `/reload` | 无 | 触发配置重载 |

### 运行时修改日志级别

```bash
# 伪代码/示意 — v0.0.1 中控制面尚未启动
# 调试时临时提高日志详细程度
# curl -X POST http://127.0.0.1:9090/level \
#   -H "Content-Type: application/json" \
#   -d '{"level": "DEBUG"}'

# 恢复生产日志级别
# curl -X POST http://127.0.0.1:9090/level \
#   -H "Content-Type: application/json" \
#   -d '{"level": "INFO"}'

# 查询当前级别
# curl -s http://127.0.0.1:9090/status | jq .level
```

### 触发配置重载

启用 `[watcher]` 时（见上文热重载小节），热重载会自动触发。控制面重载端点仍为规划中：

```bash
# 规划中 — 此版本控制面尚未启动
# 直接重载（语法合法即应用变更）
# curl -X POST http://127.0.0.1:9090/reload
```

### 安全注意事项

- 控制面默认监听 `127.0.0.1:9090`（规划中 — v0.0.1 中控制面未启动）— 仅同主机进程可达。
- mTLS + JWT 认证（远程访问）为规划中。
- 生产部署应使用主机级防火墙规则限制对控制面端口的访问：
  ```bash
  # iptables：仅允许本机访问
  sudo iptables -A INPUT -p tcp --dport 9090 -s 127.0.0.1 -j ACCEPT
  sudo iptables -A INPUT -p tcp --dport 9090 -j DROP
  ```
- `DO_LOG_CONFIG_LOCK=1` 环境变量可禁止回退配置搜索（配置的 `DO_LOG_CONFIG_FILE` 必须存在）。

---

## 日志生命周期管理

### 滚动策略

文件 Sink 同时支持按大小与按时间滚动：

```toml
# （示意 — v0.0.1 的 FileSinkConfig 包含：path、max_size（字节）、
# fsync_on_write、durability_level、buffer_size；按时间滚动、
# 压缩与文件数保留均为规划中）
[sinks.file]
type = "file"
path = "/var/log/dologger/app.log"
max_size = 104857600            # 文件超过 100 MB 时滚动
rotation_interval = "24h"       # 无论大小，每日零点滚动
max_rotated_files = 90          # 最多保留 90 个滚动文件
compression = "zstd"            # 压缩滚动文件（gzip | zstd | none）
```

滚动不阻塞日志提交。新文件打开的同时，旧文件在后台线程中关闭并（可选地）压缩。

### 保留策略

```toml
# （示意 — 保留策略键为规划中，v0.0.1 未解析）
[sinks.file]
retention_days = 90             # 删除超过 90 天的文件
retention_total_size = "10GB"   # 总量超过 10 GB 时删除最旧文件
```

保留检查在每次滚动时执行一次。若同时设置 `retention_days` 与 `retention_total_size`，满足**任一**条件即删除文件。

### 冷热分层

**表 6：存储分层策略**

| 层级 | 存储 | 保留期 | 格式 | 访问模式 |
|:-:|:-:|:-:|:-:|:-:|
| 热层 | 本地 NVMe/SSD | 0–7 天 | 未压缩 | `tail -f`、`grep`、实时看板 |
| 温层 | 本地 HDD | 7–90 天 | Zstd 压缩 | `dologctl query`、事件调查 |
| 冷层 | S3 / GCS / ABS | 90 天以上 | Parquet 列存 | 合规审计、长期分析 |

**自动分层（规划中）：**

```toml
# （规划中 — 示意 schema）
[sinks.file.tiering]
enabled = true
warm_storage = "/data/dologger/warm/"
cold_storage = "s3://my-audit-logs/cold/"
promote_to_warm_after = "7d"
archive_to_cold_after = "90d"
```

### WORM 审计日志处理

WORM（一次性写入、多次读取）审计日志单独存储，需要特别处理：

```bash
# 列出 WORM 段
ls -la /var/lib/dologger/audit/
# -r-------- 1 root root 104857600 Aug 12 00:00 audit-000001.worm
# -r-------- 1 root root  52428800 Aug 12 12:00 audit-000002.worm

# 校验单个 WORM 文件的链完整性（verify-log 接受文件路径）
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm

# 或报告目录下所有 *.worm 文件的 LSN 连续性
dologctl recovery-report /var/lib/dologger/audit/

# （规划中 — v0.0.1 未发布 dologctl audit export）
# 导出审计记录为 JSON 以便分析
dologctl audit export \
    --path /var/lib/dologger/audit/ \
    --from "2026-08-01" \
    --to   "2026-08-12" \
    --format json \
    --output audit-august-2026.json
```

---

## 备份与灾难恢复

### WORM 审计日志备份

```bash
# 备份前先校验完整性
dologctl recovery-report /var/lib/dologger/audit/

# 校验通过后，rsync 到备份位置
rsync -avz \
    /var/lib/dologger/audit/ \
    backup-server:/backups/dologger/$(hostname)/audit/

# （规划中 — v0.0.1 未发布 --latest-lsn-only 参数与锚定发布）
# 记录最后校验通过的 LSN 以便外部锚定
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm --latest-lsn-only
# {"latest_lsn": 100042,"root_hash": "a3f8b2c1..."}

# 将根哈希发布到外部见证（S3 对象元数据、区块链锚点等）
# 规划中：dologctl anchor publish --s3-bucket audit-anchors --root-hash "a3f8b2c1..."
```

### 紧急缓冲恢复

当环形缓冲区溢出时，记录溢出到磁盘上的紧急文件（位于系统临时目录的 `dologger/` 子目录 — 见 `core/src/buffer/emergency_buffer.rs`）：

```text
dologger_emergency_<pid>_<spill_id>.buf
```

恢复时（当环形缓冲区有可用空间）：

1. 引擎在启动时检测到紧急文件。
2. 从文件读取记录并注入主管线。
3. 重放成功后删除紧急文件。
4. 发出 `EMERGENCY_RECOVERED` sysmon 事件。

**手动恢复：**

```bash
# 检查遗留的紧急文件（系统临时目录的 dologger/ 子目录）
ls -la /tmp/dologger/dologger_emergency_*.buf

# 若引擎正在运行且文件持续存在，检查引擎状态
# （伪代码/示意 — v0.0.1 中控制面尚未启动；
# 规划中的 /status 响应尚无 ring_buffer 对象）
# curl http://127.0.0.1:9090/status

# 若引擎已崩溃，紧急文件将在下次启动时重放
```

### 配置备份

```bash
# 备份当前生效配置
cp /etc/dologger/default.toml /backups/dologger/config-$(date +%Y%m%d).toml

# （规划中 — v0.0.1 未发布 config show 子命令）
# 使用 dologctl 备份（包含合并后的生效配置）
dologctl config show --effective > /backups/dologger/effective-$(date +%Y%m%d).toml
```

### 恢复时间目标

**表 7：RTO/RPO 参考**

| 场景 | RPO | RTO | 处置流程 |
|:-:|:-:|:-:|:-:|
| 磁盘故障（非 WORM） | 最近一次滚动（最长 24h） | 重新置备磁盘 + 从备份恢复所需时间 | 从备份服务器恢复 |
| 磁盘故障（WORM） | 最近一次 fsync（0 条记录丢失） | 重新置备磁盘所需时间 | WORM 文件每次写入均 fsync |
| 进程崩溃 | 紧急缓冲区重放 | < 10 秒 | 引擎自动重启；紧急缓冲区重放 |
| 意外删除日志（非 WORM） | 最近一次备份 | 从备份恢复所需时间 | 从备份服务器恢复 |
| 意外删除日志（WORM） | 不适用 — 文件只读 | 不适用 | 未经操作系统层面干预，WORM 文件无法被删除 |

---

## 安全运维

### 不可降级项

以下 5 个配置项跨配置层只能**收紧**（朝更安全的方向调整），永远不能放宽：

**表 8：不可降级安全项**

| 配置项 | 放宽方式 | 放宽后的安全影响 |
|:-:|:-:|:-:|
| `enable_signature` | `true` → `false` | 日志不再可加密验证，失去不可否认性。 |
| `escape_html` | `true` → `false` | 可能出现日志注入（CRLF）攻击。 |
| `fsync_on_write` | `true` → `false` | 崩溃可能丢失在途审计记录；持久化保证失效。 |
| `require_tls` | `true` → `false` | 网络 Sink 接受未加密连接；暴露中间人攻击面。 |
| `sign_ring2` | `true` → `false` | 受验证的扩展字段失去加密绑定。 |

任何放宽这些项的尝试都会触发一条 `CONFIG_RELOAD_DENIED` sysmon 事件，且变更被拒绝。

### 密钥管理

用于日志签名的 Ed25519 密钥对由 `KeyProvider` 插件管理：

- **默认**：内置临时密钥生成器。密钥在启动时于内存中生成一次，**永不落盘**。重启引擎会生成新密钥，使旧签名失效。
- **生产**：部署由 HSM（硬件安全模块）、AWS KMS 或 HashiCorp Vault 支撑的外部 `KeyProvider`。这样可保证密钥跨重启持久化，并提供基于硬件的密钥保护。

```toml
# （示意 — v0.0.1 未解析插件配置段）
[plugins.hsm-key-provider]
type = "key_provider"
path = "/usr/lib/dologger/plugins/libhsm_keyprovider.so"

[plugins.hsm-key-provider.config]
pkcs11_module = "/usr/lib/softhsm/libsofthsm2.so"
slot_id = 0
key_label = "dologger-signing-key"
```

### 审计链路防篡改检测

LSN（日志序列号）+ content_hash 链提供密码学层面的篡改证据（伪代码 — 示意）：

```
Record(N):
  lsn          = N
  content_hash = SHA-256( canonical_serialization(Record(N)) )
  prev_hash    = SHA-256( Record(N-1).content_hash || Record(N-1).lsn )
  # 侧车 audit.log.sig: sig(N) = Ed25519_Sign(TPM 密钥, SHA-256(lsn || content_hash || prev_hash))

Record(N+1):
  lsn          = N+1
  content_hash = SHA-256( canonical_serialization(Record(N+1)) )
  prev_hash    = SHA-256( Record(N).content_hash || Record(N).lsn )
  # 侧车 audit.log.sig: sig(N+1) = Ed25519_Sign(TPM 密钥, SHA-256(lsn || content_hash || prev_hash))
```

若任何记录被修改、插入或删除，`content_hash` / `prev_hash` 链将断裂，验证失败。

**验证命令：**

```bash
# （--verbose 为规划中；v0.0.1 的 verify-log 以位置参数接受文件路径）
dologctl verify-log /var/lib/dologger/audit/audit-000001.worm \
    --sidecar /var/lib/dologger/audit/audit-000001.sig

# （示意输出示例）
# [OK]     LSN 000001 — content_hash 有效，签名有效，prev_hash=genesis
# [OK]     LSN 000002 — content_hash 有效，签名有效，prev_hash 匹配
# [GAP]    LSN 000003 — 缺失（期望存在，实际发现 LSN 000004）
# [OK]     LSN 000004 — content_hash 有效，签名有效，prev_hash 匹配
# [FAIL]   LSN 000005 — content_hash 无效（记录可能被篡改）
# ...
# 汇总：9995 通过、1 处缺口、1 失败 — 完整性校验未通过
```

### 安全监控检查清单

- [ ] sysmon 事件已接入集中式日志平台
- [ ] `SIGNATURE_FAILURE` 与 `SANDBOX_VIOLATION` 事件触发 PagerDuty 告警
- [ ] `dologctl verify-log` 通过 cron 每日运行并上报失败
- [ ] 每周对照生产配置审计不可降级项
- [ ] 制定密钥轮换计划（当前手动；自动化轮换为规划中）
- [ ] 每次引擎启动时验证插件签名
- [ ] 控制面已通过防火墙限制为仅本机访问
- [ ] 网络 Sink 的 TLS 证书到期时间受监控

---

## 故障处理流程

### 事件：检测到日志丢失

**症状：**

- sysmon 中出现 `PIPELINE_DROP` 事件
- 输出文件中缺少记录
- LSN 序列出现缺口

**响应：**

1. **分诊**：`curl http://127.0.0.1:9090/status | jq .ring_buffer`（伪代码/示意 — v0.0.1 中控制面尚未启动）
2. **检查丢弃情况**：查看 `pct_used`、`drops_total`、`emergency_spills`
3. **定位瓶颈**：Sink 健康状态 — 是否有 Sink 处于 `circuit_open` 状态？
4. **缓解措施**：
   ```bash
   # （规划中 — v0.0.1 未发布 /sink/disable 端点）
   # 若某 Sink 熔断且非关键，将其禁用
   curl -X POST http://127.0.0.1:9090/sink/disable -d '{"sink": "kafka"}'
   ```
5. **扩容**：
   ```bash
   # 通过环境变量设置更大的环形缓冲区并重启
   export DO_LOG_BUF_SIZE=524288
   ```
6. **恢复**：紧急缓冲区文件会自动重放。使用 `dologctl verify-log` 验证。

### 事件：签名验证失败

**症状：**

- sysmon 中出现 `SIGNATURE_FAILURE` 事件
- `dologctl verify-log` 对一条或多条记录报告 `FAIL`

**响应：**

1. **隔离**：确定受影响的 LSN 范围。
   ```bash
   # （--verbose 为规划中）
   dologctl verify-log /var/lib/dologger/audit/audit-000001.worm 2>&1 | grep FAIL
   ```
2. **评估**：判断是单条记录损坏（磁盘错误）还是系统性篡改。
3. **调查**：
   - 检查受影响时间点附近的系统日志中是否有磁盘 I/O 错误。
   - 验证文件权限 — WORM 文件是否曾被未授权进程写入？
   - 检查受影响时间窗口内主机上的 root 用户活动。
4. **遏制**：若怀疑篡改，将主机与网络隔离并保留取证镜像。
5. **报告**：提交安全事件报告。受影响的记录带有密码学线索 — 保留 WORM 文件作为证据。
6. **修复**：若确认密钥失陷，轮换签名密钥。

### 事件：沙箱违规

**症状：**

- sysmon 中出现 `SANDBOX_VIOLATION` 事件
- 插件进程被终止

**响应：**

1. **识别**：sysmon 事件包含插件名称与尝试调用的系统调用（示意示例）。
   ```json
   {"event":"SANDBOX_VIOLATION","plugin":"untrusted-plugin","syscall":"fork","action":"KILL"}
   ```
2. **隔离**：违规插件已被沙箱终止。
3. **调查**：复核插件的 `manifest.toml` — 其 `trust.color` 是否与实际行为一致？
4. **决策**：
   - 若插件恶意或已失陷：永久移除。
   - 若插件合法但分类错误：仅在代码审查并重新签名后升级其信任色（Red → Yellow、Yellow → Blue）。
5. **预防**：更新插件准入审查流程。

### 事件：性能下降

**症状：**

- `PIPELINE_BACKLOG` 频率上升
- 应用延迟上升（AUDIT 记录阻塞在 `dologger_log`）
- 环形缓冲区占用持续走高

**响应：**

1. **基线**：运行 `cargo bench` 确认引擎自身性能符合预期。
2. **Profile 检查**：核对 `performance_profile` — 是否被改为低吞吐 Profile？
   ```bash
   # （伪代码/示意 — v0.0.1 中控制面尚未启动）
   curl http://127.0.0.1:9090/status | jq .profile
   ```
3. **检查 Sink**：Sink 是否健康？下游变慢会引起背压。
4. **检查签名**：`enable_signature` 是否意外为 `true`？签名使每条记录增加约 17 us。
5. **检查磁盘**：文件 Sink 所在文件系统是否高延迟？
   ```bash
   iostat -x 1
   ```
6. **缓解措施**：
   - 临时将日志级别降至 `WARN` 或 `ERROR`。
   - 若尚未使用，切换到 `prod-performance` Profile。
   - 增大 `ring_buffer_size`。
   - 增加更多 Sink 消费者以并行写入。

### 事后复盘

任何事件发生后，收集诊断报告：

```bash
# （dologctl diag collect 为规划中；当前请手动收集以下内容）
dologctl about --output json > post-incident-$(date +%Y%m%d-%H%M%S).json
dologctl config validate --strict
```

结合 sysmon 事件时间线复核收集到的数据，定位根因并制定预防措施。
