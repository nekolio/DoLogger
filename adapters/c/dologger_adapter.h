/**
 * @file dologger_adapter.h
 * @brief DoLogger C adapter — thin convenience layer over the core C ABI.
 *
 * This header is the **thin adapter** for C host applications. It adds a
 * small amount of ergonomic sugar on top of the core C ABI (an owned handle
 * struct and one-shot init/log/shutdown helpers) but does **not** redefine any
 * ABI type or function. The single source of truth for types, error codes, and
 * function signatures remains `core/include/dologger_core.h`.
 *
 * # Usage
 *
 * Add `core/include` to your include path and link `libdologger_core`:
 *
 *     cc -I<repo>/core/include -I<repo>/adapters/c app.c -ldologger_core
 *
 * If you only need the raw ABI, include `dologger_core.h` directly and skip
 * this adapter entirely.
 */

#ifndef DOLOGGER_ADAPTER_H
#define DOLOGGER_ADAPTER_H

#include "dologger_core.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Owned DoLogger instance wrapping an opaque engine handle.
 *
 * Keeps the handle plus a reusable error buffer so callers don't have to
 * manage a `dologger_error_t` on the stack for every call.
 */
typedef struct {
    dologger_handle_t *handle;   /**< Engine handle, or NULL if not initialized */
    dologger_error_t   err;      /**< Last error (see dologger_logger_last_error) */
} DologgerLogger;

/**
 * @brief Initialize a logger (auto-discovers config when `config_path` is NULL).
 * @return 1 on success, 0 on failure (error details in `logger->err`).
 */
static inline int dologger_logger_init(DologgerLogger *logger, const char *config_path) {
    logger->handle = dologger_init(config_path, &logger->err);
    return logger->handle != NULL;
}

/**
 * @brief Submit a log record at `level` with `message`.
 * @return DO_LOG_OK (0) on success, negative error code on failure.
 */
static inline int32_t dologger_logger_log(
        DologgerLogger *logger, dologger_level_t level, const char *message) {
    if (logger->handle == NULL) {
        return DO_LOG_ERR_NOT_INITIALIZED;
    }
    dologger_record_params_t params = {0}; /* zero-fills pointers + _reserved */
    params.level = level;
    params.message = message;

    return dologger_log(logger->handle, &params);
}

/**
 * @brief Gracefully shut down and free the logger.
 */
static inline void dologger_logger_shutdown(DologgerLogger *logger) {
    if (logger->handle != NULL) {
        dologger_shutdown(logger->handle);
        logger->handle = NULL;
    }
}

#ifdef __cplusplus
}
#endif

#endif /* DOLOGGER_ADAPTER_H */
