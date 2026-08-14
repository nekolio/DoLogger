#!/usr/bin/env bash
# ==============================================================================
# DoLogger — Full Project Build (Rust core + CLI + all plugins)
# ==============================================================================
# One command to build the entire DoLogger project from source:
#   1. C dependencies via Conan (if conanfile.py exists)
#   2. Rust core engine (libdologger_core) via Cargo
#   3. CLI tool (dologctl) via Cargo
#   4. All non-Rust plugins (C/C++/Go) via CMake + Go
#
# Usage:
#   bash scripts/build-all.sh              # debug build
#   bash scripts/build-all.sh --release    # release build
#   bash scripts/build-all.sh --core-only  # Rust only, no plugins
#   bash scripts/build-all.sh --test       # build + run all tests
# ==============================================================================
set -euo pipefail

# Shared helpers (PROJECT_DIR, colours, die/info) — resolves the repo root
# regardless of the invocation directory.
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/common.sh"
cd "$PROJECT_DIR"

BUILD_TYPE="debug"
BUILD_FLAGS=""
CORE_ONLY=false
RUN_TESTS=false

for arg in "$@"; do
    case "$arg" in
        --release|-r) BUILD_TYPE="release"; BUILD_FLAGS="--release" ;;
        --core-only)  CORE_ONLY=true ;;
        --test|-t)    RUN_TESTS=true ;;
    esac
done

banner() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║     DoLogger — Full Project Build              ║${NC}"
    echo -e "${CYAN}╠══════════════════════════════════════════════════╣${NC}"
    echo -e "${CYAN}║  Build type: $(printf '%-36s' "$BUILD_TYPE")║${NC}"
    echo -e "${CYAN}║  Target:     $(printf '%-36s' "$([ "$CORE_ONLY" = true ] && echo 'Rust only' || echo 'Rust + plugins')")║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════════════╝${NC}"
    echo ""
}

step() { echo -e "${GREEN}[$1/$TOTAL]${NC} $2"; }

# Count total steps
TOTAL=3
[ "$CORE_ONLY" = false ] && TOTAL=4
[ "$RUN_TESTS" = true ] && TOTAL=$((TOTAL + 1))
CURRENT=0

# --- Main ---
banner

# ── Step 1: Check prerequisites ──────────────────────────────────────────
CURRENT=$((CURRENT + 1))
step "$CURRENT" "Checking prerequisites..."

command -v cargo  &>/dev/null || { echo -e "${RED}Cargo not found. Install Rust: https://rustup.rs${NC}"; exit 1; }
command -v cmake  &>/dev/null || { echo -e "${RED}CMake not found. Install CMake ≥ 3.20${NC}"; exit 1; }
echo "  Rust:  $(rustc --version)"
echo "  Cargo: $(cargo --version)"
echo "  CMake: $(cmake --version | head -1)"

# ── Step 2: C dependencies (optional — only if conanfile.py present) ─────
if [ "$CORE_ONLY" = false ] && [ -f "$PROJECT_DIR/conanfile.py" ]; then
    CURRENT=$((CURRENT + 1))
    step "$CURRENT" "C dependencies (Conan)..."
    if command -v conan &>/dev/null; then
        bash "$PROJECT_DIR/scripts/setup-conan.sh" || echo "  Conan setup skipped (deps may already be installed)"
    else
        echo "  Conan not installed — skipping C dependency management"
        echo "  Install: pip install conan"
    fi
else
    echo "  Skipping C dependencies (core-only build)"
fi

# ── Step 3: Rust core + CLI ──────────────────────────────────────────────
CURRENT=$((CURRENT + 1))
step "$CURRENT" "Building Rust core + CLI..."

cd "$PROJECT_DIR"
cargo build $BUILD_FLAGS

echo "  Core library: target/$BUILD_TYPE/dologger_core.{so,dylib,dll}"
echo "  CLI tool:     target/$BUILD_TYPE/dologctl"

# ── Step 4: Non-Rust plugins ─────────────────────────────────────────────
if [ "$CORE_ONLY" = false ]; then
    CURRENT=$((CURRENT + 1))
    step "$CURRENT" "Building non-Rust plugins..."
    bash "$PROJECT_DIR/scripts/build-plugins.sh" --"$BUILD_TYPE"
fi

# ── Step 5: Run tests (optional) ─────────────────────────────────────────
if [ "$RUN_TESTS" = true ]; then
    CURRENT=$((CURRENT + 1))
    step "$CURRENT" "Running all tests..."
    cargo test $BUILD_FLAGS
fi

# ── Done ─────────────────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}╔══════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║  ${GREEN}Build complete!${CYAN}                              ║${NC}"
echo -e "${CYAN}╠══════════════════════════════════════════════════╣${NC}"
echo -e "${CYAN}║  Core:   target/$BUILD_TYPE/dologger_core       ║${NC}"
echo -e "${CYAN}║  CLI:    target/$BUILD_TYPE/dologctl            ║${NC}"
[ "$CORE_ONLY" = false ] && echo -e "${CYAN}║  Plugin: build/plugins/                         ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════════╝${NC}"
echo ""
