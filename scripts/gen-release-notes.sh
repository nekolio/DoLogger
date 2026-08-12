#!/usr/bin/env bash
# gen-release-notes.sh — assemble the GitHub Release body.
#
# The release page is the changelog for its version. The body is written in
# two self-contained sections — English, then Chinese — switched via the
# language bar at the top. The release workflow fills the benchmark
# placeholders (<!--BENCH_EN--> / <!--BENCH_ZH-->) with the output of the
# per-release benchmark job.
#
# Usage:
#   scripts/gen-release-notes.sh <tag> > release-notes.md
#
# Requires a full git history (actions/checkout with fetch-depth: 0).
set -euo pipefail

TAG="${1:?usage: gen-release-notes.sh <tag>}"
BODY="$(mktemp)"

# ── Commit list since the previous tag ─────────────────────────────
PREV_TAG="$(git describe --tags --abbrev=0 "${TAG}^" 2>/dev/null || true)"
if [ -n "$PREV_TAG" ]; then
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${PREV_TAG}..${TAG}" 2>/dev/null | head -100 || true)"
    CHANGELOG_EN="Changes since ${PREV_TAG}"
    CHANGELOG_ZH="自 ${PREV_TAG} 以来的变更"
else
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${TAG}" 2>/dev/null | head -100 || true)"
    CHANGELOG_EN="Initial release"
    CHANGELOG_ZH="首次发布"
fi
COMMITS="${COMMITS:-_(no commits found — does the workflow check out full history?)_}"

DL_BASE="https://github.com/Nekolio/DoLogger/releases/download/${TAG}"

