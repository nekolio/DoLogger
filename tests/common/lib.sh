#!/usr/bin/env bash
# lib.sh — shared helpers for DoLogger shell test suites.
#
# Source from a suite under tests/ (NOT tests/common/) with:
#   . "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../common/lib.sh"
#
# Provides:
#   PROJECT_DIR              — absolute repo root
#   colour vars              — RED / GREEN / YELLOW / CYAN / BOLD / NC
#   FAILURES                 — running failure counter (see fail())
#   die() / info()           — fatal error / informational line
#   note() / pass() / fail() — check-result helpers; fail() bumps FAILURES
#   resolve_artifact_dir()   — locate the release artifact directory
#   detect_platform()        — set OS / ARCH / LIB_NAME / CLI_NAME
#   resolve_cli() / resolve_lib() — discover dologctl / core lib in a dir
#   find_python()            — a usable Python interpreter (python3 first)
#
# Unlike scripts/lib/common.sh this lib does NOT force `set -e`: test suites
# count individual failures with the FAILURES counter and decide their own exit
# code at the end (see tests/smoke/check-smoke.sh). Consumers should `set -u`
# themselves; the lib already sets it for its own bookkeeping.
set -u

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"          # <repo>/tests/common
PROJECT_DIR="$(dirname "$(dirname "$LIB_DIR")")"                 # <repo>
FAILURES="${FAILURES:-0}"

# Colours — disabled when output is not a TTY or NO_COLOR is set.
RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'
YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'
if [ -n "${NO_COLOR:-}" ] || [ ! -t 1 ]; then
    RED=''; GREEN=''; CYAN=''; YELLOW=''; BOLD=''; NC=''
fi

die()  { printf "${RED}[ERROR]${NC} %s\n" "$*" >&2; exit 1; }
info() { printf "${CYAN}[%s]${NC}\n" "$*"; }

# note — print a section banner.
note() { printf "\n${BOLD}${CYAN}== %s${NC}\n" "$*"; }

# pass / fail — record a check result; fail() increments FAILURES so a suite
# can tolerate individual failures and exit non-zero at the end instead of
# aborting on the first one.
pass() { printf "  ${GREEN}[PASS]${NC} %s\n" "$*"; }
fail() { printf "  ${RED}[FAIL]${NC} %s\n" "$*"; FAILURES=$((FAILURES + 1)); }

# ── Artifact discovery ───────────────────────────────────────────────
# resolve_artifact_dir [dir] — print the release artifact directory.
# $1 overrides the search; otherwise ./release-artifacts (CI layout) then
# target/release (local build layout).
resolve_artifact_dir() {
    local dir="${1:-}"
    if [ -z "$dir" ]; then
        if [ -d release-artifacts ]; then
            dir="release-artifacts"
        else
            dir="target/release"
        fi
    fi
    printf '%s' "$dir"
}

# detect_platform — set OS, ARCH, LIB_NAME and CLI_NAME for the current host.
# Returns non-zero (printing to stderr) on unsupported platforms.
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"
    case "$OS" in
        Linux)  LIB_NAME="libdologger_core.so";   CLI_NAME="dologctl" ;;
        Darwin) LIB_NAME="libdologger_core.dylib"; CLI_NAME="dologctl" ;;
        *) printf '%s\n' "Unsupported OS: $OS" >&2; return 1 ;;
    esac
}

# resolve_cli dir — print the dologctl path: the CI layout name
# (dologctl-<os>-<arch>) first, then the plain name in the artifact dir.
# Call detect_platform first.
resolve_cli() {
    local dir="$1"
    local ci="$dir/dologctl-${OS,,}-${ARCH}"
    if [ -x "$ci" ]; then printf '%s' "$ci"; return; fi
    if [ -x "$dir/$CLI_NAME" ]; then printf '%s' "$dir/$CLI_NAME"; return; fi
}

# resolve_lib dir — print the core shared-library path in the artifact dir,
# by exact name first, then a shallow find for the CI-layout subpath.
# Call detect_platform first.
resolve_lib() {
    local dir="$1"
    if [ -f "$dir/$LIB_NAME" ]; then printf '%s' "$dir/$LIB_NAME"; return; fi
    find "$dir" -maxdepth 1 -name "$LIB_NAME" | head -1
}

# find_python — print a usable Python interpreter (python3 preferred).
find_python() {
    command -v python3 || command -v python || true
}
