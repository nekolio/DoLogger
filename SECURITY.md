# Security Policy

> 🌐 **语言 / Language**: 中文说明见[下半部分](#安全政策中文版) · Full Chinese docs: [安全开发规范](docs/zh_CN/guides/SecurityDevelopmentSpec.md)

## Supported Versions

| Version | Supported | Notes |
|:-:|:-:|:-:|
| 0.1.x (latest) | ✅ | Pre-1.0 development phase — pin exact versions |
| < 0.1.0 | ❌ | Not released |

## Reporting a Vulnerability

DoLogger is a security product — reports from security researchers are treated with priority.

1. **Contact**: send details to **nekoliowork+DoLogger@gmail.com** with subject `[SECURITY] <short summary>`.
2. **What to include**:
   - Affected version and platform (Linux/macOS/Windows)
   - Reproduction steps or a proof-of-concept
   - Impact assessment (e.g. audit-chain bypass, sandbox escape)
3. **Encryption**: if the report contains sensitive material, ask for a PGP key in your first email.
4. **Response targets**: initial acknowledgment within **48 hours**, fix targeted within **7 days** for critical issues, coordinated disclosure afterwards.

### Scope

- Core engine (`core/`), `dologctl` CLI (`cli/`), official plugins (`plugins/official/`)
- C ABI misuse that breaks the ABI stability or audit-chain guarantees
- Sandbox escapes (seccomp-bpf / AppContainer / Sandbox)

### Out of scope

- Issues in example plugins (`plugins/examples/`) — report as regular bugs
- Vulnerabilities in third-party dependencies — reported upstream (see `dologctl version --licenses`)

## Security Model

The design baseline is documented in:

- [Security Whitepaper](docs/en_US/guides/SecurityWhitepaper.md) — threat model, Ed25519 audit chain, WORM guarantees
- [Security Development Spec](docs/en_US/guides/SecurityDevelopmentSpec.md) — 15 implemented security tests, coding requirements

---

## 安全政策(中文版)

### 支持的版本

| 版本 | 支持状态 | 说明 |
|:-:|:-:|:-:|
| 0.1.x(最新) | ✅ | 1.0 之前开发阶段 —— 请锁定确切版本 |
| < 0.1.0 | ❌ | 尚未发布 |

### 报告漏洞

DoLogger 本身是安全产品,来自安全研究者的报告将被优先处理。

1. **联系方式**:将详情发送至 **nekoliowork+DoLogger@gmail.com**,邮件主题 `[SECURITY] <简短摘要>`。
2. **应包含内容**:
   - 受影响版本与平台(Linux/macOS/Windows)
   - 复现步骤或概念验证(PoC)
   - 影响评估(例如审计链绕过、沙箱逃逸)
3. **加密**:若报告含敏感材料,请先来信索取 PGP 公钥。
4. **响应目标**:48 小时内初步确认;严重问题 7 天内完成修复;随后协调披露。

### 范围

- 核心引擎(`core/`)、`dologctl` 命令行(`cli/`)、官方插件(`plugins/official/`)
- 破坏 ABI 稳定性或审计链保证的 C ABI 误用
- 沙箱逃逸(seccomp-bpf / AppContainer / Sandbox)

### 范围之外

- 示例插件(`plugins/examples/`)中的问题 —— 请按常规 Bug 报告
- 第三方依赖中的漏洞 —— 上报至上游(参见 `dologctl version --licenses`)

### 安全模型

设计基线文档:

- [安全白皮书](docs/zh_CN/guides/SecurityWhitepaper.md) —— 威胁模型、Ed25519 审计链、WORM 保证
- [安全开发规范](docs/zh_CN/guides/SecurityDevelopmentSpec.md) —— 15 项已实现的安全测试与编码要求
