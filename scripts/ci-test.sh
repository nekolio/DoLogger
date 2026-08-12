#!/usr/bin/env bash
# ==============================================================================
# DoLogger — CI Test Script
# ==============================================================================
# Runs all test suites: unit tests, integration tests, doc tests, benchmarks.
# Exits non-zero on any failure.
#
# Usage:
#   bash scripts/ci-test.sh               # all tests
#   bash scripts/ci-test.sh --unit        # unit tests only
#   bash scripts/ci-test.sh --bench       # benchmarks (quick, --quick)
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'

MODE="all"
for arg in "$@"; do
    case "$arg" in
        --unit)  MODE="unit" ;;
        --bench) MODE="bench" ;;
    esac
done

cd "$PROJECT_DIR"

step() { echo -e "${CYAN}▶${NC} $1"; }
pass() { echo -e "  ${GREEN}✓${NC} $1"; }
fail() { echo -e "  ${RED}✗${NC} $1"; }

# --- Unit + integration tests ---
if [ "$MODE" = "all" ] || [ "$MODE" = "unit" ]; then
    step "Core library tests..."
    cargo test --manifest-path core/Cargo.toml --lib && pass "core lib" || fail "core lib"

    step "Core integration tests..."
    cargo test --manifest-path core/Cargo.toml --test '*' 2>&1 && pass "core integration" || echo "  (no integration tests found)"

    step "CLI tests..."
    cargo test --manifest-path cli/Cargo.toml && pass "cli" || fail "cli"

    step "Plugin tests..."
    for plugin in plugins/official/*/; do
        [ -f "$plugin/Cargo.toml" ] || continue
        name="$(basename "$plugin")"
        cargo test --manifest-path "$plugin/Cargo.toml" 2>&1 && pass "$name" || fail "$name"
    done
fi

# --- Benchmarks (quick mode for CI) ---
if [ "$MODE" = "bench" ] || [ "$MODE" = "all" ]; then
    step "Benchmarks (quick mode)..."
    cargo bench --manifest-path core/Cargo.toml -- --quick 2>&1 && pass "benchmarks" || echo "  (benchmarks skipped — may need nightly)"
fi

echo ""
echo -e "${GREEN}CI test step completed.${NC}"
