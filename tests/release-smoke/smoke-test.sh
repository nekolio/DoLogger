#!/usr/bin/env bash
# smoke-test.sh — release artifact smoke test for Linux / macOS.
#
# Verifies that the built release artifacts actually run:
#   1. the dologctl CLI binary starts and reports its version
#   2. the core shared library exports the dologger_* C ABI symbols
#   3. a foreign-language (Python ctypes) host can drive the full C ABI
#      lifecycle — init, log, config, shutdown
#
# Usage:
#   bash tests/release-smoke/smoke-test.sh [artifact_dir]
#
# artifact_dir defaults to ./release-artifacts (CI layout), then falls back
# to target/release (local build layout).
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACT_DIR="${1:-}"
FAILURES=0

note()  { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }
pass() { printf '  \033[32m[PASS]\033[0m %s\n' "$*"; }
fail() { printf '  \033[31m[FAIL]\033[0m %s\n' "$*"; FAILURES=$((FAILURES + 1)); }

# ── Locate artifacts ────────────────────────────────────────────────
if [ -z "$ARTIFACT_DIR" ]; then
    if [ -d release-artifacts ]; then
        ARTIFACT_DIR="release-artifacts"
    else
        ARTIFACT_DIR="target/release"
    fi
fi
note "Artifact directory: $ARTIFACT_DIR"

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux)  LIB_NAME="libdologger_core.so";  CLI_NAME="dologctl" ;;
    Darwin) LIB_NAME="libdologger_core.dylib"; CLI_NAME="dologctl" ;;
    *) echo "Unsupported OS: $OS"; exit 2 ;;
esac

# CI layout: dologctl-<os>-<arch>; local layout: plain dologctl
CLI="$ARTIFACT_DIR/dologctl-${OS,,}-${ARCH}"
[ -x "$CLI" ] || CLI="$ARTIFACT_DIR/$CLI_NAME"
[ -x "$CLI" ] || CLI="$ARTIFACT_DIR/${CLI_NAME}"
LIB="$ARTIFACT_DIR/$LIB_NAME"
[ -f "$LIB" ] || LIB="$(find "$ARTIFACT_DIR" -maxdepth 1 -name "$LIB_NAME" | head -1)"

# ── 1. CLI binary runs ──────────────────────────────────────────────
note "1. CLI binary"
if [ -x "$CLI" ]; then
    OUT="$("$CLI" version 2>&1)"
    RC=$?
    if [ $RC -eq 0 ] && echo "$OUT" | grep -qi "dologctl"; then
        pass "dologctl version ran (exit $RC)"
        echo "$OUT" | sed 's/^/       /' | head -4
    else
        fail "dologctl version failed (exit $RC)"
    fi
else
    fail "CLI binary not found (looked in $ARTIFACT_DIR)"
fi

# ── 2. Shared library exports the C ABI ─────────────────────────────
note "2. C ABI symbols"
if [ -f "$LIB" ]; then
    SYMBOLS="$(nm -D --defined-only "$LIB" 2>/dev/null || nm --defined-only "$LIB" 2>/dev/null || true)"
    for sym in dologger_init dologger_log dologger_shutdown dologger_version; do
        if echo "$SYMBOLS" | grep -q " $sym\$"; then
            pass "exported: $sym"
        else
            fail "missing symbol: $sym"
        fi
    done
else
    fail "core library not found (looked for $LIB_NAME in $ARTIFACT_DIR)"
fi

# ── 3. Foreign-language C ABI lifecycle (Python ctypes) ─────────────
note "3. C ABI via Python ctypes"
PY="$(command -v python3 || command -v python || true)"
if [ -n "$PY" ] && [ -f "$LIB" ]; then
    if "$PY" "$SCRIPT_DIR/cabi_smoke.py" "$LIB"; then
        pass "full C ABI lifecycle via ctypes"
    else
        fail "C ABI lifecycle via ctypes"
    fi
elif [ -z "$PY" ]; then
    fail "python3 not found — C ABI cross-language check skipped"
fi

# ── Summary ─────────────────────────────────────────────────────────
echo
if [ $FAILURES -eq 0 ]; then
    echo "SMOKE TEST: ALL PASSED"
    exit 0
else
    echo "SMOKE TEST: $FAILURES FAILURE(S)"
    exit 1
fi
