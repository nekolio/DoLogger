# DoLogger — Shared Test Utilities

Home for test helpers reused across the suites in `tests/` and `core/tests/`.
The Rust suites live directly in `core/tests/` (auto-discovered); anything
they share is promoted here rather than duplicated per suite.

## Bash helper library — `lib.sh`

`lib.sh` mirrors `scripts/lib/common.sh` for the test tree. Shell-based
suites source it with:

```bash
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../common/lib.sh"
```

It provides `PROJECT_DIR`, colour vars, `note`/`pass`/`fail` check helpers
(`fail()` bumps a `FAILURES` counter), and release-artifact discovery
(`resolve_artifact_dir`, `detect_platform`, `resolve_cli`, `resolve_lib`,
`find_python`). Unlike `scripts/lib/common.sh` it deliberately does **not**
force `set -e`, so suites that count failures (like
[`tests/smoke/check-smoke.sh`](../smoke/check-smoke.sh)) can stay tolerant.

## Mock plugins (planned)

| Item | Purpose |
|:-:|:-:|
| `mock_filter.c` / `mock_processor.c` | Mock Filter/Processor plugins for pipeline tests |
| `mock_utils.h` | Common assertions and plugin-bundle helpers |

Add helpers here (and keep them platform-neutral) rather than duplicating
set-up logic in each suite.
