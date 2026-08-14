/*
 * Minimal DoLogger host application — C ABI usage example.
 *
 * Demonstrates the three-step host lifecycle:
 *   dologger_init()  ->  dologger_log()  ->  dologger_shutdown()
 *
 * Build (after `cargo build --release` in the repo root):
 *   cc -I../../core/include main.c \
 *      -L../../target/release -ldologger_core -o dologger-c-example
 *
 * Or via CMake (see examples/c/CMakeLists.txt).
 */

#include <stdio.h>
#include <string.h>

#include "dologger_core.h"

int main(void) {
    dologger_error_t err;
    memset(&err, 0, sizeof(err));

    printf("DoLogger core version: %s\n", dologger_version());

    /* NULL config path => auto-discovery (./dologger.toml, .dologger.toml). */
    dologger_handle_t *handle = dologger_init(NULL, &err);
    if (handle == NULL) {
        fprintf(stderr, "dologger_init failed (code=%d): %s\n", err.code, err.message);
        return 1;
    }

    dologger_record_params_t params;
    memset(&params, 0, sizeof(params));
    params.level = DO_LOG_INFO;
    params.message = "Hello from the C host example";
    params.domain = "examples.c";

    if (dologger_log(handle, &params) != DO_LOG_OK) {
        fprintf(stderr, "dologger_log failed\n");
    }

    params.level = DO_LOG_AUDIT;
    params.message = "C host example: audit record (signed + WORM)";
    if (dologger_log(handle, &params) != DO_LOG_OK) {
        fprintf(stderr, "dologger_log(audit) failed\n");
    }

    dologger_shutdown(handle);
    printf("shutdown complete\n");
    return 0;
}
