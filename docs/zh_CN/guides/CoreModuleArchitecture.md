# 核心模块架构

> 架构审查基线：本地 `main` 的 `87b8a7b`，加上 2026-08-21 尚未提交的
> 记录安全、错误码、编解码、本地化和资源布局修改。不以远端状态作为实现依据。

## 决策摘要

编码与解码是**核心能力**，不是本地化局部功能，也不是普通动态插件。核心必须
提供确定、可校验的 codec 契约，因为平台输出、FFI 文本、配置值、目录输入和未来
非 UTF-8 适配都需要经过同一边界。

本地化只在面向人的展示边界调用核心 codec。它负责 locale 选择、目录查找、fallback
和翻译，不拥有代码页策略。持久化记录、SIF/KV 数据、WORM 容器、哈希、签名和审计链
字节保持规范形式，绝不经过本地化或展示转码。

AUDIT 仍然是**可选使用场景**。它放在 `security/audit` 是为了明确安全边界，不代表
审计日志默认或强制开启。

## 规范模块地图

```text
core/src/
├── buffer/          所有权令牌、对象池、环形缓冲区、紧急内存
├── codec/           核心文本编解码与平台检测
├── config/          配置模型、校验、watcher、热重载
├── error.rs         稳定数字错误码、描述与 fallback
├── ffi.rs           C ABI 边界与 last-error 出口
├── localization/    locale 链、目录、fallback registry
├── pipeline/        阶段、调度、背压、准入策略
├── plugin/          加载、ABI 校验、沙箱、配额、分发
├── record/          热路径 Record 与 KV 表示
├── security/        密码学、密钥、TPM 边界、秘密检测、审计
├── sif/             结构化持久化格式 codec
├── sink/            输出 sink，包括 WORM/安全 sink
├── sys/             OS 服务、I/O、诊断、控制面
└── util/            小型无依赖工具
```

为现有 Rust 调用者保留兼容别名：

| 旧路径 | 规范路径 |
|---|---|
| `dologger_core::encoding` | `dologger_core::codec` |
| `dologger_core::i18n` | `dologger_core::localization` |
| `dologger_core::policy` | `dologger_core::pipeline::policy` |
| `dologger_core::audit` | `dologger_core::security::audit` |

新代码必须使用规范路径，兼容别名不代表新的架构边界。

## 为什么 codec 不是插件

动态插件是不可信、版本独立、可单独部署的扩展，适合可选展示和流水线行为，但不应
成为规范字节的权威。如果 codec 插件控制审计或持久化字节，它可能改变哈希、签名、
重放语义或跨平台校验结果，也会让基础核心能力变成启动依赖，并把 ABI、分配和失败
开销带入热路径。

因此核心 codec 负责：

- UTF-8 作为规范文本表示；
- 带范围校验的显式 Windows 代码页转换；
- locale/codeset 解析与平台探测；
- 无损转换策略和非法字节拒绝；
- 供调用者和未来 FFI 适配使用的稳定错误类型。

未来可以通过内部 codec trait 按平台或 feature 选择内置后端，但这只是实现细节，不
允许外部插件重新定义规范序列化。

## 插件可以参与什么

| 扩展 | 允许职责 | 禁止职责 |
|---|---|---|
| Formatter | 记录处理后的面向人展示 | 重写规范审计/持久化字节 |
| Filter / processor | 明确声明的流水线行为 | 绕过安全准入或所有权规则 |
| Catalog provider（未来 ABI） | 提供经过校验的 locale 条目 | 修改错误码或本地化审计字节 |
| Codec backend（未来、需审查） | 明确授权的展示/配置转换 | FFI 线布局、SIF/KV、WORM、哈希或签名编码 |

Catalog provider 是本地化扩展；如果未来需要 codec backend，它也是独立的受审能力，
不能伪装成 formatter 插件。

## 运行时边界

1. 记录以规范 Rust/FFI 表示进入核心。
2. 流水线在声明的阶段内执行策略和插件。
3. 持久化与安全 sink 使用规范字节完成序列化、哈希或签名，不做本地化。
4. 面向人的输出根据错误码/消息 key 通过本地化解析文本。
5. 输出层向核心 codec 请求目标展示编码。
6. `sys::io` 写出字节，或调用平台 Unicode 控制台 API；重定向/文件输出保持 UTF-8。

生产者热路径不执行目录查找或依赖 locale 的转换。

## 迁移与未完成项

- `core/src/codec/` 当前提供文本 codec 桩和常见代码页处理；POSIX/macOS 原生 codeset
  后端仍为 TODO(author)。
- `core/src/localization/` 当前提供经过校验的内存目录；Fluent 兼容编译、受限重载和
  目录 provider ABI 仍为 TODO(author)。
- `core/src/sif/` 仍是持久化格式边界，计划中的 SIF → KV 迁移与展示编码是两件事。
- `security/tpm.rs` 在真实 provider 接入前不能宣称硬件能力已完成。
- 兼容别名只能在规划好的 major API 变更中移除。

参见[本地化架构](LocalizationArchitecture.md)、
[ADR-006](../../../.agents/living/decisions/ADR-006-localization-architecture.md)和
[ADR-007](../../../.agents/living/decisions/ADR-007-core-module-boundaries.md)。
