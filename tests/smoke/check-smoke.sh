#!/usr/bin/env bash
# check-smoke.sh — release artifact smoke test for Linux / macOS.
#
# Verifies that the built release artifacts actually run:
#   1. the dologctl CLI binary starts and reports its version
#   2. the core shared library exports the dologger_* C ABI symbols
#   3. a foreign-language (Python ctypes) host can drive the full C ABI
#      lifecycle — init, log, config, shutdown
#
# Usage:
#   bash tests/smoke/check-smoke.sh [artifact_dir]
#
# artifact_dir defaults to ./release-artifacts (CI layout), then falls back
# to target/release (local build layout).
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Shared helpers (PROJECT_DIR, note/pass/fail, artifact discovery). The lib
# does not force `set -e`, so the tolerant FAILURES counter below works.
. "$SCRIPT_DIR/../common/lib.sh"
cd "$PROJECT_DIR"

ARTIFACT_DIR="$(resolve_artifact_dir "${1:-}")"
note "Artifact directory: $ARTIFACT_DIR"

detect_platform || exit 2
CLI="$(resolve_cli "$ARTIFACT_DIR")"
LIB="$(resolve_lib "$ARTIFACT_DIR")"

# ── 1. CLI binary runs ──────────────────────────────────────────────
note "1. CLI binary"
if [ -n "$CLI" ] && [ -x "$CLI" ]; then
    OUT="$("$CLI" version 2>&1)"
    RC=$?
    if [ $RC -eq 0 ] && printf '%s\n' "$OUT" | grep -qi "dologctl"; then
        pass "dologctl version ran (exit $RC)"
        printf '%s\n' "$OUT" | sed 's/^/       /' | head -4
    else
        fail "dologctl version failed (exit $RC)"
    fi
else
    fail "CLI binary not found (looked in $ARTIFACT_DIR)"
fi

# ── 2. Shared library exports the C ABI ─────────────────────────────
note "2. C ABI symbols"
if [ -n "$LIB" ] && [ -f "$LIB" ]; then
    SYMBOLS="$(nm -D --defined-only "$LIB" 2>/dev/null || nm --defined-only "$LIB" 2>/dev/null || true)"
    for sym in dologger_init dologger_log dologger_shutdown dologger_version; do
        if printf '%s\n' "$SYMBOLS" | grep -q " $sym\$"; then
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
PY="$(find_python)"
if [ -n "$PY" ] && [ -n "$LIB" ] && [ -f "$LIB" ]; then
    if "$PY" "$SCRIPT_DIR/c_abi_smoke.py" "$LIB"; then
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
