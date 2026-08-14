# DoLogger — C Adapter

Thin convenience layer over the core C ABI for C/C++ host applications.

**Single source of truth:** [core/include/dologger_core.h](../../core/include/dologger_core.h).
This adapter adds an owned-handle struct and one-shot helpers but never
redefines a type, error code, or function signature from the core header.

## What's here

| File | Purpose |
|:-:|:-:|
| [dologger_adapter.h](dologger_adapter.h) | `DologgerLogger` + `dologger_logger_init/log/shutdown` inline helpers |

## Usage

```c
#include "dologger_adapter.h"   /* pulls in dologger_core.h */

int main(void) {
    DologgerLogger logger;
    if (!dologger_logger_init(&logger, NULL)) {
        return 1;               /* logger.err.code / .message hold details */
    }
    dologger_logger_log(&logger, DO_LOG_INFO, "hello from C");
    dologger_logger_log(&logger, DO_LOG_AUDIT, "signed audit record");
    dologger_logger_shutdown(&logger);
    return 0;
}
```

Compile (after `cargo build --release` at the repo root):

```bash
cc -I<repo>/core/include -I<repo>/adapters/c app.c \
   -L<repo>/target/release -ldologger_core
```

If you only need the raw ABI, include `dologger_core.h` directly and skip this
adapter — it exists purely for ergonomics, not as an abstraction boundary.
