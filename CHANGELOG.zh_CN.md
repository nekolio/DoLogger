# 更新日志

> 🌐 **语言 / Language**: [中文](CHANGELOG.zh_CN.md) | [English: Changelog](CHANGELOG.md)

本项目的所有重要变更都会记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
并且本项目遵循[语义化版本](https://semver.org/lang/zh-CN/spec/v2.0.0.html)。

## [未发布]

## [0.1.0] - 2026-08-12

DoLogger 首次公开发布。

### 新增

- **核心引擎**（`dologger-core`）：7 级流水线、带协作式协助的无锁 MPSC 环形缓冲区、Treiber 栈对象池（提交零 malloc）
- **审计链**：Ed25519 签名记录，LSN + `prev_hash` 链式关联与外部锚定
- **11 种输出落点**：console、file、callback、Kafka、syslog、webhook、SQLite、WORM、security file、shared memory、OpenTelemetry
- **插件系统**：10 种 VTable 插件类型，三色信任模型（蓝/黄/红）
- **插件沙箱**：seccomp-bpf（Linux）、AppContainer（Windows）、Sandbox（macOS）
- **4 种性能预设**：Dev / ProdPerformance / ProdAudit / Balanced
- **合规模板**：GDPR、HIPAA、PCI-DSS
- **语言适配器**：Rust SDK、Python ctypes、Go cgo
- **C ABI**（`dologger_*`）：带 ABI 版本门禁的插件接口与稳定性保证
- **`dologctl` 命令行**：init / run / plugin / config / verify-log / perf / record / shm 等
- **Conan 2.x 集成**：为 C/C++ 插件依赖服务（librdkafka、sqlite3、libsodium）
- **跨平台 CI**：Linux（x86\_64 + aarch64）、macOS（aarch64）、Windows（x86\_64）
- **双语文档**（英文 / 中文）及 Mermaid 图表

### 安全

- 15 项已实现的安全测试：沙箱逃逸、审计链篡改、密钥检测、WORM 强制执行
- 不可降级的安全配置项（签名、WORM、fsync、TLS、Ring 2 签名）
- 环形缓冲区溢出的紧急 AES-256-GCM 加密 mmap 落盘缓冲
- 远程落点故障隔离的熔断器
- 14 条密钥检测规则（AWS、GCP、GitHub 令牌、私钥）
