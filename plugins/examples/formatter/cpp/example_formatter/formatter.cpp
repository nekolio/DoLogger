/**
 * Example Formatter plugin for DoLogger — C++ implementation.
 *
 * This formatter demonstrates the DoLogger C ABI plugin interface.
 * It formats log records as simple human-readable text lines.
 *
 * # Build (Linux)
 *
 *   g++ -shared -fPIC -o dologger-plugin-formatter-cpp.so \
 *       formatter.cpp \
 *       -I../../../../../core/include
 *
 * # Build (macOS)
 *
 *   g++ -shared -fPIC -o dologger-plugin-formatter-cpp.dylib \
 *       formatter.cpp \
 *       -I../../../../../core/include
 *
 * # C ABI symbols exported (via extern "C")
 *
 *   plugin_query(core_abi_version) → returns pointer to static PluginInfo
 *   plugin_init(config)             → stores format template, returns 0
 *   plugin_shutdown()               → cleanup, returns 0
 */

#include "dologger_core.h"   /* core ABI types, PluginInfo, Formatter VTable */

#include <cstdio>             /* snprintf */
#include <cstring>            /* strlen, strncpy */

/* =========================================================================
 * Plugin state
 * ======================================================================== */

namespace {

/** Log level name lookup table. */
const char *const kLevelNames[] = {
    "TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL", "AUDIT"
};

constexpr size_t kMaxLevelNameLen = 5;   /* "TRACE" = 5 */
constexpr size_t kMaxFormatLen    = 256; /* max format template length */

/** Stored format template (default: simple "[LEVEL] message"). */
char g_format_template[kMaxFormatLen] = "[{level}] {message}";

}  // namespace

/* =========================================================================
 * Formatter format function
 * ======================================================================== */

/**
 * @brief Format a log record into the output buffer.
 *
 * Writes a simple text representation of the record into `buf->data`.
 * The output is a null-terminated UTF-8 string.
 *
 * @param rec     Opaque record handle (unused — simplified example).
 * @param buf     Output buffer to write formatted text into.
 * @param config  Pointer to an int containing the record's level.
 * @return        0 on success, non-zero on error.
 *
 * C ABI contract for formatters:
 *   - Write formatted output to buf->data (pre-allocated by core).
 *   - Update buf->len to the number of bytes written (excluding null).
 *   - Do NOT exceed buf->capacity.
 *   - Return 0 for success.
 */
static int format(const dologger_record_handle_t *rec,
                  dologger_output_buffer_t *buf, void *config)
{
    (void)rec;  /* unused in this simplified example */

    if (buf == nullptr || buf->data == nullptr || buf->capacity == 0) {
        return -1;
    }

    /* Read the record level from config pointer. */
    int level = DO_LOG_INFO;  /* default */
    if (config != nullptr) {
        level = *(const int *)config;
    }
    if (level < DO_LOG_TRACE) level = DO_LOG_TRACE;
    if (level > DO_LOG_AUDIT) level = DO_LOG_AUDIT;

    const char *level_name = kLevelNames[level];

    /*
     * Build a formatted line using the stored template.
     * For simplicity we substitute {level} and {message} manually.
     * The message is hard-coded here; a real formatter reads it
     * from the record handle via the field access API.
     */
    int written = snprintf(
        reinterpret_cast<char *>(buf->data),
        buf->capacity,
        "[%-*s] %s",
        static_cast<int>(kMaxLevelNameLen), level_name,
        "(log message)"   /* placeholder — real impl reads from record */
    );

    if (written < 0) {
        return -1;   /* encoding error */
    }

    /* Clamp to available capacity (snprintf includes null terminator). */
    if (static_cast<size_t>(written) >= buf->capacity) {
        buf->len = buf->capacity - 1;
    } else {
        buf->len = static_cast<size_t>(written);
    }

    return 0;
}

/* =========================================================================
 * Formatter VTable — static, pointed to by PluginInfo
 * ======================================================================== */

static dologger_formatter_vtable_t g_vtable = {
    .format = format
};

/* =========================================================================
 * Plugin name — read-only string literal
 * ======================================================================== */

static const char g_plugin_name[] = "cpp-example-formatter";

/* =========================================================================
 * PluginInfo — static, returned by plugin_query()
 *
 * All fields must match dologger_plugin_info_t in dologger_core.h:
 *   name, version, abi_version, phase, vtable
 * ======================================================================== */

static dologger_plugin_info_t g_plugin_info = {
    .name        = g_plugin_name,
    .version     = 0x000001,                   /* 0.0.1   */
    .abi_version = 0x000001,                   /* 0.0.1   */
    .phase       = DO_LOG_PHASE_FORMATTING,    /* 0x0010   */
    .vtable      = &g_vtable
};

/* =========================================================================
 * C ABI exports — all three symbols use extern "C" for C linkage
 * ======================================================================== */

extern "C" {

dologger_plugin_info_t *plugin_query(uint32_t core_abi_version)
{
    /*
     * A production plugin should verify the core's ABI version
     * and return NULL if incompatible.
     */
    (void)core_abi_version;
    return &g_plugin_info;
}

int plugin_init(const void *config)
{
    if (config == nullptr) {
        return 0;   /* use default format template */
    }

    const char *str = static_cast<const char *>(config);
    size_t len = std::strlen(str);
    if (len == 0) {
        return 0;
    }

    /* Store a copy of the format template. */
    if (len >= kMaxFormatLen) {
        len = kMaxFormatLen - 1;
    }
    std::strncpy(g_format_template, str, len);
    g_format_template[len] = '\0';

    return 0;
}

int plugin_shutdown(void)
{
    /* Reset to default format template. */
    std::strncpy(g_format_template, "[{level}] {message}", kMaxFormatLen - 1);
    g_format_template[kMaxFormatLen - 1] = '\0';
    return 0;
}

}  // extern "C"
