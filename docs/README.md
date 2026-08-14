# DoLogger Documentation

> Cross-platform, high-security logging engine — complete documentation index.

---

## Language / 语言

| Language | Directory | Description |
|:-:|:-:|:-:|
| **中文** | [zh_CN/](zh_CN/) | 架构参考、插件快速入门、中文指南 |
| **English** | [en_US/](en_US/) | Guides, integration manuals, security whitepaper |

---

## Learning Path

```
New to DoLogger?
  └─▶ QuickStart.md ──▶ IntegrationGuide.md

Plugin developer (C / C++ / Go)?
  └─▶ PluginDevelopmentQuickStart.md ──▶ PluginDevelopmentGuide.md

Integrating into production?
  └─▶ ArchitectureReference.md ──▶ OperationsAndSecurity.md

Plugin developer (Rust)?
  └─▶ PluginDevelopmentGuide.md ──▶ OfficialPluginRoadmap.md

Security engineer / Compliance officer?
  └─▶ SecurityWhitepaper.md ──▶ OperationsAndSecurity.md

Writing a language adapter?
  └─▶ AdapterDevelopmentGuide.md ──▶ HostIntegrationGuide.md

Operating the dologctl CLI?
  └─▶ DologctlCommandReference.md ──▶ OperationsAndSecurity.md

Deep dive into the full specification?
  └─▶ ArchitectureReference.md
```

---

## Quick Navigation

### English Documents (en_US) — Layered from Beginner to Expert

| Layer | Document | Description |
|:-:|:-:|:-:|
| **1** | [Quick Start Guide](en_US/QuickStart.md) | 5-minute setup — clone, build, run. For new users. |
| **2** | [Integration Guide](en_US/IntegrationGuide.md) | C API, config deep-dive, domain inheritance, plugin selection, language adapters. For application developers. |
| **3** | [Architecture Reference](en_US/ArchitectureReference.md) | Pipeline design, ring buffer, audit chain, security model, backpressure, SIF format. For core developers and systems engineers. |
| **4** | [Operations & Security Guide](en_US/OperationsAndSecurity.md) | Deployment modes, monitoring, key management, audit verification, incident response, compliance configuration. For SREs and security engineers. |

### English Documents (en_US) — Developer Guides

| Document | Description |
|:-:|:-:|
| [Plugin Development QuickStart](en_US/PluginDevelopmentQuickStart.md) | Zero-to-plugin for C, C++, Go. Build chain, Conan, cross-compilation. |
| [Host Integration Guide](en_US/guides/HostIntegrationGuide.md) | Full C ABI reference — every function, struct, and error code |
| [Plugin Development Guide](en_US/guides/PluginDevelopmentGuide.md) | Rust plugin VTable implementation, signing, publishing |
| [Adapter Development Guide](en_US/guides/AdapterDevelopmentGuide.md) | Creating language adapters (Python, Go, C, C++) |
| [Extended Plugin Type Guide](en_US/guides/ExtendedPluginTypeGuide.md) | Advanced patterns for all 9 VTable plugin types |
| [dologctl Command Reference](en_US/guides/DologctlCommandReference.md) | Complete CLI reference — every subcommand, option, exit code |

### English Documents (en_US) — Operations & Security

| Document | Description |
|:-:|:-:|
| [Operations Manual](en_US/guides/OperationsManual.md) | Deployment, monitoring, hot-reload, disaster recovery |
| [Security Whitepaper](en_US/guides/SecurityWhitepaper.md) | Threat model, cryptographic design, sandboxing, compliance |
| [Security Development Spec](en_US/guides/SecurityDevelopmentSpec.md) | Memory safety, input validation, fuzzing for plugin devs |
| [Performance Tuning Guide](en_US/guides/PerformanceTuningGuide.md) | System-level tuning: kernel params, CPU affinity, NUMA |
| [Performance Benchmark Guide](en_US/guides/PerformanceBenchmarkGuide.md) | Running and interpreting benchmarks, CI regression detection |
| [Versioning & Deprecation Policy](en_US/guides/VersioningAndDeprecation.md) | Semantic versioning, ABI compatibility, migration guides |

### English Documents (en_US) — Official Plugins

| Document | Description |
|:-:|:-:|
| [Official Plugins](en_US/OfficialPluginRoadmap.md) | Official plugins shipped in v0.1.0 — inventory, not a roadmap |

### 中文文档 (zh_CN) — 分层学习路径

| 层次 | 文档 | 说明 |
|:-:|:-:|:-:|
| **1** | [快速开始指南](zh_CN/QuickStart.md) | 5分钟快速上手 |
| **2** | [集成指南](zh_CN/IntegrationGuide.md) | C API、配置、域继承、语言适配器 |
| **3** | [架构参考手册](zh_CN/ArchitectureReference.md) | 管道设计、环形缓冲区、审计链、安全模型 |
| **4** | [运维与安全指南](zh_CN/OperationsAndSecurity.md) | 部署、监控、密钥管理、事件响应 |

