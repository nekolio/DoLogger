/**
 * @file c_abi_bench.c
 * @brief C ABI log-throughput benchmark for the DoLogger core.
 *
 * Loads libdologger_core at runtime — dlopen() on POSIX, LoadLibrary() on
 * Windows — so there is no link step and the same binary can be pointed at
 * any release artifact. This mirrors the artifact-driving model of
 * tests/smoke/c_abi_smoke.py, but from a compiled C host so the benchmark
 * measures the cross-language boundary itself.
 *
 * dologger_log() returns once the record is pushed into the lock-free ring
 * buffer; background pipeline threads filter, format and sink asynchronously.
 * The reported rate is therefore the C ABI submission fast path, not the
 * end-to-end persistence rate. Rust in-process Criterion benches covering the
 * Rust API live in core/benches/.
 *
 * The engine is started with config auto-discovery (dologger_init(NULL)),
 * so an operator can shape the pipeline — sinks, signatures, ring size —
 * through the standard dologger.toml mechanism instead of recompiling.
 *
 * Usage:
 *   c_abi_bench <libdologger_core> [records] [warmup]
 *
 *   records  timed submissions      (default 1000000)
 *   warmup   pre-timed submissions  (default 100000)
 *
 * Exit code 0 on success; non-zero on any failure (load, symbols, init,
 * or a failed submission).
 */

#define _POSIX_C_SOURCE 200809L   /* clock_gettime, dlsym */

#include "dologger_core.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ── Runtime loader shim ──────────────────────────────────────────────── */
#if defined(_WIN32) || defined(_WIN64)
#  include <windows.h>
#  define BENCH_DLOPEN(path)  ((void *)LoadLibraryA(path))
#  define BENCH_DLSYM(h, n)   ((void *)GetProcAddress((HMODULE)(h), (n)))
#  define BENCH_DLCLOSE(h)    ((void)FreeLibrary((HMODULE)(h)))
#else
#  include <dlfcn.h>
#  include <time.h>
#  define BENCH_DLOPEN(path)  dlopen((path), RTLD_NOW | RTLD_LOCAL)
#  define BENCH_DLSYM(h, n)   dlsym((h), (n))
#  define BENCH_DLCLOSE(h)    ((void)dlclose((h)))
#endif

/* ── Monotonic clock shim ─────────────────────────────────────────────── */
#if defined(_WIN32) || defined(_WIN64)
static double now_seconds(void) {
    LARGE_INTEGER freq, count;
    QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&count);
    return (double)count.QuadPart / (double)freq.QuadPart;
}
#else
static double now_seconds(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec * 1e-9;
}
#endif

/* ── C ABI entry points, resolved from the loaded library ─────────────── */
typedef dologger_handle_t *(*init_fn)(const char *, dologger_error_t *);
typedef int32_t (*log_fn)(dologger_handle_t *, const dologger_record_params_t *);
typedef void (*shutdown_fn)(dologger_handle_t *);
typedef const char *(*version_fn)(void);

static uint64_t parse_count(const char *s, uint64_t fallback) {
    char *end = NULL;
    uint64_t v = strtoull(s, &end, 10);
    return (end && *end == '\0' && v > 0) ? v : fallback;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr,
                "usage: %s <libdologger_core> [records] [warmup]\n"
                "  records  timed submissions (default 1000000)\n"
                "  warmup   pre-timed submissions (default 100000)\n",
                argv[0]);
        return 2;
    }

    const char *lib_path = argv[1];
    uint64_t records = argc >= 3 ? parse_count(argv[2], 1000000) : 1000000;
    uint64_t warmup  = argc >= 4 ? parse_count(argv[3], 100000)  : 100000;

    /* ── Load the core shared library and resolve the C ABI ──────────── */
    void *lib = BENCH_DLOPEN(lib_path);
    if (!lib) {
        fprintf(stderr, "[ERROR] cannot load core library: %s\n", lib_path);
        return 1;
    }
    init_fn      p_init     = (init_fn)BENCH_DLSYM(lib, "dologger_init");
    log_fn       p_log      = (log_fn)BENCH_DLSYM(lib, "dologger_log");
    shutdown_fn  p_shutdown = (shutdown_fn)BENCH_DLSYM(lib, "dologger_shutdown");
    version_fn   p_version  = (version_fn)BENCH_DLSYM(lib, "dologger_version");
    if (!p_init || !p_log || !p_shutdown) {
        fprintf(stderr, "[ERROR] core library is missing required dologger_* symbols\n");
        return 1;
    }
    if (p_version) {
        printf("dologger core version: %s\n", p_version());
    }

    /* ── Start the engine (config auto-discovery: dologger.toml or defaults) ── */
    dologger_error_t err;
    memset(&err, 0, sizeof(err));
    dologger_handle_t *handle = p_init(NULL, &err);
    if (!handle) {
        fprintf(stderr, "[ERROR] dologger_init failed (code=%d, msg=%s)\n",
                (int)err.code, err.message);
        return 1;
    }

    /* ── Fixed record template ────────────────────────────────────────── */
    dologger_record_params_t params;
    memset(&params, 0, sizeof(params));
    params.level           = DO_LOG_INFO;
    params.message         = "c-abi-bench pipeline throughput probe";
    params.source_file     = "c_abi_bench.c";
    params.source_function = "main";
    params.source_line     = 1;
    params.source_column   = 1;
    params.domain          = "bench";

    /* Warm up first so allocators, pools and pipeline threads are in steady
     * state before timing. Calls go through the resolved function pointers —
     * opaque external side effects the compiler cannot elide. */
    uint64_t i;
    int32_t rc;
    for (i = 0; i < warmup; i++) {
        rc = p_log(handle, &params);
        if (rc != DO_LOG_OK) {
            fprintf(stderr, "[ERROR] warmup submission %" PRIu64 " failed (rc=%d)\n", i, rc);
            p_shutdown(handle);
            return 1;
        }
    }

    double t0 = now_seconds();
    for (i = 0; i < records; i++) {
        rc = p_log(handle, &params);
        if (rc != DO_LOG_OK) {
            fprintf(stderr, "[ERROR] submission %" PRIu64 " failed (rc=%d)\n", i, rc);
            p_shutdown(handle);
            return 1;
        }
    }
    double elapsed = now_seconds() - t0;

    if (elapsed <= 0.0) {
        elapsed = 1e-12;
    }
    printf("%" PRIu64 " records in %.6f s — %.2f records/sec, %.2f ns/record\n",
           records, elapsed, (double)records / elapsed, elapsed * 1e9 / (double)records);

    p_shutdown(handle);
    BENCH_DLCLOSE(lib);
    return 0;
}
