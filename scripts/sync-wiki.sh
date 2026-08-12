#!/usr/bin/env bash
# sync-wiki.sh — publish Docs/ to the GitHub wiki.
#
# The wiki is a separate git repository (https://github.com/<repo>.wiki.git).
# GitHub wikis only render .md files at the repository ROOT as pages — files
# in subdirectories are served as raw attachments. This script therefore
# FLATTENS the docs tree into flat page names (en_US-QuickStart.md,
# zh_CN-guides-PluginDevelopmentGuide.md, ...) and rewrites every relative
# markdown link to the flattened names, so cross-links and the language
# switch headers keep working on the wiki.
#
# Usage:
#   CI:      scripts/sync-wiki.sh     (uses $GITHUB_TOKEN / $GITHUB_REPOSITORY)
#   Local:   scripts/sync-wiki.sh     (uses `gh auth token`)
#
# Exit codes: 0 = wiki updated or already in sync or not yet initialized.
#             1 = real failure worth surfacing.
set -euo pipefail

REPO="${GITHUB_REPOSITORY:-Nekolio/DoLogger}"
BRANCH="${GITHUB_REF_NAME:-main}"
SHA="${GITHUB_SHA:-$(git rev-parse HEAD 2>/dev/null || echo local)}"

TOKEN="${GITHUB_TOKEN:-}"
if [ -z "$TOKEN" ] && command -v gh >/dev/null 2>&1; then
    TOKEN="$(gh auth token 2>/dev/null || true)"
fi

if [ -z "$TOKEN" ]; then
    echo "::warning::No GitHub token available — skipping wiki sync."
    exit 0
fi

WIKI_URL="https://x-access-token:${TOKEN}@github.com/${REPO}.wiki.git"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ── 1. Clone (or initialize) the wiki repository ───────────────────
if ! git clone --quiet --depth 1 "$WIKI_URL" "$WORK/wiki" 2>/dev/null; then
    # The wiki repository does not exist until its first page is created
    # (GitHub offers no API/push path to create it). Surface friendly
    # instructions and skip.
    echo "::warning::Wiki repository not found. Create the first wiki page once"
    echo "::warning::at https://github.com/${REPO}/wiki (e.g. a 'Home' page), then"
    echo "::warning::re-run this workflow — afterwards it stays in sync automatically."
    exit 0
fi

git -C "$WORK/wiki" config user.name  "${GITHUB_ACTOR:-github-actions[bot]}"
git -C "$WORK/wiki" config user.email "github-actions[bot]@users.noreply.github.com"

# ── 2. Flatten Docs/ into wiki-root pages with rewritten links ─────
# Clean slate: drop every previously synced page (root .md pages and the
# old en_US/ zh_CN/ folders), keep _Sidebar/_Footer for regeneration.
find "$WORK/wiki" -mindepth 1 -maxdepth 1 -name '*.md' -delete
rm -rf "$WORK/wiki/en_US" "$WORK/wiki/zh_CN"

rewrite_links() {
    python3 - "$1" "$2" <<'PY'
import re, sys, os
path, page = sys.argv[1], sys.argv[2]  # page: Docs-relative, e.g. en_US/guides/PluginDevelopmentGuide.md
with open(path, encoding='utf-8') as f:
    text = f.read()

page_dir = os.path.dirname(page)

def map_link(m):
    target = m.group(1)
    # skip absolute URLs, mailto, images, and non-.md targets
    if target.startswith(('http://', 'https://', 'mailto:', '#')):
        return m.group(0)
    if '](' in m.group(0) and m.group(0).startswith('!['):
        return m.group(0)
    anchor = ''
    body = target
    if '#' in target:
        body, anchor = target.split('#', 1)
        anchor = '#' + anchor
    if not body.endswith('.md'):
        return m.group(0)
    resolved = os.path.normpath(os.path.join(page_dir, body)).replace('\\', '/')
    parts = resolved.split('/')
    if len(parts) < 2:
        return m.group(0)
    lang, rel = parts[0], '/'.join(parts[1:])
    if rel.endswith('.md'):
        rel = rel[:-3]
    return f']({lang}-{rel.replace("/", "-")}{anchor})'

text = re.sub(r'\]\(([^)]+)\)', map_link, text)
with open(path, 'w', encoding='utf-8', newline='\n') as f:
    f.write(text)
PY
}