### 中文文档 (zh_CN) — 开发者指南

| 文档 | 说明 |
|:-:|:-:|
| [插件开发快速入门](zh_CN/PluginDevelopmentQuickStart.md) | C/C++/Go 插件零基础入门 |
| [宿主集成手册](zh_CN/guides/HostIntegrationGuide.md) | 将 DoLogger 嵌入宿主应用的完整指南 |
| [插件开发指南](zh_CN/guides/PluginDevelopmentGuide.md) | Rust 插件 VTable 实现、签名、发布流程 |
| [适配器开发指南](zh_CN/guides/AdapterDevelopmentGuide.md) | 为 Python、Go、C/C++ 创建语言适配器 |
| [扩展插件类型开发指南](zh_CN/guides/ExtendedPluginTypeGuide.md) | 9 种 VTable 插件类型高级模式 |
| [dologctl 命令参考](zh_CN/guides/DologctlCommandReference.md) | CLI 完整参考 —— 每个子命令、选项、退出码 |

### 中文文档 (zh_CN) — 运维与安全

| 文档 | 说明 |
|:-:|:-:|
| [运维手册](zh_CN/guides/OperationsManual.md) | 部署、监控、热重载、故障恢复 |
| [安全白皮书](zh_CN/guides/SecurityWhitepaper.md) | 威胁模型、加密方案、沙箱隔离、合规 |
| [安全开发规范](zh_CN/guides/SecurityDevelopmentSpec.md) | 内存安全、输入验证、模糊测试 |
| [高性能调优指南](zh_CN/guides/PerformanceTuningGuide.md) | 系统级调优：内核参数、CPU 亲和性、NUMA |
| [性能基准测试指南](zh_CN/guides/PerformanceBenchmarkGuide.md) | 运行和解读基准测试、CI 回归检测 |
| [版本与废弃策略](zh_CN/guides/VersioningAndDeprecation.md) | 语义化版本、ABI 兼容、迁移指南 |

### 中文文档 (zh_CN) — 官方插件

| 文档 | 说明 |
|:-:|:-:|
| [官方插件](zh_CN/OfficialPluginRoadmap.md) | v0.1.0 随附的官方插件清单 — 非路线图 |

### 设计规范

[架构参考](zh_CN/ArchitectureReference.md) / [Architecture Reference](en_US/ArchitectureReference.md) 是 DoLogger 架构决策、API 与安全属性的权威设计文档。

### Project-Level

| Document | Description |
|:-:|:-:|
| [README.md](../README.md) | Project overview, quick start, architecture diagram |
| [README.zh_CN.md](../README.zh_CN.md) | 中文项目概述 |
| [GitHub Releases](https://github.com/Nekolio/DoLogger/releases) | Release history — each release page is the changelog |
| [adapters/README.md](../adapters/README.md) | Language adapter SDKs (Rust, Python, Go) |
| [compliance/README.md](../compliance/README.md) | GDPR / HIPAA / PCI-DSS compliance templates |
| [conanfile.py](../conanfile.py) | Conan 2.x C dependency recipe |
| [cmake/](../cmake/) | CMake helper modules (cross-compilation, Conan toolchain) |
| [scripts/](../scripts/) | Build and setup scripts (local + CI) |
| [tools/](../peripheral/tools/) | Maintainer-only auxiliary tools — **not part of the project**; see [tools/README.md](../peripheral/tools/README.md) |
| [site/](../peripheral/site/) | Vue 3 + TypeScript landing page (GitHub Pages) |
| [github/](../peripheral/github/) | GitHub publishing automation — Pages / wiki / release scripts |
| [.conan/profiles/](../.conan/profiles/) | Pre-built Conan cross-compilation profiles |

### Project Conventions / 工程规范

| Document | Description |
|:-:|:-:|
| [Repository Layout](en_US/guides/RepositoryLayout.md) / [仓库布局](zh_CN/guides/RepositoryLayout.md) | Six-zone root map — product vs build-infra vs peripheral（六区根地图：产品 / 构建 / 外围） |
| [Naming Convention](en_US/guides/NamingConvention.md) / [命名规范](zh_CN/guides/NamingConvention.md) | Path-as-namespace, role suffixes, abbreviation rules（路径即命名空间、角色词表、缩写规则） |

---

## Documentation Standards

All documentation follows the project documentation standards:

- **Structure**: audience classification, cross-references, terminology
- **Code style**: C, Rust, Go, Python, TOML — comment conventions, API documentation templates

---

*Last updated: 2026-08-12*
