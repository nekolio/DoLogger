# DoLogger — Shared Test Utilities

Home for test helpers and mock plugins reused across the suites in `tests/`
(and copied/symlinked into `core/tests/` when needed).

**Status: skeleton.** Nothing lives here yet. Planned contents:

| Item | Purpose |
|:-:|:-|
| `mock_filter.c` / `mock_processor.c` | Mock Filter/Processor plugins for pipeline tests |
| `mock_utils.h` | Common assertions and plugin-bundle helpers |

Add helpers here (and keep them platform-neutral) rather than duplicating
set-up logic in each suite.