for lang in en_US zh_CN; do
    while IFS= read -r src; do
        rel="${src#Docs/${lang}/}"                 # e.g. guides/PluginDevelopmentGuide.md
        page="${lang}/${rel}"                      # Docs-relative page path
        flat="$(printf '%s' "${lang}-${rel}" | tr '/' '-')"   # en_US-guides-PluginDevelopmentGuide.md
        cp "$src" "$WORK/wiki/$flat"
        rewrite_links "$WORK/wiki/$flat" "$page"
    done < <(find "Docs/${lang}" -name '*.md' | sort)
done

# ── 3. Page title mapping (flat filename → human title) ────────────
title_of() {
    local base="$1"   # e.g. guides-PluginDevelopmentGuide
    base="${base##*-guides-}"; base="${base##*-}"; base="${base%%.md}"
    case "$base" in
        QuickStart)               echo "Quick Start" ;;
        IntegrationGuide)         echo "Integration Guide" ;;
        ArchitectureReference)    echo "Architecture Reference" ;;
        OperationsAndSecurity)    echo "Operations & Security" ;;
        PluginDevelopmentQuickStart) echo "Plugin Development QuickStart" ;;
        OfficialPluginRoadmap)    echo "Official Plugin Roadmap" ;;
        AdapterDevelopmentGuide)  echo "Adapter Development Guide" ;;
        DologctlCommandReference) echo "dologctl Command Reference" ;;
        ExtendedPluginTypeGuide)  echo "Extended Plugin Type Guide" ;;
        HostIntegrationGuide)     echo "Host Integration Guide" ;;
        OperationsManual)         echo "Operations Manual" ;;
        PerformanceBenchmarkGuide) echo "Performance Benchmark Guide" ;;
        PerformanceTuningGuide)   echo "Performance Tuning Guide" ;;
        PluginDevelopmentGuide)   echo "Plugin Development Guide" ;;
        SecurityDevelopmentSpec)  echo "Security Development Spec" ;;
        SecurityWhitepaper)       echo "Security Whitepaper" ;;
        VersioningAndDeprecation) echo "Versioning & Deprecation" ;;
        *) echo "$base" ;;
    esac
}

zh_title_of() {
    local base="$1"
    base="${base##*-guides-}"; base="${base##*-}"; base="${base%%.md}"
    case "$base" in
        QuickStart)               echo "快速开始" ;;
        IntegrationGuide)         echo "集成指南" ;;
        ArchitectureReference)    echo "架构参考" ;;
        OperationsAndSecurity)    echo "运维与安全" ;;
        PluginDevelopmentQuickStart) echo "插件开发快速入门" ;;
        OfficialPluginRoadmap)    echo "官方插件路线图" ;;
        AdapterDevelopmentGuide)  echo "适配器开发指南" ;;
        DologctlCommandReference) echo "dologctl 命令参考" ;;
        ExtendedPluginTypeGuide)  echo "扩展插件类型开发指南" ;;
        HostIntegrationGuide)     echo "宿主集成手册" ;;
        OperationsManual)         echo "运维手册" ;;
        PerformanceBenchmarkGuide) echo "性能基准测试指南" ;;
        PerformanceTuningGuide)   echo "性能调优指南" ;;
        PluginDevelopmentGuide)   echo "插件开发指南" ;;
        SecurityDevelopmentSpec)  echo "安全开发规范" ;;
        SecurityWhitepaper)       echo "安全白皮书" ;;
        VersioningAndDeprecation) echo "版本与废弃策略" ;;
        *) echo "$base" ;;
    esac
}

# ── 4. _Sidebar.md — grouped, proper titles per language ───────────
SIDEBAR="$WORK/wiki/_Sidebar.md"
cat > "$SIDEBAR" <<'EOF'
# DoLogger Wiki

> 由仓库 `Docs/` 经 Wiki Sync workflow 自动生成 — 请勿在此编辑。
> Auto-generated from the repository `Docs/` — do not edit here.

EOF
{
    echo "## 📖 中文文档"
    echo
    for f in $(cd "$WORK/wiki" && ls zh_CN-*.md | sort); do
        printf -- '- [%s](%s)\n' "$(zh_title_of "$f")" "${f%.md}"
    done
    echo
    echo "## 📚 English"
    echo
    for f in $(cd "$WORK/wiki" && ls en_US-*.md | sort); do
        printf -- '- [%s](%s)\n' "$(title_of "$f")" "${f%.md}"
    done
} >> "$SIDEBAR"

