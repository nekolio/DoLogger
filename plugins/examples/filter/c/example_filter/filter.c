/**
 * Example Filter plugin for DoLogger — C implementation.
 *
 * This filter demonstrates the DoLogger C ABI plugin interface.
 * It drops log records below a configurable minimum severity level.
 *
 * # Build (Linux)
 *
 *   gcc -shared -fPIC -o dologger-plugin-filter-c.so filter.c \
 *       -I../../../../core/include
 *
 * # Build (macOS)
 *
 *   gcc -shared -fPIC -o dologger-plugin-filter-c.dylib filter.c \
 *       -I../../../../core/include
 *
 * # C ABI symbols exported
 *
 *   plugin_query(core_abi_version) → returns pointer to static PluginInfo
 *   plugin_init(config)             → parses min_level integer, returns 0
 *   plugin_shutdown()               → cleanup, returns 0
 */

#include "dologger_core.h"   /* core ABI types, constants, PluginInfo, Filter VTable */

#include <stdlib.h>          /* strtol */
#include <string.h>          /* strlen */

/* =========================================================================
 * Plugin state
 * ======================================================================== */

/** Minimum log level that passes the filter. Default: WARN (3). */
static int g_min_level = DO_LOG_WARN;

/* =========================================================================
 * Filter evaluate function
 * ======================================================================== */

/**
 * @brief Decide whether a log record should be kept.
 *
 * Evaluates the record against the configured minimum level.
 *
 * @param rec     Opaque record handle (unused in this simple example).
 * @param config  Pointer to the record's severity level (as int).
 * @return        1 if record level >= min_level (keep), 0 otherwise (drop).
 *
 * NOTE: This function uses an inverted convention compared to the core's
 * dologger_filter_vtable_t::filter (which returns non-zero to drop).
 * The intent here is to demonstrate an "evaluate-to-keep" semantic.
 * A production filter should use the core's convention: 0 = keep, non-zero = drop.
 */
static int evaluate(const dologger_record_handle_t *rec, void *config)
{
    (void)rec;   /* unused in this simplified example */

    if (config == NULL) {
        return 0;   /* no level info — drop */
    }

    int record_level = *(const int *)config;

    if (record_level >= g_min_level) {
        return 1;   /* keep */
    }
    return 0;       /* drop */
}

/* =========================================================================
 * Filter VTable — static, pointed to by PluginInfo
 * ======================================================================== */

static dologger_filter_vtable_t g_vtable = {
    .filter = evaluate
};

/* =========================================================================
 * Plugin name — read-only string literal lives in .rodata
 * ======================================================================== */

static const char g_plugin_name[] = "c-example-filter";

/* =========================================================================
 * PluginInfo — static, returned by plugin_query()
 * ======================================================================== */

static dologger_plugin_info_t g_plugin_info = {
    .name        = g_plugin_name,
    .version     = 0x000100,    /* 0.1.0  (major.minor.patch packed) */
    .abi_version = 0x000100,    /* 0.1.0  (core ABI this plugin targets) */
    .phase       = DO_LOG_PHASE_FILTER,   /* 0x0002 */
    .vtable      = &g_vtable
};

/* =========================================================================
 * C ABI: plugin_query — required export
 * ======================================================================== */

dologger_plugin_info_t *plugin_query(uint32_t core_abi_version)
{
    /*
     * A production plugin should verify compatibility:
     *   if (core_abi_version > g_plugin_info.abi_version) return NULL;
     *
     * For this example we always return the info struct.
     */
    (void)core_abi_version;
    return &g_plugin_info;
}

/* =========================================================================
 * C ABI: plugin_init — required export
 * ======================================================================== */

/**
 * @brief Parse configuration and prepare the plugin.
 *
 * Expects `config` to be a null-terminated string containing the minimum
 * log level as a decimal integer.  Example: "3" for WARN.
 *
 * If `config` is NULL or empty, the default (WARN) is kept.
 *
 * @param config  Opaque config pointer (null-terminated string).
 * @return        0 on success, negative on error.
 */
int plugin_init(const void *config)
{
    if (config == NULL) {
        return 0;   /* keep default */
    }

    const char *str = (const char *)config;
    size_t len = strlen(str);
    if (len == 0) {
        return 0;   /* keep default */
    }

    /* Parse integer from config string. */
    char *endptr = NULL;
    long val = strtol(str, &endptr, 10);

    if (endptr == str) {
        return -1;  /* parse error */
    }

    /* Clamp to valid range: TRACE(0) .. AUDIT(6). */
    if (val < DO_LOG_TRACE) val = DO_LOG_TRACE;
    if (val > DO_LOG_AUDIT) val = DO_LOG_AUDIT;

    g_min_level = (int)val;
    return 0;
}

/* =========================================================================
 * C ABI: plugin_shutdown — required export
 * ======================================================================== */

int plugin_shutdown(void)
{
    /* Reset state for a potential re-init. */
    g_min_level = DO_LOG_WARN;
    return 0;
}
