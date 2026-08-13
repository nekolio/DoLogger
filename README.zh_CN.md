# DoLogger

> 下一代安全日志引擎 — 无锁速度下的 Ed25519 审计链。

<p align="center">
  <img src="./Docs/assets/hero.svg" alt="DoLogger 启动序列 — Hello DoLogger、4 个沙箱插件、Ed25519 审计链已武装、7 级管道在线" width="880">
</p>

[English](README.md) | [中文](README.zh_CN.md)

<p align="center">
  <a href="https://github.com/Nekolio/DoLogger/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Nekolio/DoLogger/ci.yml?branch=main&style=flat-square&label=CI" alt="CI"></a>
  <a href="https://github.com/Nekolio/DoLogger/releases"><img src="https://img.shields.io/github/v/release/Nekolio/DoLogger?include_prereleases&style=flat-square&label=release" alt="Release"></a>
  <a href="https://github.com/Nekolio/DoLogger/stargazers"><img src="https://img.shields.io/github/stars/Nekolio/DoLogger?style=flat-square&color=yellow" alt="Stars"></a>
  <a href="https://github.com/Nekolio/DoLogger/blob/main/LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/rust-stable-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/platform-Linux_|_macOS_|_Windows-808080?style=flat-square" alt="Platform">
  <a href="https://github.com/Nekolio/DoLogger/commits/main"><img src="https://img.shields.io/github/last-commit/Nekolio/DoLogger?style=flat-square&label=last%20commit" alt="Last commit"></a>
</p>

---

## 概述

DoLogger 是一个跨平台、高安全性的日志引擎,为需要签名、防篡改审计日志
的应用而设计。它将纳秒级延迟的无锁记录提交与 Ed25519 签名审计链、插件
沙箱隔离、以及 11 种内置输出接收器相结合——全部由 TOML 配置驱动,支持
域继承与不可降级的安全保障。

| 能力 | DoLogger | 传统日志库 |
|:-:|:-:|:-:|
| **提交延迟（P50）** | 102 ns | 500–2000 ns |
| **批量吞吐量** | 13.3M rec/s | 1–5M rec/s |
| **审计链** | Ed25519 + LSN + prev_hash 区块链 | 少见 / 外挂 |
| **插件沙箱** | seccomp-bpf / AppContainer / Sandbox | 无 |
| **性能配置** | 4 种配置（Dev/Prod/ProdAudit/Balanced） | 手动调优 |
| **输出接收器** | 11 种内置（Console、File、Callback、Kafka、Syslog、Webhook、SQLite、WORM、Security、Shared Memory、OTel） | 通常 1–3 种 |
| **配置方式** | TOML + 域继承 + 7 级优先级 | 平铺式配置 |

---

## 特性

- `[PERF]` **无锁热路径** — 基于 CAS 的环形缓冲区 + Treiber 栈对象池;记录提交零堆分配(本地 P50 ≈ 102 ns)。
- `[SIGN]` **Ed25519 审计链** — 每条审计记录在组装阶段被签名,通过 LSN + prev_hash 链接成链;离线可用 `dologctl verify-log` 校验。
- `[SINK]` **11 种接收器 + 沙箱插件** — Console、File、Kafka、Syslog、Webhook、SQLite、WORM、Security、Shared Memory、OTel、Callback;插件在 seccomp-bpf / AppContainer / Sandbox 隔离下运行,按信任级别着色。
- `[OBSV]` **内置可观测性** — 逐记录管道耗时(`--trace`)、SIF 录制/回放、`dologctl perf` 基准测试、崩溃恢复报告。

---

## 性能快照

基于同一份代码测量(release + LTO);与其他 Rust 日志库的横向对比数据
尚未发布——每个 release 都会在发布说明中携带 GitHub Actions runner 上
的最新实测数据。

| 环境 | 提交 P50 | 吞吐量 | 签名提交（Ed25519） |
|:-:|:-:|:-:|:-:|
| GitHub runner — AMD EPYC 7763,v0.1.0 release | **120 ns** | 5.06M rec/s | 19.8 µs |
| 本机 — Windows 11 LTSC,Intel i5-12400F | **102 ns** | 9.78M rec/s | 16.96 µs |