# ── 5. Home.md (中文优先) + English-Home.md (English-first) ────────
cat > "$WORK/wiki/Home.md" <<EOF
# 🔐 DoLogger Wiki

> 🌐 **语言 / Language**: 中文 · [English](English-Home)

*跨平台、高安全日志引擎 — 像一本书一样阅读的技术文档。*

[![CI](https://img.shields.io/github/actions/workflow/status/${REPO}/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/${REPO}/actions)
[![Stars](https://img.shields.io/github/stars/${REPO}?style=flat-square&color=yellow)](https://github.com/${REPO}/stargazers)
[![License](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue?style=flat-square)](https://github.com/${REPO}/blob/main/LICENSE-APACHE)

> [!NOTE]
> 本 wiki 由仓库 \`Docs/\` 目录经 Wiki Sync workflow **自动生成** —— 请勿在此直接编辑。修改源文档并推送到 main 即可自动更新。

## 📖 目录 · 按阅读顺序

| # | 章节 | English |
|:-:|:-:|:-:|
| 1 | [快速开始](zh_CN-QuickStart) | [QuickStart](en_US-QuickStart) |
| 2 | [集成指南](zh_CN-IntegrationGuide) | [Integration Guide](en_US-IntegrationGuide) |
| 3 | [架构参考](zh_CN-ArchitectureReference) | [Architecture Reference](en_US-ArchitectureReference) |
| 4 | [运维与安全](zh_CN-OperationsAndSecurity) | [Operations & Security](en_US-OperationsAndSecurity) |
| 5 | [插件开发快速入门](zh_CN-PluginDevelopmentQuickStart) | [Plugin Development QuickStart](en_US-PluginDevelopmentQuickStart) |
| 6 | [安全白皮书](zh_CN-guides-SecurityWhitepaper) | [Security Whitepaper](en_US-guides-SecurityWhitepaper) |
| 7 | [dologctl 命令参考](zh_CN-guides-DologctlCommandReference) | [Command Reference](en_US-guides-DologctlCommandReference) |
| 8 | [版本与废弃策略](zh_CN-guides-VersioningAndDeprecation) | [Versioning & Deprecation](en_US-guides-VersioningAndDeprecation) |

## 📚 进阶章节

| 章节 | English |
|:-:|:-:|
| [宿主集成手册](zh_CN-guides-HostIntegrationGuide) | [Host Integration Guide](en_US-guides-HostIntegrationGuide) |
| [适配器开发指南](zh_CN-guides-AdapterDevelopmentGuide) | [Adapter Development Guide](en_US-guides-AdapterDevelopmentGuide) |
| [扩展插件类型开发指南](zh_CN-guides-ExtendedPluginTypeGuide) | [Extended Plugin Type Guide](en_US-guides-ExtendedPluginTypeGuide) |
| [安全开发规范](zh_CN-guides-SecurityDevelopmentSpec) | [Security Development Spec](en_US-guides-SecurityDevelopmentSpec) |
| [运维手册](zh_CN-guides-OperationsManual) | [Operations Manual](en_US-guides-OperationsManual) |
| [性能调优指南](zh_CN-guides-PerformanceTuningGuide) | [Performance Tuning Guide](en_US-guides-PerformanceTuningGuide) |
| [性能基准测试指南](zh_CN-guides-PerformanceBenchmarkGuide) | [Performance Benchmark Guide](en_US-guides-PerformanceBenchmarkGuide) |
| [官方插件路线图](zh_CN-OfficialPluginRoadmap) | [Official Plugin Roadmap](en_US-OfficialPluginRoadmap) |

## 📎 附录

- [文档总索引](https://github.com/${REPO}/blob/main/Docs/README.md)
- [GitHub Releases](https://github.com/${REPO}/releases) —— 每个 release 页面即该版本的 changelog
- [Issue Tracker](https://github.com/${REPO}/issues)
- [安全政策](https://github.com/${REPO}/blob/main/SECURITY.md)

---
*Synced from [${REPO}](https://github.com/${REPO}) \`Docs/\` — Wiki Sync workflow*
EOF

cat > "$WORK/wiki/English-Home.md" <<EOF
# 🔐 DoLogger Wiki

> 🌐 **Language / 语言**: English · [中文](Home)

*Cross-platform, high-security logging engine — documentation you can read like a book.*

[![CI](https://img.shields.io/github/actions/workflow/status/${REPO}/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/${REPO}/actions)
[![Stars](https://img.shields.io/github/stars/${REPO}?style=flat-square&color=yellow)](https://github.com/${REPO}/stargazers)
[![License](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue?style=flat-square)](https://github.com/${REPO}/blob/main/LICENSE-APACHE)

> [!NOTE]
> This wiki is **auto-generated** from the repository's \`Docs/\` directory by the Wiki Sync workflow — do not edit here. Push changes to main and they sync automatically.

## 📖 Table of Contents — in reading order

| # | Chapter | 中文 |
|:-:|:-:|:-:|
| 1 | [QuickStart](en_US-QuickStart) | [快速开始](zh_CN-QuickStart) |
| 2 | [Integration Guide](en_US-IntegrationGuide) | [集成指南](zh_CN-IntegrationGuide) |
| 3 | [Architecture Reference](en_US-ArchitectureReference) | [架构参考](zh_CN-ArchitectureReference) |
| 4 | [Operations & Security](en_US-OperationsAndSecurity) | [运维与安全](zh_CN-OperationsAndSecurity) |
| 5 | [Plugin Development QuickStart](en_US-PluginDevelopmentQuickStart) | [插件开发快速入门](zh_CN-PluginDevelopmentQuickStart) |
| 6 | [Security Whitepaper](en_US-guides-SecurityWhitepaper) | [安全白皮书](zh_CN-guides-SecurityWhitepaper) |
| 7 | [dologctl Command Reference](en_US-guides-DologctlCommandReference) | [命令参考](zh_CN-guides-DologctlCommandReference) |
| 8 | [Versioning & Deprecation](en_US-guides-VersioningAndDeprecation) | [版本与废弃策略](zh_CN-guides-VersioningAndDeprecation) |

## 📚 Advanced Chapters

| Chapter | 中文 |
|:-:|:-:|
| [Host Integration Guide](en_US-guides-HostIntegrationGuide) | [宿主集成手册](zh_CN-guides-HostIntegrationGuide) |
| [Adapter Development Guide](en_US-guides-AdapterDevelopmentGuide) | [适配器开发指南](zh_CN-guides-AdapterDevelopmentGuide) |
| [Extended Plugin Type Guide](en_US-guides-ExtendedPluginTypeGuide) | [扩展插件类型开发指南](zh_CN-guides-ExtendedPluginTypeGuide) |
| [Security Development Spec](en_US-guides-SecurityDevelopmentSpec) | [安全开发规范](zh_CN-guides-SecurityDevelopmentSpec) |
| [Operations Manual](en_US-guides-OperationsManual) | [运维手册](zh_CN-guides-OperationsManual) |
| [Performance Tuning Guide](en_US-guides-PerformanceTuningGuide) | [性能调优指南](zh_CN-guides-PerformanceTuningGuide) |
| [Performance Benchmark Guide](en_US-guides-PerformanceBenchmarkGuide) | [性能基准测试指南](zh_CN-guides-PerformanceBenchmarkGuide) |
| [Official Plugin Roadmap](en_US-OfficialPluginRoadmap) | [官方插件路线图](zh_CN-OfficialPluginRoadmap) |

## 📎 Appendix

- [Documentation Index](https://github.com/${REPO}/blob/main/Docs/README.md)
- [GitHub Releases](https://github.com/${REPO}/releases) — each release page is that version's changelog
- [Issue Tracker](https://github.com/${REPO}/issues)
- [Security Policy](https://github.com/${REPO}/blob/main/SECURITY.md)

---
*Synced from [${REPO}](https://github.com/${REPO}) \`Docs/\` — Wiki Sync workflow*
EOF

# ── 6. Commit and push (skip when nothing changed) ─────────────────
git -C "$WORK/wiki" add -A
if git -C "$WORK/wiki" diff --cached --quiet; then
    echo "Wiki already in sync — nothing to push."
    exit 0
fi

git -C "$WORK/wiki" commit --quiet -m "docs: sync wiki from ${REPO}@${SHA}" || {
    echo "::warning::Wiki commit failed (empty?) — skipping push."
    exit 0
}

# Retry once on concurrent-push races (another run may land between our
# fetch and push); rebase on top and push again.
if ! git -C "$WORK/wiki" push --quiet origin HEAD:master; then
    git -C "$WORK/wiki" pull --quiet --rebase origin master || true
    git -C "$WORK/wiki" push --quiet origin HEAD:master
fi

echo "Wiki updated: $(git -C "$WORK/wiki" log -1 --pretty=%s)"
