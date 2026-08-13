#!/usr/bin/env bash
# gen-release-notes.sh — assemble the GitHub Release body.
#
# The release page is the changelog for its version. The body is written in
# two self-contained sections — English, then Chinese — switched via the
# language bar at the top, with a contents index linking to every section
# and one shared changelog at the end (the commit list is language-neutral,
# only its heading differs). The release workflow fills the benchmark
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
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${PREV_TAG}..${TAG}" 2>/dev/null | head -40 || true)"
    CHANGELOG_TITLE="Changes since ${PREV_TAG} / 自 ${PREV_TAG} 以来的变更"
else
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${TAG}" 2>/dev/null | head -40 || true)"
    CHANGELOG_TITLE="Initial release / 首次发布"
fi
COMMITS="${COMMITS:-_(no commits found — does the workflow check out full history?)_}"

DL_BASE="https://github.com/Nekolio/DoLogger/releases/download/${TAG}"

# dl <asset> — markdown link to a release asset on this release.
dl() { printf '[%s](%s/%s)' "$1" "$DL_BASE" "$1"; }

{
    echo "# DoLogger ${TAG}"
    echo
    echo "[English](#english) | [中文](#chinese)"
    echo
    echo '<a id="contents"></a>'
    echo
    echo '**Contents / 目录**'
    echo
    echo '- [Benchmarks](#benchmarks-en) / [跑分结果](#benchmarks-zh)'
    echo '- [Downloads](#downloads-en) / [下载](#downloads-zh)'
    echo '- [Verify Your Download](#verify-en) / [校验下载](#verify-zh)'
    echo '- [Documentation](#documentation-en) / [文档](#documentation-zh)'
    echo '- [Changelog / 更新日志](#changelog)'
    echo
    echo '---'
    echo
    echo '<a id="english"></a>'
    echo
    echo '## English'
    echo
    echo 'DoLogger is a cross-platform, high-security logging engine for'
    echo 'applications that need signed, tamper-evident audit logs.'
    echo
    echo '<a id="benchmarks-en"></a>'
    echo
    echo '<!--BENCH_EN-->'
    echo
    echo '<a id="downloads-en"></a>'
    echo
    echo '### Downloads'
    echo
    echo 'All assets follow `dologctl-<version>-<os>-<arch>`: Windows ends in'
    echo '`.exe`, Linux/macOS carry no extension (POSIX), and the core library'
    echo 'keeps the linker `lib` prefix. Click a cell to download:'
    echo
    echo '| OS | Architecture | CLI | Core library |'
    echo '|:-:|:-:|:-:|:-:|'
    echo "| Linux | x86_64 | $(dl dologctl-${TAG}-linux-x86_64) | $(dl libdologger_core-${TAG}-linux-x86_64.so) |"
    echo "| Linux | aarch64 | $(dl dologctl-${TAG}-linux-aarch64) | $(dl libdologger_core-${TAG}-linux-aarch64.so) |"
    echo "| Linux | i686 (32-bit) | $(dl dologctl-${TAG}-linux-i686) | $(dl libdologger_core-${TAG}-linux-i686.so) |"
    echo "| Linux | armv7 (32-bit) | $(dl dologctl-${TAG}-linux-armv7) | $(dl libdologger_core-${TAG}-linux-armv7.so) |"
    echo "| Linux | riscv64 | $(dl dologctl-${TAG}-linux-riscv64) | $(dl libdologger_core-${TAG}-linux-riscv64.so) |"
    echo "| Windows | x86_64 | $(dl dologctl-${TAG}-windows-x86_64.exe) | $(dl dologger_core-${TAG}-windows-x86_64.dll) |"
    echo "| Windows | aarch64 | $(dl dologctl-${TAG}-windows-aarch64.exe) | $(dl dologger_core-${TAG}-windows-aarch64.dll) |"
    echo "| Windows | i686 (32-bit) | $(dl dologctl-${TAG}-windows-i686.exe) | $(dl dologger_core-${TAG}-windows-i686.dll) |"
    echo "| macOS | aarch64 (Apple Silicon) | $(dl dologctl-${TAG}-macos-aarch64) | $(dl libdologger_core-${TAG}-macos-aarch64.dylib) |"
    echo "| macOS | x86_64 (Intel) | $(dl dologctl-${TAG}-macos-x86_64) | $(dl libdologger_core-${TAG}-macos-x86_64.dylib) |"
    echo
    echo "SHA-256 of every asset: [checksums-sha256.txt](${DL_BASE}/checksums-sha256.txt)"
    echo
    echo '<a id="verify-en"></a>'
    echo
    echo '### Verify Your Download'
    echo
    echo 'Compare the SHA-256 of a downloaded file against its entry in'
    echo '`checksums-sha256.txt` before running it.'
    echo
    echo '**Linux / macOS**'
    echo
    echo '```bash'
    echo "grep 'dologctl-${TAG}-linux-x86_64' checksums-sha256.txt | sha256sum -c -"
    echo '```'
    echo
    echo '**Windows (PowerShell)**'
    echo
    echo '```powershell'
    echo "Get-FileHash dologctl-${TAG}-windows-x86_64.exe -Algorithm SHA256"
    echo '```'
    echo
    echo 'The output hash must match the line for the same file in'
    echo '`checksums-sha256.txt`. For a supply-chain audit, see the'
    echo '[Security Whitepaper](https://github.com/Nekolio/DoLogger/blob/main/Docs/en_US/guides/SecurityWhitepaper.md).'
    echo
    echo '<a id="documentation-en"></a>'
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
    echo '<a id="benchmarks-zh"></a>'
    echo
    echo '<!--BENCH_ZH-->'
    echo
    echo '<a id="downloads-zh"></a>'
    echo
    echo '### 下载'
    echo
    echo '全部资产按 `dologctl-<版本>-<os>-<arch>` 命名：Windows 以 `.exe`'
    echo '结尾，Linux/macOS 不带扩展名，核心库保留 `lib` 前缀。'
    echo '点击单元格即可下载：'
    echo
    echo '| 操作系统 | 架构 | CLI | 核心库 |'
    echo '|:-:|:-:|:-:|:-:|'
    echo "| Linux | x86_64 | $(dl dologctl-${TAG}-linux-x86_64) | $(dl libdologger_core-${TAG}-linux-x86_64.so) |"
    echo "| Linux | aarch64 | $(dl dologctl-${TAG}-linux-aarch64) | $(dl libdologger_core-${TAG}-linux-aarch64.so) |"
    echo "| Linux | i686（32 位） | $(dl dologctl-${TAG}-linux-i686) | $(dl libdologger_core-${TAG}-linux-i686.so) |"
    echo "| Linux | armv7（32 位） | $(dl dologctl-${TAG}-linux-armv7) | $(dl libdologger_core-${TAG}-linux-armv7.so) |"
    echo "| Linux | riscv64 | $(dl dologctl-${TAG}-linux-riscv64) | $(dl libdologger_core-${TAG}-linux-riscv64.so) |"
    echo "| Windows | x86_64 | $(dl dologctl-${TAG}-windows-x86_64.exe) | $(dl dologger_core-${TAG}-windows-x86_64.dll) |"
    echo "| Windows | aarch64 | $(dl dologctl-${TAG}-windows-aarch64.exe) | $(dl dologger_core-${TAG}-windows-aarch64.dll) |"
    echo "| Windows | i686（32 位） | $(dl dologctl-${TAG}-windows-i686.exe) | $(dl dologger_core-${TAG}-windows-i686.dll) |"
    echo "| macOS | aarch64（Apple Silicon） | $(dl dologctl-${TAG}-macos-aarch64) | $(dl libdologger_core-${TAG}-macos-aarch64.dylib) |"
    echo "| macOS | x86_64（Intel） | $(dl dologctl-${TAG}-macos-x86_64) | $(dl libdologger_core-${TAG}-macos-x86_64.dylib) |"
    echo
    echo "所有资产的 SHA-256：[checksums-sha256.txt](${DL_BASE}/checksums-sha256.txt)"
    echo
    echo '<a id="verify-zh"></a>'
    echo
    echo '### 校验下载'
    echo
    echo '运行前，请将下载文件的 SHA-256 与 `checksums-sha256.txt` 中对应条目比对。'
    echo
    echo '**Linux / macOS**'
    echo
    echo '```bash'
    echo "grep 'dologctl-${TAG}-linux-x86_64' checksums-sha256.txt | sha256sum -c -"
    echo '```'
    echo
    echo '**Windows（PowerShell）**'
    echo
    echo '```powershell'
    echo "Get-FileHash dologctl-${TAG}-windows-x86_64.exe -Algorithm SHA256"
    echo '```'
    echo
    echo '输出的哈希必须与 `checksums-sha256.txt` 中同一文件的条目一致。'
    echo '供应链审计请参阅[安全白皮书](https://github.com/Nekolio/DoLogger/blob/main/Docs/zh_CN/guides/SecurityWhitepaper.md)。'
    echo
    echo '<a id="documentation-zh"></a>'
    echo
    echo '### 文档'
    echo
    echo '- [README](https://github.com/Nekolio/DoLogger#readme)'
    echo '- [Wiki](https://github.com/Nekolio/DoLogger/wiki)'
    echo
    echo '---'
    echo
    echo '<a id="changelog"></a>'
    echo
    echo '## Changelog / 更新日志'
    echo
    echo "**${CHANGELOG_TITLE}**"
    echo
    printf '%s\n' "${COMMITS}"
    echo
} > "$BODY"

cat "$BODY"
rm -f "$BODY"