Criterion(同一本机):

| 基准测试 | P50 | 吞吐量 |
|:-:|:-:|:-:|
| 单条记录提交 | **102 ns** | ~9.78M rec/s |
| 环形缓冲区推入（1K） | **121 µs** | ~8.26M rec/s |
| 批量推入（256） | **19.2 µs** | ~13.3M rec/s |

CRC32C 通过 SSE 4.2(`_mm_crc32_u64`)硬件加速,并提供 Slicing-by-8
软件回退。

---

## 快速开始

### 预编译二进制

每个 [GitHub Release](https://github.com/Nekolio/DoLogger/releases) 都
附带 `dologctl-<os>-<arch>` 二进制(Windows 上为 `.exe`)及对应架构的
核心库。请用随附的 `checksums-sha256.txt` 校验每个下载文件:

```bash
curl -fLO https://github.com/Nekolio/DoLogger/releases/download/v0.1.0/dologctl-linux-x86_64
chmod +x dologctl-linux-x86_64
./dologctl-linux-x86_64 init --template dev
./dologctl-linux-x86_64 run --config dologger.toml
```

### 从源码构建

```bash
git clone https://github.com/Nekolio/DoLogger.git
cd DoLogger
cargo build --release

./target/release/dologctl init --template dev
./target/release/dologctl run --config dologger.toml
```

### Rust SDK

SDK 随仓库发布(`adapters/rust`);以路径依赖方式引入:
`dologger-sdk = { path = "adapters/rust" }`。

```rust
use dologger_sdk::Logger;

fn main() {
    let mut logger = Logger::init(None).expect("init"); // 默认配置
    logger.info("Application started");
    logger.audit("User 42 deleted record #7"); // 签名 + WORM
    logger.shutdown();
}
```

审计记录经 Ed25519 签名;离线校验日志:

```shell
dologctl verify-log audit.log
```

### Shell 补全

```bash
source <(dologctl completions bash)                              # bash
source <(dologctl completions zsh)                               # zsh
dologctl completions fish | source                               # fish
dologctl completions powershell | Out-String | Invoke-Expression # PowerShell
```

> [!TIP]
> 将补全脚本写入 shell 配置文件,让每个新终端自动生效,例如 `dologctl completions bash > ~/.dologctl-complete.bash && echo 'source ~/.dologctl-complete.bash' >> ~/.bashrc`。

---

## 架构

> [!IMPORTANT]
> DoLogger 目前处于 **1.0 之前**阶段。MINOR 版本可能包含破坏性变更,ABI 也可能发生变化——生产环境请锁定到确切版本。详见[版本管理与弃用策略](Docs/zh_CN/guides/VersioningAndDeprecation.md)。

![架构](./Docs/assets/architecture.svg)

应用将记录直接推入无锁 MPSC 环形缓冲区——热路径上没有任何锁。后台
管道运行七个阶段(PreFilter → Filter → FieldProvider → Assembly →
Processing → Formatting → Sink),在 Assembly 阶段用 Ed25519 为审计
记录签名,在 Processing 阶段校验校验和。批量排空后扇出到接收器层,
慢速 I/O 永远不会阻塞生产者。

### 关键设计决策

- **无锁热路径**:基于 CAS 的环形缓冲区 + Treiber 栈对象池——记录提交零堆分配
- **Ring 0–3 字段权限**:CPU 式特权环模型;Ring 2 修改自动追加审计追踪
- **AUDIT 铁律**:`block_timeout_ms=0`,`drop_strategy=Never`——审计记录绝不丢弃
- **背压控制**:90% 告警 + 协作式帮助,95% 紧急模式 + 可选丢弃
- **6 项不可降级配置**:`enable_signature`、`escape_html`、`worm_enabled`、`fsync_on_write`、`require_tls`、`sign_ring2`
- **4 种性能配置**:Dev / ProdPerformance / ProdAudit / Balanced——每种绑定具体超时与策略值

---

## 配置与部署

默认配置开箱即用——`dologctl run` 不带配置时使用内置默认值,
`dologctl init --template dev` 生成开发模板。

| 环境变量 | 用途 |
|:-:|:-:|
| `DO_LOGGER_LIB_PATH` | 语言适配器共享库路径 |
| `DO_LOG_PLUGIN_DIR` | 插件搜索路径(覆盖 `./plugins`) |
| `DO_LOG_CONFIG_FILE` | `dologctl config validate` 的配置文件 |

```shell
dologctl init --template gdpr    # 欧盟 GDPR
dologctl init --template hipaa   # 美国 HIPAA
dologctl init --template pci     # PCI-DSS
```

合规模板自动激活不可降级安全项并强制审计要求。

---

## dologctl CLI

```
dologctl init                    生成配置模板
dologctl run --trace             启动引擎并追踪每条记录的管道耗时
dologctl plugin list             列出已安装插件（含信任颜色）
dologctl plugin install <path>   安装插件
dologctl plugin verify [name]    验证插件签名与 ABI
dologctl plugin scan             安全扫描可疑符号
dologctl config validate         验证配置文件（--strict 严格模式）
dologctl verify-log <file>       离线审计日志验证
dologctl verify-anchor           外部锚定验证
dologctl recovery-report         崩溃恢复报告
dologctl record / replay         SIF 录制与回放
dologctl shm status              共享内存通道检查
dologctl perf                    性能基准测试
dologctl completions <shell>     Shell 补全脚本
dologctl version                 项目横幅与系统信息
dologctl version --licenses      第三方许可证归属
```

全局参数:`--output json|text`、`--color auto|always|never`、`--quiet`、`--config <path>`

---

## 插件系统（10 种 VTable 类型）

| 插件类型 | 管道阶段 | 说明 |
|:-:|:-:|:-:|
| **Filter** | 1 | 基于规则丢弃或放行记录 |
| **FieldProvider** | 2 | 注入字段(HostInfoProvider 为受限子类型) |
| **Processor** | 4 | 转换 / 增强 / 检测密钥 |
| **Formatter** | 5 | 将记录序列化为输出格式 |
| **IOSink** | 6 | 最终输出目标 |
| **ConfigProvider** | — | 外部配置源(远程配置中心) |
| **KeyProvider** | — | Ed25519 密钥服务(可外接 HSM) |
| **PolicyProvider** | 0 | 提交前策略(限流、级别过滤) |
| **HostInfoProvider** | 2 | 系统信息注入(ring1_only=true) |
| **SyscallBroker** | — | 沙箱插件的系统调用代理 |

### 信任级别

| 级别 | 颜色 | 签名要求 | 系统调用权限 | 可用插件类型 |
|:-:|:-:|:-:|:-:|:-:|
| **Blue** | 蓝 | Ed25519 签名 | 完整 | 全部 |
| **Yellow** | 黄 | 自签名 | 受限 | 有限 |
| **Red** | 红 | 无(开发模式) | 最小白名单 | 仅 Filter、Formatter、Processor |

---

## 安全

- **Ed25519 审计链**:每条审计记录均被签名;LSN + prev_hash 形成类区块链防篡改链条
- **WORM 存储**:一次写入多次读取,fsync + 只读权限强制
- **插件沙箱**:seccomp-bpf(Linux)、AppContainer(Windows)、Sandbox(macOS),信任颜色能力矩阵
- **密钥检测**:14 条前缀匹配规则,覆盖 Critical/High/Medium 三个严重级别(AWS、GCP、GitHub Token、私钥等)
- **密钥轮换 + CRL**:多密钥并行验证、轮换生命周期、紧急吊销
- **外部锚定**:定期将根哈希锚定到不可变存储(S3/HTTP)
- **断路器**:3 状态(CLOSED→OPEN→HALF_OPEN→CLOSED),用于远程接收器故障隔离
- **紧急 mmap 缓冲区**:AES-256-GCM 加密溢出缓冲区,环形缓冲区溢出时启用

---

## 语言适配器

| 语言 | 位置 | 状态 |
|:-:|:-:|:-:|
| **Rust** | `adapters/rust/` | SDK crate(dologger-sdk) |
| **Python** | `adapters/python/` | ctypes 封装 |
| **Go** | `adapters/go/` | cgo 封装 |

---

## 项目结构

<details>
<summary>仓库目录布局</summary>

```
DoLogger/
├── core/                       # 核心引擎（Rust cdylib）
│   ├── src/                    # 40+ 模块
│   ├── include/                # C ABI 公共头文件
│   └── benches/                # Criterion 基准测试
├── cli/                        # dologctl CLI 工具
│   └── src/commands/           # 子命令实现
├── plugins/                    # 插件生态
│   ├── official/               # 官方插件（fmt_json、filter_level、fmt_text、field_container）
│   └── examples/               # 多语言示例（Rust、C、C++、Go）
├── adapters/                   # 语言 SDK（Rust、Python、Go）
├── compliance/                 # GDPR/HIPAA/PCI-DSS 合规模板
├── Docs/                       # 技术文档
│   ├── assets/                 # 静态资源（架构图、图片）
│   ├── zh_CN/                  # 中文文档
│   └── en_US/                  # 英文文档（自动同步至 GitHub wiki）
├── tests/                      # 集成与安全测试
└── scripts/                    # 开发环境搭建脚本
```

</details>

---

## 编译

```bash
# 前置条件:Rust stable,CMake ≥ 3.20
cargo build --release

# 启用 Kafka 支持(需 librdkafka)
cargo build --release --features sink-kafka

# 跨平台目标检查
cargo check --target x86_64-unknown-linux-gnu
cargo check --target x86_64-apple-darwin
cargo check --target aarch64-apple-darwin
```

> [!WARNING]
> `sink-kafka` 需要 librdkafka(通过 Conan 或系统包管理器安装)。CI 与发布构建仅在 Linux x86_64 上包含该特性;macOS、Windows 和 Linux aarch64 构建不包含它。

---

## 部分文档

| 指南 | 内容 |
|:-:|:-:|
| [架构参考](Docs/zh_CN/ArchitectureReference.md) | 管道、环形缓冲区、审计链、安全模型 |
| [dologctl 命令参考](Docs/zh_CN/guides/DologctlCommandReference.md) | 每个 CLI 子命令、选项与退出码 |
| [插件开发快速入门](Docs/zh_CN/PluginDevelopmentQuickStart.md) | C/C++/Go 插件开发 |
| [插件开发指南](Docs/zh_CN/guides/PluginDevelopmentGuide.md) | Rust 插件开发 |
| [安全白皮书](Docs/zh_CN/guides/SecurityWhitepaper.md) | 威胁模型与加密设计 |
| [文档总索引](Docs/README.md) | 全部指南,英文 + 中文 |

---

## 参与贡献

欢迎参与贡献!Bug 报告与功能建议请使用 [issue 模板](https://github.com/Nekolio/DoLogger/issues/new/choose);Pull Request 须满足 [PR 检查清单](.github/pull_request_template.md)。安全漏洞请按 [SECURITY.md](SECURITY.md) 私下报告,不要以公开 issue 形式提交。

### 贡献者

<a href="https://github.com/Nekolio">
  <img src="https://images.weserv.nl/?url=https://github.com/Nekolio.png&w=96&h=96&fit=cover&mask=circle" width="96" height="96" alt="@Nekolio" />
</a>

[@Nekolio](https://github.com/Nekolio) —— 项目作者与维护者

---

## 许可证

基于 [Apache License 2.0](LICENSE-APACHE) 或 [MIT license](LICENSE-MIT) 二者选一进行许可。

---

## Star 曲线

<a href="https://star-history.com/#Nekolio/DoLogger&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=Nekolio/DoLogger&type=Date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=Nekolio/DoLogger&type=Date" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=Nekolio/DoLogger&type=Date" />
  </picture>
</a>

---

*由 [@Nekolio](https://github.com/Nekolio) 构建 | nekoliowork+DoLogger@gmail.com*
