#!/usr/bin/env bash
# common.sh — shared helpers for DoLogger build/setup scripts.
#
# Source from a script inside scripts/ (NOT scripts/lib/) with:
#   . "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/common.sh"
#
# Exports:
#   PROJECT_DIR  — absolute repo root
#   colour vars  — RED / GREEN / YELLOW / CYAN / BOLD / NC
#   die()        — print a red [ERROR] line to stderr and exit 1
#   info()       — print a cyan [label] line
#
# Scripts that source this SHOULD `cd "$PROJECT_DIR"` after sourcing so their
# relative paths resolve from the repo root no matter where they are invoked.
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"          # <repo>/scripts/lib
PROJECT_DIR="$(dirname "$(dirname "$LIB_DIR")")"                 # <repo>

# Colours
RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'
YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'

die()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }
info()  { echo -e "${CYAN}[$*]${NC}"; }
