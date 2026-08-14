#!/usr/bin/env bash
# DoLogger Development Environment Setup
# One-command setup for all required dev tools.
# Usage: bash scripts/setup-development.sh
set -euo pipefail

# Shared helpers (PROJECT_DIR, colours, die/info) — resolves the repo root
# regardless of the invocation directory.
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/common.sh"
cd "$PROJECT_DIR"

echo ""
echo "=========================================="
echo " DoLogger Development Environment Setup"
echo "=========================================="
echo ""

# --- Rust components ---
echo "[1/7] Checking Rust toolchain..."
rustup default stable 2>/dev/null || true
rustup component add clippy rustfmt 2>/dev/null || true
echo "  Rust: $(rustc --version)"
echo "  Cargo: $(cargo --version)"

# --- Cargo tools ---
echo ""
echo "[2/7] Installing Rust dev tools..."
for tool in cargo-deny cargo-audit; do
    if command -v "$tool" &>/dev/null; then
        echo "  $tool: already installed ($($tool --version 2>&1 | head -1))"
    else
        echo "  $tool: installing..."
        cargo install "$tool" --locked 2>&1 | tail -1 || echo "  $tool: install failed (continue)"
    fi
done

# --- Conan (C/C++ dependency manager — preferred) ---
echo ""
echo "[3/7] Checking Conan 2.x..."
if command -v conan &>/dev/null; then
    echo "  Conan: $(conan --version 2>&1 | head -1)"
    if conan profile show default &>/dev/null; then
        echo "  Default profile: ready"
    else
        echo "  Running 'conan profile detect'..."
        conan profile detect
    fi
    echo "  → Run 'bash scripts/setup-conan.sh' to install C dependencies"
else
    echo "  Conan: NOT FOUND"
    echo "  Install: pip install conan  (recommended: pipx install conan)"
    echo "  Conan manages C libraries (librdkafka, sqlite3, libsodium)"
    echo "  needed by non-Rust plugins and feature-gated sinks."
fi

# --- Bun tools ---
echo ""
echo "[4/7] Checking Bun..."
echo "  Bun: $(bun --version 2>/dev/null || echo 'not found — install from https://bun.sh')"

# --- C/C++ dependencies (vcpkg fallback) ---
echo ""
echo "[5/7] Checking C/C++ fallback (vcpkg)..."
if command -v vcpkg &>/dev/null; then
    echo "  vcpkg: $(vcpkg version 2>&1 | head -1)"
    for lib in librdkafka sqlite3 libsodium protobuf flatbuffers; do
        if vcpkg list 2>/dev/null | grep -q "$lib"; then
            echo "  $lib: installed (via vcpkg)"
        else
            echo "  $lib: not installed — preferred: use Conan (scripts/setup-conan.sh)"
        fi
    done
else
    echo "  vcpkg: not found (Conan is preferred for C dependency management)"
fi

# --- Docker ---
echo ""
echo "[6/7] Checking Docker..."
if command -v docker &>/dev/null; then
    echo "  Docker: $(docker --version)"
else
    echo "  Docker: not found — install Docker Desktop from https://docker.com"
fi

# --- System tools ---
echo ""
echo "[7/7] Checking system tools..."
for tool in cmake gpg openssl curl protoc sqlite3 pwsh python go gh; do
    if command -v "$tool" &>/dev/null; then
        ver=$($tool --version 2>&1 | head -1 || echo "?")
        echo "  $tool: $ver"
    else
        echo "  $tool: NOT FOUND"
    fi
done

echo ""
echo "=========================================="
echo " Run 'bash scripts/check-environment.sh' to verify."
echo " Run 'bash scripts/setup-conan.sh' for C deps."
echo " Run 'bash scripts/build-all.sh --release' to build."
echo "=========================================="
echo ""
