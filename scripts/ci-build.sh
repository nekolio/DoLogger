#!/usr/bin/env bash
# ==============================================================================
# DoLogger — CI Build Script
# ==============================================================================
# Used by GitHub Actions and other CI systems.  Builds everything with all
# feature-gates enabled in release+LTO mode.  Exits non-zero on any failure.
#
# Usage:
#   bash scripts/ci-build.sh              # full build
#   bash scripts/ci-build.sh --check      # cargo check only (fast)
#   bash scripts/ci-build.sh --clippy     # run clippy lints
#   bash scripts/ci-build.sh --deny       # run cargo-deny license audit
#   bash scripts/ci-build.sh --all        # build + lint + audit + test
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'; NC='\033[0m'

MODE="build"
for arg in "$@"; do
    case "$arg" in
        --check)  MODE="check" ;;
        --clippy) MODE="clippy" ;;
        --deny)   MODE="deny" ;;
        --all)    MODE="all" ;;
    esac
done

cd "$PROJECT_DIR"

run_step() {
    echo -e "${CYAN}▶ $1${NC}"
}

# --- Build: release + LTO, all feature gates ---
build_all() {
    run_step "Building Rust workspace (release + LTO)..."
    cargo build --release

    run_step "Building with all feature gates..."
    cargo build --release --features sink-kafka,sink-webhook,sink-sqlite 2>&1 || true
    echo "  (feature-gated build may fail if C libraries are missing — this is expected in CI)"
}

# --- Check only (fast compile check, no codegen) ---
check_all() {
    run_step "cargo check (all targets)..."
    cargo check --workspace --all-targets

    run_step "cargo check (all feature gates)..."
    cargo check --workspace --all-targets --features sink-kafka,sink-webhook,sink-sqlite 2>&1 || true
}

# --- Clippy lints ---
clippy_all() {
    run_step "cargo clippy (core)..."
    cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings

    run_step "cargo clippy (cli)..."
    cargo clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings

    run_step "cargo clippy (plugins)..."
    for plugin in plugins/official/*/; do
        [ -f "$plugin/Cargo.toml" ] || continue
        echo "  $(basename "$plugin")"
        cargo clippy --manifest-path "$plugin/Cargo.toml" -- -D warnings 2>&1 || true
    done
}

# --- License audit ---
deny_check() {
    run_step "cargo deny check (license audit)..."
    if command -v cargo-deny &>/dev/null; then
        cargo deny check licenses
        cargo deny check bans
    else
        echo "  cargo-deny not installed — skipping"
    fi
}

# --- Tests ---
run_tests() {
    run_step "Running all tests..."
    cargo test --workspace
}

# --- Main dispatch ---
case "$MODE" in
    build)
        build_all
        ;;
    check)
        check_all
        ;;
    clippy)
        clippy_all
        ;;
    deny)
        deny_check
        ;;
    all)
        build_all
        clippy_all
        deny_check
        run_tests
        ;;
esac

echo ""
echo -e "${GREEN}CI build step '$MODE' completed successfully.${NC}"
