#!/usr/bin/env bash
# gen-release-notes.sh — assemble a GitHub Release body from CHANGELOG.md,
# CHANGELOG.zh_CN.md and the commit history.
#
# Usage:
#   scripts/gen-release-notes.sh <tag> > release-notes.md
#
# Requires a full git history (actions/checkout with fetch-depth: 0).
set -euo pipefail

TAG="${1:?usage: gen-release-notes.sh <tag>}"
VER="${TAG#v}"
BODY="$(mktemp)"

# ── 1. Changelog sections (EN + ZH) ────────────────────────────────
extract_section() {
    local changelog="$1"
    [ -f "$changelog" ] || return 0
    awk -v pat="## [${VER}]" '
        !started { if (index($0, pat) == 1) { started = 1; print; next } next }
        started  { if (/^## \[/) exit; print }
    ' "$changelog"
}

EN_SECTION="$(extract_section CHANGELOG.md)"
ZH_SECTION="$(extract_section CHANGELOG.zh_CN.md)"

# ── 2. Commit list since the previous tag ──────────────────────────
PREV_TAG="$(git describe --tags --abbrev=0 "${TAG}^" 2>/dev/null || true)"
if [ -n "$PREV_TAG" ]; then
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${PREV_TAG}..${TAG}" 2>/dev/null | head -100 || true)"
else
    COMMITS="$(git log --no-merges --pretty=format:'- %s (%h)' "${TAG}" 2>/dev/null | head -100 || true)"
fi

{
    echo "# DoLogger ${TAG}"
    echo

    if [ -n "$EN_SECTION" ]; then
        echo "## 📝 Changelog"
        echo
        printf '%s\n' "$EN_SECTION"
        echo
    else
        echo "> No entry for ${VER} found in CHANGELOG.md — please add one before releasing."
        echo
    fi

    if [ -n "$ZH_SECTION" ]; then
        echo "## 📝 更新日志"
        echo
        printf '%s\n' "$ZH_SECTION"
        echo
    fi

    echo "## 🔄 Commits"
    echo
    if [ -n "$PREV_TAG" ]; then
        echo "Changes since ${PREV_TAG}:"
    else
        echo "Initial release:"
    fi
    echo
    printf '%s\n' "${COMMITS:-_(no commits found — check that the workflow checks out full history)_}"
    echo

    echo "## 🚀 Usage"
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
    echo 'See the [README](https://github.com/Nekolio/DoLogger#readme) and the'
    echo '[wiki](https://github.com/Nekolio/DoLogger/wiki) for full documentation.'
    echo

    echo "## 📦 Asset Naming Conventions"
    echo
    echo '| Asset | Pattern |'
    echo '|:-:|:-:|'
    echo '| CLI binary | `dologctl-<os>-<arch>` (`.exe` on Windows) |'
    echo '| Core library | `libdologger_core.so` (Linux), `libdologger_core.dylib` (macOS), `dologger_core.dll` (Windows) |'
    echo '| Checksums | `checksums-sha256.txt` — SHA-256 of every asset |'
    echo '| Git tags | `vMAJOR.MINOR.PATCH`; pre-releases use `-alpha.N`, `-beta.N`, `-rc.N` |'
    echo

    echo "## 🔒 Integrity Verification"
    echo
    echo 'Every asset is listed in `checksums-sha256.txt`. Before running the'
    echo 'binary, verify the download against its checksum. For supply-chain'
    echo 'audits, see the [Security Whitepaper](https://github.com/Nekolio/DoLogger/blob/main/Docs/en_US/guides/SecurityWhitepaper.md).'
    echo
} > "$BODY"

cat "$BODY"
rm -f "$BODY"
