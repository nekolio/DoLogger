#!/usr/bin/env bash
# ==============================================================================
# DoLogger Development Environment Check
# ==============================================================================
# Usage: bash scripts/check-env.sh
# Exit code 0 = all OK, 1 = some missing
set -euo pipefail

# Shared helpers (PROJECT_DIR, colours, die/info) — resolves the repo root
# regardless of the invocation directory.
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/common.sh"
cd "$PROJECT_DIR"

MISSING=0

check() {
    local name="$1"; local cmd="$2"; local ver_flag="${3:---version}"
    if command -v "$cmd" &>/dev/null; then
        local ver; ver=$($cmd $ver_flag 2>&1 | head -1 || echo "?")
        printf "${GREEN}  ✅${NC} %-18s %s\n" "$name" "$ver"
    else
        printf "${RED}  ❌${NC} %-18s NOT FOUND\n" "$name"
        MISSING=1
    fi
}

check_python() {
    for py in python3 python; do
        if command -v "$py" &>/dev/null; then
            local ver; ver=$($py --version 2>&1 | head -1)
            printf "${GREEN}  ✅${NC} %-18s %s\n" "Python" "$ver"
            return 0
        fi
    done
    printf "${RED}  ❌${NC} %-18s NOT FOUND\n" "Python"
    MISSING=1
}

echo ""
echo "=========================================="
echo " DoLogger Development Environment Check"
echo "=========================================="

echo ""; echo "[Core Tools: Rust + CMake]"
check "Rust"           "rustc"
check "Cargo"          "cargo"
check "CMake"          "cmake"
check "Bun"            "bun"

echo ""; echo "[C/C++ Dependency Management]"
check "Conan 2.x"      "conan"
if command -v conan &>/dev/null; then
    echo "  Profiles available:"
    for p in .conan/profiles/*; do
        [ -f "$p" ] && echo "    $(basename "$p")"
    done
fi
check "vcpkg (fallback)" "vcpkg"
check "pkg-config"     "pkg-config"

echo ""; echo "[Crypto & Signing]"
check "OpenSSL"        "openssl"
check "GnuPG"          "gpg"
check "curl"           "curl"

echo ""; echo "[Remote Sinks & Control Plane]"
check "Docker"         "docker"
check "protoc"         "protoc"
check "SQLite3"        "sqlite3"
check "flatc"          "flatc"

echo ""; echo "[CI/CD & Compliance]"
check "cargo-deny"     "cargo-deny"
check "cargo-audit"    "cargo-audit"
check "PowerShell 7"   "pwsh"
check_python
check "Go"             "go"       "version"
check "GitHub CLI"     "gh"

echo ""; echo "[Signing Tools (Optional)]"
check "Cosign"         "cosign"
check "minisign"       "minisign"

# Conan package check
echo ""; echo "[Conan C Libraries]"
if command -v conan &>/dev/null; then
    for lib in librdkafka sqlite3 libsodium; do
        if conan list "$lib/*" 2>/dev/null | grep -q "$lib"; then
            printf "${GREEN}  ✅${NC} %-18s cached in Conan\n" "$lib"
        else
            printf "${YELLOW}  ⚠️${NC}  %-18s not cached → 'bash scripts/setup-conan.sh'\n" "$lib"
        fi
    done
else
    printf "${YELLOW}  ⚠️${NC}  Conan not installed — skip C lib check\n"
fi

# Build targets
echo ""; echo "[Quick Build Test]"
echo "  Build all:      bash scripts/build-all.sh --release"
echo "  Build plugins:  bash scripts/build-plugins.sh"
echo "  Setup Conan:    bash scripts/setup-conan.sh"
echo "  Dry-run Conan:  bash scripts/setup-conan.sh --dry-run"

echo ""; echo "=========================================="
if [ $MISSING -eq 0 ]; then
    printf "${GREEN}  All checks passed!${NC}\n"
else
    printf "${RED}  $MISSING tool(s) missing. See above.${NC}\n"
fi
echo "=========================================="
echo ""
exit $MISSING