{
    echo "# DoLogger ${TAG}"
    echo
    echo "[English](#english) | [中文](#chinese)"
    echo
    echo '<a id="english"></a>'
    echo
    echo '## English'
    echo
    echo 'DoLogger is a cross-platform, high-security logging engine for'
    echo 'applications that need signed, tamper-evident audit logs.'
    echo
    echo '<!--BENCH_EN-->'
    echo
    echo '### Changelog'
    echo
    echo "**${CHANGELOG_EN}**"
    echo
    printf '%s\n' "${COMMITS}"
    echo
    echo '### Quick Download'
    echo
    echo 'All assets are attached at the bottom of this page. The commands'
    echo 'below fetch the CLI binary for the most common platforms; the rest'
    echo 'of the platforms are listed in the asset table.'
    echo
    echo '**Linux (x86_64)**'
    echo
    echo '```bash'
    echo "curl -fLO ${DL_BASE}/dologctl-linux-x86_64"
    echo 'chmod +x dologctl-linux-x86_64'
    echo '```'
    echo
    echo '**Linux (aarch64)**'
    echo
    echo '```bash'
    echo "curl -fLO ${DL_BASE}/dologctl-linux-aarch64"
    echo 'chmod +x dologctl-linux-aarch64'
    echo '```'
    echo
    echo '**macOS (Apple Silicon)**'
    echo
    echo '```bash'
    echo "curl -fLO ${DL_BASE}/dologctl-macos-aarch64"
    echo 'chmod +x dologctl-macos-aarch64'
    echo '```'
    echo
    echo '**Windows (x86_64)** — PowerShell:'
    echo
    echo '```powershell'
    echo "curl.exe -fLO ${DL_BASE}/dologctl-windows-x86_64.exe"
    echo '```'
    echo
    echo '### Asset Naming'
    echo
    echo 'CLI binaries follow `dologctl-<os>-<arch>`. Windows binaries end'
    echo 'in `.exe`; Linux and macOS binaries carry no extension (POSIX'
    echo 'convention). Each platform also ships its core library'
    echo '(`dologger_core`), and `checksums-sha256.txt` holds the SHA-256 of'
    echo 'every asset.'
    echo
    echo '| OS | Architecture | CLI asset | Core library |'
    echo '|:-:|:-:|:-:|:-:|'
    echo '| Linux | x86_64 | `dologctl-linux-x86_64` | `libdologger_core.so` |'
    echo '| Linux | aarch64 | `dologctl-linux-aarch64` | `libdologger_core.so` |'
    echo '| Linux | i686 (32-bit) | `dologctl-linux-i686` | `libdologger_core.so` |'
    echo '| Linux | armv7 (32-bit) | `dologctl-linux-armv7` | `libdologger_core.so` |'
    echo '| Linux | riscv64 | `dologctl-linux-riscv64` | `libdologger_core.so` |'
    echo '| Windows | x86_64 | `dologctl-windows-x86_64.exe` | `dologger_core.dll` |'
    echo '| Windows | aarch64 | `dologctl-windows-aarch64.exe` | `dologger_core.dll` |'
    echo '| Windows | i686 (32-bit) | `dologctl-windows-i686.exe` | `dologger_core.dll` |'
    echo '| macOS | aarch64 (Apple Silicon) | `dologctl-macos-aarch64` | `libdologger_core.dylib` |'
    echo '| macOS | x86_64 (Intel) | `dologctl-macos-x86_64` | `libdologger_core.dylib` |'
    echo
    echo '### Verify Your Download'
    echo
    echo 'Compare the SHA-256 of a downloaded file against its entry in'
    echo '`checksums-sha256.txt` before running it.'
    echo
    echo '**Linux / macOS**'
    echo
    echo '```bash'
    echo "grep 'dologctl-linux-x86_64' checksums-sha256.txt | sha256sum -c -"
    echo '```'
    echo
    echo '**Windows (PowerShell)**'
    echo
    echo '```powershell'
    echo "Get-FileHash dologctl-windows-x86_64.exe -Algorithm SHA256"
    echo '```'
    echo
    echo 'The output hash must match the line for the same file in'
    echo '`checksums-sha256.txt`. For a supply-chain audit, see the'
    echo '[Security Whitepaper](https://github.com/Nekolio/DoLogger/blob/main/Docs/en_US/guides/SecurityWhitepaper.md).'
    echo
    echo '### Documentation'
    echo
    echo '- [README](https://github.com/Nekolio/DoLogger#readme)'
    echo '- [Wiki](https://github.com/Nekolio/DoLogger/wiki)'
    echo
    echo '---'
    echo
    echo '<a id="chinese"></a>'
    echo
    echo '## 中文'
    echo
    echo 'DoLogger 是一个跨平台、高安全性的日志引擎，为需要签名、防篡改'
    echo '审计日志的应用而设计。'
    echo
    echo '<!--BENCH_ZH-->'
    echo
    echo '### 更新日志'
    echo
    echo "**${CHANGELOG_ZH}**"
    echo
    printf '%s\n' "${COMMITS}"
    echo
    echo '### 快速下载'
    echo
    echo '所有资产位于本页下方的附件列表中。以下命令获取最常用平台的'
    echo 'CLI 二进制，其余平台见资产表格。'
    echo
    echo '**Linux（x86_64）**'
    echo
    echo '```bash'
    echo "curl -fLO ${DL_BASE}/dologctl-linux-x86_64"
    echo 'chmod +x dologctl-linux-x86_64'
    echo '```'
    echo
    echo '**Linux（aarch64）**'
    echo
    echo '```bash'
    echo "curl -fLO ${DL_BASE}/dologctl-linux-aarch64"
    echo 'chmod +x dologctl-linux-aarch64'
    echo '```'
    echo
    echo '**macOS（Apple Silicon）**'
    echo
    echo '```bash'
    echo "curl -fLO ${DL_BASE}/dologctl-macos-aarch64"
    echo 'chmod +x dologctl-macos-aarch64'
    echo '```'
    echo
    echo '**Windows（x86_64）** — PowerShell：'
    echo
    echo '```powershell'
    echo "curl.exe -fLO ${DL_BASE}/dologctl-windows-x86_64.exe"
    echo '```'
    echo
    echo '### 资产命名规则'
    echo
    echo 'CLI 二进制遵循 `dologctl-<os>-<arch>` 命名。Windows 以 `.exe` 结尾；'
    echo 'Linux 与 macOS 不带扩展名（POSIX 惯例）。每个平台同时附赠其核心库'
    echo '（`dologger_core`），`checksums-sha256.txt` 记录所有资产的 SHA-256。'
    echo
    echo '| 操作系统 | 架构 | CLI 资产 | 核心库 |'
    echo '|:-:|:-:|:-:|:-:|'
    echo '| Linux | x86_64 | `dologctl-linux-x86_64` | `libdologger_core.so` |'
    echo '| Linux | aarch64 | `dologctl-linux-aarch64` | `libdologger_core.so` |'
    echo '| Linux | i686（32 位） | `dologctl-linux-i686` | `libdologger_core.so` |'
    echo '| Linux | armv7（32 位） | `dologctl-linux-armv7` | `libdologger_core.so` |'
    echo '| Linux | riscv64 | `dologctl-linux-riscv64` | `libdologger_core.so` |'
    echo '| Windows | x86_64 | `dologctl-windows-x86_64.exe` | `dologger_core.dll` |'
    echo '| Windows | aarch64 | `dologctl-windows-aarch64.exe` | `dologger_core.dll` |'
    echo '| Windows | i686（32 位） | `dologctl-windows-i686.exe` | `dologger_core.dll` |'
    echo '| macOS | aarch64（Apple Silicon） | `dologctl-macos-aarch64` | `libdologger_core.dylib` |'
    echo '| macOS | x86_64（Intel） | `dologctl-macos-x86_64` | `libdologger_core.dylib` |'
    echo
    echo '### 校验下载'
    echo
    echo '运行前，请将下载文件的 SHA-256 与 `checksums-sha256.txt` 中对应条目比对。'
    echo
    echo '**Linux / macOS**'
    echo
    echo '```bash'
    echo "grep 'dologctl-linux-x86_64' checksums-sha256.txt | sha256sum -c -"
    echo '```'
    echo
    echo '**Windows（PowerShell）**'
    echo
    echo '```powershell'
    echo "Get-FileHash dologctl-windows-x86_64.exe -Algorithm SHA256"
    echo '```'
    echo
    echo '输出的哈希必须与 `checksums-sha256.txt` 中同一文件的条目一致。'
    echo '供应链审计请参阅[安全白皮书](https://github.com/Nekolio/DoLogger/blob/main/Docs/zh_CN/guides/SecurityWhitepaper.md)。'
    echo
    echo '### 文档'
    echo
    echo '- [README](https://github.com/Nekolio/DoLogger#readme)'
    echo '- [Wiki](https://github.com/Nekolio/DoLogger/wiki)'
    echo
} > "$BODY"

cat "$BODY"
rm -f "$BODY"
