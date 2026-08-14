#!/usr/bin/env bash
# ==============================================================================
# DoLogger — Conan Setup Script (friendly interactive setup)
# ==============================================================================
# Detects your platform, selects the right Conan profile, installs C
# dependencies, and generates the CMake toolchain file.
#
# Usage:
#   bash scripts/setup-conan.sh              # auto-detect profile
#   bash scripts/setup-conan.sh --profile linux-gcc-x86_64  # explicit profile
#   bash scripts/setup-conan.sh --dry-run    # show what would happen
#   bash scripts/setup-conan.sh --detect     # print detected profile only
# ==============================================================================
set -euo pipefail

# Shared helpers (PROJECT_DIR, colours, die/info) — resolves the repo root
# regardless of the invocation directory.
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/common.sh"
cd "$PROJECT_DIR"

PROFILES_DIR="$PROJECT_DIR/.conan/profiles"
BUILD_DIR="${BUILD_DIR:-$PROJECT_DIR/build}"

banner() {
    echo ""
    echo -e "${CYAN}=============================================${NC}"
    echo -e "${CYAN} DoLogger — Conan Dependency Setup${NC}"
    echo -e "${CYAN}=============================================${NC}"
    echo ""
}

# ---------------------------------------------------------------------------
# Platform auto-detection → Conan profile name
# ---------------------------------------------------------------------------
detect_profile() {
    local os=""; local arch=""; local compiler=""

    case "$(uname -s)" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *)      os="unknown" ;;
    esac

    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="arm64"   ;;
        *)              arch="x86_64"  ;;  # fallback
    esac

    if [ "$os" = "linux" ]; then
        if command -v clang &>/dev/null && [ "$(clang --version 2>&1 | head -1 | grep -ci clang)" -gt 0 ]; then
            compiler="clang"
        else
            compiler="gcc"
        fi
    elif [ "$os" = "macos" ]; then
        compiler="clang"
    elif [ "$os" = "windows" ]; then
        compiler="msvc"
    fi

    echo "${os}-${compiler}-${arch}"
}

# ---------------------------------------------------------------------------
# Verify Conan is installed and at the right version
# ---------------------------------------------------------------------------
check_conan() {
    if ! command -v conan &>/dev/null; then
        echo -e "${RED}[ERROR] Conan not found in PATH.${NC}"
        echo ""
        echo "Install Conan 2.x:"
        echo "  pip install conan          # system Python"
        echo "  pipx install conan         # isolated install (recommended)"
        echo "  brew install conan         # macOS Homebrew"
        echo ""
        echo "Then run: conan profile detect"
        exit 1
    fi

    local ver; ver=$(conan --version 2>&1 | grep -oP '[\d]+\.[\d]+' | head -1 || echo "0.0")
    local major; major=$(echo "$ver" | cut -d. -f1)
    if [ "$major" -lt 2 ]; then
        echo -e "${RED}[ERROR] Conan 2.x required (found $ver).${NC}"
        echo "Upgrade: pip install --upgrade conan"
        exit 1
    fi
    echo -e "  Conan version: ${GREEN}$ver${NC}"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
DRY_RUN=false; DETECT_ONLY=false; PROFILE_ARG=""

for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=true ;;
        --detect)  DETECT_ONLY=true ;;
        --profile) shift; PROFILE_ARG="${1:-}" ;;
        --profile=*) PROFILE_ARG="${arg#*=}" ;;
    esac
done

banner

PROFILE_NAME="${PROFILE_ARG:-$(detect_profile)}"

if $DETECT_ONLY; then
    echo "$PROFILE_NAME"
    exit 0
fi

PROFILE_PATH="$PROFILES_DIR/$PROFILE_NAME"

if [ ! -f "$PROFILE_PATH" ]; then
    echo -e "${RED}[ERROR] Profile not found: $PROFILE_PATH${NC}"
    echo ""
    echo "Available profiles:"
    for f in "$PROFILES_DIR"/*; do
        [ -f "$f" ] && echo "  $(basename "$f")"
    done
    echo ""
    echo "Create a custom profile at: $PROFILES_DIR/<name>"
    exit 1
fi

# --- Print what we are about to do ---
echo -e "  Platform detected: ${BOLD}$(detect_profile)${NC}"
echo -e "  Selected profile:  ${BOLD}$PROFILE_NAME${NC}"
echo -e "  Build directory:   ${CYAN}$BUILD_DIR${NC}"
echo -e "  Profile path:      ${CYAN}$PROFILE_PATH${NC}"
echo ""

check_conan

if $DRY_RUN; then
    echo -e "${YELLOW}[DRY-RUN] Would execute:${NC}"
    echo "  conan install \"$PROJECT_DIR\" \\"
    echo "    --output-folder=\"$BUILD_DIR\" \\"
    echo "    --profile:host=\"$PROFILE_PATH\" \\"
    echo "    --profile:build=\"$PROFILE_PATH\" \\"
    echo "    --build=missing"
    echo ""
    echo -e "${YELLOW}[DRY-RUN] Then for CMake:${NC}"
    echo "  cmake -B \"$BUILD_DIR\" \\"
    echo "    -DCMAKE_TOOLCHAIN_FILE=\"$BUILD_DIR/conan_toolchain.cmake\" \\"
    echo "    -DCMAKE_BUILD_TYPE=Release"
    echo ""
    exit 0
fi

# --- Detect default Conan profile ---
echo -e "${CYAN}[1/3] Detecting default Conan profile...${NC}"
if ! conan profile show default &>/dev/null; then
    echo "  No default profile found — running 'conan profile detect'..."
    conan profile detect
fi
conan profile show default 2>/dev/null | head -5 || true
echo ""

# --- Install dependencies ---
echo -e "${CYAN}[2/3] Installing C dependencies via Conan...${NC}"
echo "  (This may take several minutes on first run — libraries are cached after)"
echo ""

conan install "$PROJECT_DIR" \
    --output-folder="$BUILD_DIR" \
    --profile:host="$PROFILE_PATH" \
    --profile:build="$PROFILE_PATH" \
    --build=missing

echo ""
echo -e "${GREEN}  Dependencies installed successfully.${NC}"
echo ""

# --- Post-install summary ---
echo -e "${CYAN}[3/3] Build instructions${NC}"
echo ""
echo -e "  ${BOLD}Next steps:${NC}"
echo ""
echo -e "  ${GREEN}# Build Rust core + CLI only${NC}"
echo "  cargo build --release"
echo ""
echo -e "  ${GREEN}# Build C/C++ plugins (uses Conan toolchain)${NC}"
echo "  cmake -B \"$BUILD_DIR\" -DCMAKE_TOOLCHAIN_FILE=\"$BUILD_DIR/conan_toolchain.cmake\" -DCMAKE_BUILD_TYPE=Release"
echo "  cmake --build \"$BUILD_DIR\" --target dologger_plugins"
echo ""
echo -e "  ${GREEN}# Build everything (Rust + C/C++ + Go)${NC}"
echo "  bash scripts/build-all.sh"
echo ""
echo -e "  ${GREEN}# Quick: build all with one command${NC}"
echo "  bash scripts/build-plugins.sh"
echo ""
echo -e "${CYAN}=============================================${NC}"
echo -e "${GREEN}  Conan setup complete!${NC}"
echo -e "${CYAN}=============================================${NC}"
echo ""
