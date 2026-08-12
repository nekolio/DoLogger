#!/usr/bin/env bash
# sync-wiki.sh — publish Docs/ to the GitHub wiki.
#
# The wiki is a separate git repository (https://github.com/<repo>.wiki.git).
# This script mirrors the Docs/ tree into it so every documentation change on
# main is reflected on the wiki automatically. Relative cross-links inside the
# docs keep working because the directory layout is preserved.
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
    # The wiki repository does not exist until its first page is created.
    # GITHUB_TOKEN cannot create it — surface friendly instructions and skip.
    echo "::warning::Wiki repository not found. Create the first wiki page once"
    echo "::warning::at https://github.com/${REPO}/wiki (e.g. a 'Home' page), then"
    echo "::warning::re-run this workflow — afterwards it stays in sync automatically."
    exit 0
fi

git -C "$WORK/wiki" config user.name  "${GITHUB_ACTOR:-github-actions[bot]}"
git -C "$WORK/wiki" config user.email "github-actions[bot]@users.noreply.github.com"

# ── 2. Mirror the Docs/ tree (deletions included) ──────────────────
rm -rf "$WORK/wiki/en_US" "$WORK/wiki/zh_CN"
mkdir -p "$WORK/wiki/en_US" "$WORK/wiki/zh_CN"
cp -r Docs/en_US/. "$WORK/wiki/en_US/"
cp -r Docs/zh_CN/. "$WORK/wiki/zh_CN/"

# ── 3. Generate Home.md and _Sidebar.md ────────────────────────────
generate_sidebar() {
    local lang_dir="$1" lang_title="$2" out="$3"
    {
        echo "**${lang_title}**"
        echo
        (cd "$lang_dir" && find . -name '*.md' | sort) | while IFS= read -r f; do
            path="${f#./}"
            title="$(basename "$path" .md)"
            echo "- [${title}](${lang_dir#*/wiki/}/${path})"
        done
    } >> "$out"
}

SIDEBAR="$WORK/wiki/_Sidebar.md"
cat > "$SIDEBAR" <<'EOF'
# DoLogger Wiki

> Auto-generated from the repository's `Docs/` directory by the Wiki Sync workflow — do not edit pages here; edit the source docs and push to main.

EOF
echo >> "$SIDEBAR"
generate_sidebar "$WORK/wiki/en_US" "📚 English" "$SIDEBAR"
echo >> "$SIDEBAR"
generate_sidebar "$WORK/wiki/zh_CN" "📚 中文" "$SIDEBAR"

cat > "$WORK/wiki/Home.md" <<EOF
# 🔐 DoLogger Wiki

> 🌐 **语言 / Language**: [English](../../wiki/en_US) · [中文](../../wiki/zh_CN)

This wiki is generated automatically from the [DoLogger repository](https://github.com/${REPO}) \`Docs/\` directory.

| Language | Directory |
|:-:|:-:|
| English | [en_US](en_US/) |
| 中文 | [zh_CN](zh_CN/) |

See also: [README](https://github.com/${REPO}#readme) · [CHANGELOG](https://github.com/${REPO}/blob/${BRANCH}/CHANGELOG.md) · [Issues](https://github.com/${REPO}/issues)
EOF

cat > "$WORK/wiki/_Footer.md" <<EOF
---
*Synced from [${REPO}](https://github.com/${REPO}) Docs/ at \`${SHA}\` — Wiki Sync workflow.*
EOF

# ── 4. Commit and push (skip when nothing changed) ─────────────────
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
