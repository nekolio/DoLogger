#!/usr/bin/env bash
# Build the GitHub Pages artifact for the DoLogger site.
#
# Runs identically locally and in CI:  bash peripheral/github/scripts/build-site.sh [OUT]
# (default OUT=peripheral/site/dist).
#
#  1. Builds the Vue 3 + TypeScript app with Vite (bun if available, else
#     npm) — peripheral/site/ → peripheral/site/dist.
#  2. Bakes live data server-side: the real latest release (5 entries) and
#     that release's benchmark-results.json. The browser cannot fetch
#     release assets (CORS), so this must happen at build time.
#
# data.js prefers the baked files, then its own GitHub API calls (with a
# localStorage cache), then the hardcoded v0.1.0 manifest. Without
# GITHUB_TOKEN (local dev) the data files are fallback markers and the
# page still renders fully — the artifact is static either way.
set -euo pipefail

OUT="${1:-peripheral/site/dist}"
REPO="Nekolio/DoLogger"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

# 1. Vite build (vue-tsc type-check runs inside `bun run build` too).
cd "$ROOT/peripheral/site"
if command -v bun >/dev/null 2>&1; then
  bun install --frozen-lockfile 2>/dev/null || bun install
  bun run build
else
  npm install
  npm run build
fi
cd "$ROOT"
mkdir -p "$OUT/data"

# 2. docs assets are the source of truth for the hero/architecture art —
#    overwrite whatever Vite copied so a regenerated SVG always ships.
cp "$ROOT"/docs/assets/hero.svg "$OUT/assets/hero.svg"
cp "$ROOT"/docs/assets/architecture.svg "$OUT/assets/architecture.svg"
cp "$ROOT"/docs/assets/architecture-zh.svg "$OUT/assets/architecture-zh.svg"

# 3. Bake live data.
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  echo "> baking live data (GITHUB_TOKEN set)"
  gh api "repos/$REPO/releases?per_page=100" > "$OUT/data/releases.json" 2>/dev/null \
    || echo '[]' > "$OUT/data/releases.json"
  gh api "repos/$REPO/contributors?per_page=12" > "$OUT/data/contributors.json" 2>/dev/null \
    || echo '[]' > "$OUT/data/contributors.json"
  TAG="$(gh api "repos/$REPO/releases?per_page=100" --jq '.[0].tag_name' 2>/dev/null || true)"
  if [[ -n "$TAG" ]]; then
    BENCH_URL="$(gh api "repos/$REPO/releases/tags/$TAG" \
      --jq '.assets[] | select(.name=="benchmark-results.json") | .browser_download_url' 2>/dev/null || true)"
    if [[ -n "$BENCH_URL" ]] && curl -fsSL "$BENCH_URL" -o "$OUT/data/benchmarks.json"; then
      echo "> baked benchmarks from $TAG"
    else
      echo '{"fallback":true}' > "$OUT/data/benchmarks.json"
    fi
  else
    echo '{"fallback":true}' > "$OUT/data/benchmarks.json"
  fi
else
  echo "> no GITHUB_TOKEN — fallback data files (local build)"
  echo '[]' > "$OUT/data/releases.json"
  echo '{"fallback":true}' > "$OUT/data/benchmarks.json"
  echo '[]' > "$OUT/data/contributors.json"
fi

echo "site built -> $OUT"
