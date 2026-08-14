# 仓库布局

> 仓库根的权威地图。它回答一个问题：**根目录里哪些是产品、哪些是构建所必需、哪些两者都不是（外围）。**

## 1. 六区划分

```
DoLogger/
│  ① 平台入口（GitHub 约定钉死）
├── README.md  README.zh_CN.md     ← 落地页（GitHub 渲染根 README）
├── LICENSE-APACHE  LICENSE-MIT    ← 许可检测
├── NOTICE  SECURITY.md            ← 第三方声明 / 安全策略
├── .github/                       ← Actions、issue/PR 模板、CODEOWNERS
│   .gitignore                     ← 仅本地（刻意不追踪）
│
│  ② 构建入口（cargo/cmake/conan 自动探测）
├── Cargo.toml  Cargo.lock         ← Rust workspace 根
├── rustfmt.toml  deny.toml        ← cargo fmt / cargo-deny 配置
├── CMakeLists.txt                 ← `cmake -S .` 约定
├── conanfile.py                   ← `conan install .` 约定
├── .cargo/                        ← cargo 配置
│
│  ③ 产品（DoLogger 三层架构本体）
├── core/                          ← 稳定内核（libdologger_core，稳定 C ABI）
├── cli/                           ← dologctl
├── plugins/                       ← 插件生态（official / examples / community）
├── adapters/                      ← 语言 SDK（C、Rust、Python、Go）
├── compliance/                    ← GDPR / HIPAA / PCI-DSS 模板
├── config/                        ← 示例配置
├── examples/                      ← 最小宿主应用示例（C ABI 消费者）
├── tests/                         ← 测试套件（common / release-smoke / security）
│
│  ④ 文档（内容）
├── docs/                          ← 中英双语文档，自动同步至 wiki
│
│  ⑤ 构建基础设施（源码——构建时必须存在）
├── cmake/                         ← CMake 辅助模块
├── docker/                        ← 容器镜像（Dockerfile.dev；运行时镜像在 v1.0.0）
├── .conan/                        ← 交叉编译 profile
├── scripts/                       ← 构建 / 环境脚本（本地 + CI）
│
│  ⑥ 外围（非产品、非构建）
└── peripheral/
    ├── site/                      ← GitHub Pages 营销站（Vue 3）
    ├── github/                    ← GitHub 发布自动化
    │   └── scripts/               ←   build-site · sync-wiki · generate-release-notes
    └── tools/                     ← 仅维护者使用的工具（hero-svg）
```

## 2. 什么是钉死的、为什么

① 和 ② 搬走会静默破坏平台功能：

| 条目 | 钉死原因 |
|:-:|:-:|
| `README.md` | GitHub 将根 README 渲染为落地页 |
| `LICENSE-*` | GitHub 许可检测 / 许可 API |
| `.github/` | GitHub Actions 只从根 `.github/` 运行 |
| `Cargo.toml` / `Cargo.lock` | cargo workspace 根约定 |
| `rustfmt.toml` / `deny.toml` | `cargo fmt` / cargo-deny 自动探测 |
| `CMakeLists.txt` | `cmake -S .` 约定 |
| `conanfile.py` | `conan install .` 约定 |
| `.cargo/` | cargo 从根自动发现 `.cargo/config.toml` |

这些留在根目录，并被**文档化为平台入口**，而非产品的一部分。移动其中任何一个，落地页、许可徽章、CI 或构建工具都会停止工作——且通常没有任何报错。

## 3. 真正重要的区分

- ⑤（**构建基础设施**）是**源码**：`cmake/`、`docker/`、`.conan/`、`scripts/` 是构建和交付产品所必需的。它们不是「额外」，它们与产品目录一样位于根。
- ⑥（**外围**）是唯一的非产品内容：营销站与维护工具。两者既不随产品交付，也不是构建所需。它们统一放在 `peripheral/` 下，让根目录「产品优先」。
- ① 和 ② 是平台开销。它们不可避免，其角色被文档化，这样没人会把 `LICENSE` 误认为源文件。

## 4. 本次布局对齐移动了什么

| 原位置 | 新位置 | 原因 |
|:-:|:-:|:-:|
| `site/`（根） | `peripheral/site/` | 非产品：营销 |
| `tools/`（根） | `peripheral/tools/` | 非产品：维护工具 |
| `Docs/` | `docs/` | 小写，对齐企划书 §3.3，并修复大小写敏感的 `.gitignore` 不匹配 |
| `scripts/build-site.sh` · `sync-wiki.sh` · `generate-release-notes.sh` | `peripheral/github/scripts/` | GitHub 发布自动化属外围，非构建基础设施 |
| `scripts/ci-build.sh` · `ci-test.sh` · `*.ps1` | *（已删除）* | 死代码 / 孤儿 —— workflows 内联执行等价命令 |

部署路径已同步更新：`pages.yml` / `wiki-sync.yml` 的 `paths:` 过滤器、`peripheral/github/scripts/build-site.sh`、`peripheral/github/scripts/sync-wiki.sh`、`peripheral/tools/hero-svg/hero_gen.py`。

## 5. 新增条目的判断准则

问一句：**它随产品交付吗？或构建时需要它吗？**

- 产品 / 构建必需 → 顶层产品目录或 ⑤ 目录。
- 两者都不是 → `peripheral/`。
- 平台元数据（许可、README、CI）→ 根，并在本文档中记录为平台入口。
