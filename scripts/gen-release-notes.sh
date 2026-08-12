#!/usr/bin/env bash
# gen-release-notes.sh — assemble a GitHub Release body: the release page IS
# the changelog, fully bilingual (EN + 中文), with a benchmark section that
# the workflow fills in from the per-release benchmark job.
#
# Usage:
#   scripts/gen-release-notes.sh <tag> > release-notes.md
#
# Requires a full git history (actions/checkout with fetch-depth: 0).
# The emitted body contains a `<!--BENCHMARK_SECTION-->` marker line; the
# release workflow replaces it with the benchmark job's output.
set -euo pipefail

TAG="${1:?usage: gen-release-notes.sh <tag>}"
BODY="$(mktemp)"

# ── 1. Commit list since the previous tag ──────────────────────────
PREV_TAG="$(git describe --tags --abbrev=0 "${TAG}^" 2>/dev/null || true)"
if [ -n "$PREV_TAG" ]; then
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${PREV_TAG}..${TAG}" 2>/dev/null | head -100 || true)"
else
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${TAG}" 2>/dev/null | head -100 || true)"
fi

{
    echo "# DoLogger ${TAG}"
    echo
    echo '> 🌐 **Language / 语言**: this page is bilingual — English sections first, then 中文段落。本发布页即该版本的 changelog。'
    echo
    echo '<!--BENCHMARK_SECTION-->'
    echo
    echo '## 📝 Changelog · 更新日志'
    echo
    if [ -n "$PREV_TAG" ]; then
        echo "**Changes since ${PREV_TAG} · 自 ${PREV_TAG} 以来的变更:**"
    else
        echo '**Initial release · 首次发布:**'
    fi
    echo
    printf '%s\n' "${COMMITS:-_(no commits found — check that the workflow checks out full history)_}"
    echo
    echo '## 🚀 Usage · 使用方法'
    echo
    echo '**English**'
    echo
    echo '```bash'
    echo '# 1. Download the binary for your platform (naming table below)'
    echo '# 2. Verify integrity:'
    echo 'sha256sum -c checksums-sha256.txt'
    echo
    echo '# 3. Run:'
    echo 'chmod +x dologctl-linux-x86_64 && ./dologctl-linux-x86_64 init --template dev'
    echo './dologctl-linux-x86_64 run --config dologger.toml'
    echo
    echo '# From source:'
    echo 'git clone https://github.com/Nekolio/DoLogger.git && cd DoLogger'
    echo 'cargo build --release'
    echo '```'
    echo
    echo '**中文**'
    echo
    echo '```bash'
    echo '# 1. 下载对应平台的二进制文件（命名规则见下表）'
    echo '# 2. 校验完整性:'
    echo 'sha256sum -c checksums-sha256.txt'
    echo
    echo '# 3. 运行:'
    echo 'chmod +x dologctl-linux-x86_64 && ./dologctl-linux-x86_64 init --template dev'
    echo './dologctl-linux-x86_64 run --config dologger.toml'
    echo
    echo '# 从源码构建:'
    echo 'git clone https://github.com/Nekolio/DoLogger.git && cd DoLogger'
    echo 'cargo build --release'
    echo '```'
    echo
    echo 'See the [README](https://github.com/Nekolio/DoLogger#readme) and the'
    echo '[wiki](https://github.com/Nekolio/DoLogger/wiki) for full documentation.'
    echo '完整文档见 [README](https://github.com/Nekolio/DoLogger#readme) 与 [wiki](https://github.com/Nekolio/DoLogger/wiki)。'
    echo
    echo '## 📦 Asset Naming · 资产命名规则'
    echo
    echo '| Asset · 资产 | Pattern · 规则 |'
    echo '|:-:|:-:|'
    echo '| CLI binary · 命令行工具 | `dologctl-<os>-<arch>` (`.exe` on Windows · Windows 带 `.exe`) |'
    echo '| Core library · 核心库 | `libdologger_core.so` (Linux), `libdologger_core.dylib` (macOS), `dologger_core.dll` (Windows) |'
    echo '| Checksums · 校验和 | `checksums-sha256.txt` — SHA-256 of every asset · 每个资产的 SHA-256 |'
    echo '| Git tags · 标签 | `vMAJOR.MINOR.PATCH`; pre-releases use `-alpha.N`, `-beta.N`, `-rc.N` · 预发布用 `-alpha.N`、`-beta.N`、`-rc.N` |'
    echo
    echo '## 🔒 Integrity Verification · 完整性校验'
    echo
    echo '**English:** every asset is listed in `checksums-sha256.txt`. Verify the'
    echo 'download against its checksum before running. For supply-chain audits,'
    echo 'see the [Security Whitepaper](https://github.com/Nekolio/DoLogger/blob/main/Docs/en_US/guides/SecurityWhitepaper.md).'
    echo
    echo '**中文:** 每个资产都列于 `checksums-sha256.txt`,运行前请核对校验和。供应链审计请参阅[安全白皮书](https://github.com/Nekolio/DoLogger/blob/main/Docs/zh_CN/guides/SecurityWhitepaper.md)。'
    echo
} > "$BODY"

cat "$BODY"
rm -f "$BODY"
