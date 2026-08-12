#!/usr/bin/env bash
# ==============================================================================
# DoLogger — Build All Non-Rust Plugins (C, C++, Go)
# ==============================================================================
# Builds every non-Rust plugin example in plugins/examples/ and plugins/official/
# that uses a CMakeLists.txt or go.mod.
#
# Prerequisites:
#   bash scripts/setup-conan.sh       # one-time C dependency setup
#
# Usage:
#   bash scripts/build-plugins.sh              # build all plugins
#   bash scripts/build-plugins.sh --release    # release build
#   bash scripts/build-plugins.sh --debug      # debug build (default)
#   bash scripts/build-plugins.sh --filter c   # only C plugins
#   bash scripts/build-plugins.sh --filter go  # only Go plugins
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="${BUILD_DIR:-$PROJECT_DIR/build}"
BUILD_TYPE="Debug"
FILTER_LANG=""

# Colours
RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'
YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'

for arg in "$@"; do
    case "$arg" in
        --release|-r) BUILD_TYPE="Release" ;;
        --debug|-d)   BUILD_TYPE="Debug" ;;
        --filter)     shift; FILTER_LANG="${1:-}" ;;
        --filter=*)   FILTER_LANG="${arg#*=}" ;;
    esac
done

banner() {
    echo ""
    echo -e "${CYAN}=============================================${NC}"
    echo -e "${CYAN} DoLogger — Plugin Build${NC}"
    echo -e "${CYAN}=============================================${NC}"
    echo -e "  Build type: ${BOLD}$BUILD_TYPE${NC}"
    echo -e "  Build dir:  ${CYAN}$BUILD_DIR${NC}"
    echo ""
}

build_c_plugins() {
    local count=0
    echo -e "${BOLD}[C/C++ Plugins]${NC}"

    # Use Conan toolchain if available
    local cmake_args="-DCMAKE_BUILD_TYPE=$BUILD_TYPE"
    local toolchain="$BUILD_DIR/conan_toolchain.cmake"
    if [ -f "$toolchain" ]; then
        cmake_args="$cmake_args -DCMAKE_TOOLCHAIN_FILE=$toolchain"
        echo "  Using Conan toolchain: $toolchain"
    else
        echo -e "  ${YELLOW}No Conan toolchain found — run 'bash scripts/setup-conan.sh' first for C deps${NC}"
    fi

    # Find all C/C++ plugin directories with CMakeLists.txt
    while IFS= read -r -d '' cmake_file; do
        local plugin_dir; plugin_dir="$(dirname "$cmake_file")"
        local plugin_name; plugin_name="$(basename "$plugin_dir")"
        local plugin_build_dir="$BUILD_DIR/plugins/$plugin_name"

        echo -e "  ${GREEN}→${NC} $plugin_name ($plugin_dir)"
        cmake -B "$plugin_build_dir" -S "$plugin_dir" $cmake_args > /dev/null 2>&1
        cmake --build "$plugin_build_dir" --config "$BUILD_TYPE" 2>&1 | while IFS= read -r line; do
            echo "    $line"
        done
        count=$((count + 1))
    done < <(find "$PROJECT_DIR/plugins" -name CMakeLists.txt -print0 2>/dev/null)

    echo ""
    echo -e "  ${GREEN}Built $count C/C++ plugin(s)${NC}"
}

build_go_plugins() {
    local count=0
    echo -e "${BOLD}[Go Plugins]${NC}"

    if ! command -v go &>/dev/null; then
        echo -e "  ${YELLOW}Go not found — skipping Go plugins${NC}"
        return
    fi
    echo -e "  Go version: $(go version)"

    while IFS= read -r -d '' go_mod; do
        local plugin_dir; plugin_dir="$(dirname "$go_mod")"
        local plugin_name; plugin_name="$(basename "$plugin_dir")"
        # Skip if not a cgo plugin (no "C" import)
        if ! grep -q 'import "C"' "$plugin_dir"/*.go 2>/dev/null; then
            continue
        fi

        local out_name="dologger-plugin-${plugin_name}"
        case "$(uname -s)" in
            Linux)  out_name="${out_name}.so" ;;
            Darwin) out_name="${out_name}.dylib" ;;
            MINGW*|MSYS*) out_name="${out_name}.dll" ;;
        esac

        echo -e "  ${GREEN}→${NC} $plugin_name → $out_name"
        (cd "$plugin_dir" && CGO_ENABLED=1 go build -buildmode=c-shared -o "$out_name" . 2>&1) | while IFS= read -r line; do
            echo "    $line"
        done
        count=$((count + 1))
    done < <(find "$PROJECT_DIR/plugins" -name go.mod -print0 2>/dev/null)

    echo ""
    echo -e "  ${GREEN}Built $count Go plugin(s)${NC}"
}

# --- Main ---
banner

case "$FILTER_LANG" in
    c|cpp|"")
        build_c_plugins
        ;;
esac

case "$FILTER_LANG" in
    go|"")
        build_go_plugins
        ;;
esac

echo -e "${CYAN}=============================================${NC}"
echo -e "${GREEN}  Plugin build complete!${NC}"
echo -e "${CYAN}=============================================${NC}"
echo ""
echo "Plugin artifacts:"
find "$BUILD_DIR/plugins" -name "*.so" -o -name "*.dylib" -o -name "*.dll" 2>/dev/null | while IFS= read -r f; do
    echo "  $f"
done
echo ""
