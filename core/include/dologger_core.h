/**
 * @file dologger_core.h
 * @brief DoLogger Core Engine — Public C ABI Header
 *
 * This header declares all types, constants, and function signatures
 * required to integrate the DoLogger logging engine into any C-compatible
 * host application.
 *
 * @version 0.0.1
 * @date 2026-08-11
 *
 * # Usage
 *
 * Include this header in your project and link against `libdologger_core`
 * (or `dologger_core.dll` on Windows).
 *
 * # ABI Stability
 *
 * The functions and types declared here follow semantic versioning.
 * Breaking changes will be accompanied by a major version bump.
 * Fields marked as `_reserved` must be zero-filled and ignored.
 */

#ifndef DOLOGGER_CORE_H
#define DOLOGGER_CORE_H

#include <stdint.h>
#include <stddef.h>   /* size_t */

#ifdef __cplusplus
extern "C" {
#endif

/* =========================================================================
 * Platform detection & DLL export/import macros
 * ======================================================================== */

#if defined(_WIN32) || defined(_WIN64)
#  ifdef DOLOGGER_CORE_BUILDING
#    define DOLOGGER_API __declspec(dllexport)
#  else
#    define DOLOGGER_API __declspec(dllimport)
#  endif
#else
#  define DOLOGGER_API __attribute__((visibility("default")))
#endif

/* =========================================================================
 * 128-bit unsigned integer
 * ======================================================================== */

/**
 * @brief 128-bit unsigned integer used for record IDs and timestamps.
 *
 * Represented as a struct of two 64-bit halves to guarantee C ABI
 * compatibility across all supported platforms and compilers.
 */
typedef struct {
    uint64_t hi;  /**< High 64 bits */
    uint64_t lo;  /**< Low 64 bits */
} dologger_uint128_t;

/* =========================================================================
 * Log levels
 * ======================================================================== */

/** @brief Log severity levels. */
typedef enum {
    DO_LOG_TRACE = 0,  /**< Trace-level debugging information */
    DO_LOG_DEBUG = 1,  /**< Debug information */
    DO_LOG_INFO  = 2,  /**< Informational message */
    DO_LOG_WARN  = 3,  /**< Warning condition */
    DO_LOG_ERROR = 4,  /**< Error condition */
    DO_LOG_FATAL = 5,  /**< Fatal error, system may be unstable */
    DO_LOG_AUDIT = 6   /**< Non-repudiable audit record */
} dologger_level_t;

/* =========================================================================
 * Error codes
 * ======================================================================== */

/**
 * @brief Error code constants returned by DoLogger API functions.
 *
 * The code space mirrors the *journey of a record* through the engine, so a
 * value's high nibble tells the phase in which the failure surfaced. See the
 * module docs of `core/src/error.rs` and the reference table
 * `docs/*/guides/ErrorCodesReference.md` for the full catalog. All codes are
 * negative; 0 (`DO_LOG_OK`) means success. Plugin-defined codes live in the
 * high-bit range `-0x80000000` and below and are passed through untouched.
 */
typedef enum {
    /* --- General / API (0x01xx): caller-boundary checks --- */
    DO_LOG_OK                        =  0,       /**< Success */
    DO_LOG_ERR_INVALID_ARG           = -0x0101,  /**< Invalid argument */
    DO_LOG_ERR_NOT_SUPPORTED         = -0x0102,  /**< Not supported on this platform */
    DO_LOG_ERR_NOT_INITIALIZED       = -0x0103,  /**< Core not initialized */
    DO_LOG_ERR_ALREADY_INITIALIZED   = -0x0104,  /**< Core already initialized */
    DO_LOG_ERR_OUT_OF_MEMORY         = -0x0105,  /**< Memory allocation failure */
    DO_LOG_ERR_BUFFER_TOO_SMALL      = -0x0106,  /**< Buffer too small for the result */
    DO_LOG_ERR_TIMEOUT               = -0x0107,  /**< Operation timed out */
    DO_LOG_ERR_INTERNAL              = -0x0108,  /**< Generic internal error */
    DO_LOG_ERR_INIT_FAILED           = -0x0109,  /**< Engine init internal fatal error */

    /* --- Configuration (0x02xx): load / parse / validate / merge / reload --- */
    DO_LOG_ERR_CONFIG_NOT_FOUND      = -0x0201,  /**< Config file not found */
    DO_LOG_ERR_CONFIG_PERMISSION     = -0x0202,  /**< Config file permission denied */
    DO_LOG_ERR_CONFIG_PARSE          = -0x0203,  /**< Config parse (TOML syntax) error */
    DO_LOG_ERR_CONFIG_VALIDATION     = -0x0204,  /**< Config semantic validation failed */
    DO_LOG_ERR_CONFIG_MERGE          = -0x0205,  /**< Config merge conflict (domain inheritance) */
    DO_LOG_ERR_CONFIG_HOT_RELOAD_FAILED = -0x0206, /**< Hot reload failed; old config kept */
    DO_LOG_ERR_CONFIG_HASH_MISMATCH  = -0x0207,  /**< Hot reload config hash mismatch */
    DO_LOG_ERR_CONFIG_HOT_RELOAD_INVALID = -0x0208, /**< New reload config failed validation */
    DO_LOG_ERR_CONFIG_RESTART_REQUIRED = -0x0209, /**< Protected config changes require restart */

    /* --- Plugin (0x03xx): registry and runtime --- */
    DO_LOG_ERR_PLUGIN_NOT_FOUND      = -0x0301,  /**< Plugin not found */
    DO_LOG_ERR_PLUGIN_LOAD_FAILED    = -0x0302,  /**< Plugin load failed (link/missing symbol) */
    DO_LOG_ERR_PLUGIN_MANIFEST_INVALID = -0x0303, /**< Invalid manifest */
    DO_LOG_ERR_PLUGIN_VERSION_MISMATCH = -0x0304, /**< Plugin version incompatible with ABI */
    DO_LOG_ERR_PLUGIN_ABI            = -0x0305,  /**< Plugin ABI incompatible with core */
    DO_LOG_ERR_PLUGIN_DEPENDENCY_MISSING = -0x0306, /**< Missing dependency */
    DO_LOG_ERR_PLUGIN_LOCK_MISMATCH  = -0x0307,  /**< Lock file mismatch (deterministic load) */
    DO_LOG_ERR_PLUGIN_SIGNATURE_INVALID = -0x0308, /**< Bad plugin signature */
    DO_LOG_ERR_MISSING_CAPABILITY    = -0x0309,  /**< Capability required, no provider */
    DO_LOG_ERR_CIRCULAR_DEPENDENCY   = -0x030A,  /**< Circular dependency in plugin graph */
    DO_LOG_ERR_TOKEN_EXCEEDED_DEPTH  = -0x030B,  /**< Cross-plugin token chain depth exceeded */
    DO_LOG_ERR_CALL_DEADLOCK         = -0x030C,  /**< Cross-plugin call deadlock (cyclic wait) */
    DO_LOG_ERR_STATE_FORMAT_UNSUPPORTED = -0x030D, /**< Plugin state format version unsupported */
    DO_LOG_ERR_STATE_ROLLBACK_REJECTED = -0x030E, /**< State migration rejected rollback (epoch) */
    DO_LOG_ERR_STATE_MIGRATE_FAILED  = -0x030F,  /**< Plugin state serialize/deserialize failed */

    /* --- Record / Field (0x04xx) --- */
    DO_LOG_ERR_RECORD_INVALID        = -0x0401,  /**< Record invalid state */
    DO_LOG_ERR_FIELD_NOT_FOUND       = -0x0402,  /**< Field not found */
    DO_LOG_ERR_FIELD_PERMISSION_DENIED = -0x0403, /**< Field access denied (Ring permission) */
    DO_LOG_ERR_FIELD_TYPE_MISMATCH   = -0x0404,  /**< Field type mismatch */
    DO_LOG_ERR_FIELD_DEPENDENCY_NOT_MET = -0x0405, /**< Required field not provided earlier */
    DO_LOG_ERR_RECORD_INVALID_ENCODING = -0x0406, /**< Legacy text input is not valid UTF-8 */

    /* --- Buffer / Pipeline (0x05xx): ingest, backpressure --- */
    DO_LOG_ERR_BUFFER_FULL           = -0x0501,  /**< Ring buffer full, drop/block forbidden */
    DO_LOG_ERR_PIPELINE_STAGE        = -0x0502,  /**< Pipeline stage error */
    DO_LOG_ERR_AUDIT_QUEUE_FULL      = -0x0503,  /**< Audit-domain queue full (no-drop policy) */

    /* --- Signature / Audit chain (0x06xx) --- */
    DO_LOG_ERR_SIGN_FAILED           = -0x0601,  /**< Signature generation failed */
    DO_LOG_ERR_VERIFY_FAILED         = -0x0602,  /**< Signature verification failed */
    DO_LOG_ERR_LSN_CHAIN_BROKEN      = -0x0603,  /**< LSN chain broken (tampering) */
    DO_LOG_ERR_LSN_GAP_DETECTED      = -0x0604,  /**< LSN gap detected (reorder window) */
    DO_LOG_ERR_KEY_NOT_AVAILABLE     = -0x0605,  /**< Key not available for signing */
    DO_LOG_ERR_KEY_PROVIDER_FAILED   = -0x0606,  /**< KeyProvider plugin operation failed */
    DO_LOG_ERR_AUDIT_DROP_FORBIDDEN  = -0x0607,  /**< AUDIT domain configured with drop policy */
    DO_LOG_ERR_AUDIT_CALLBACK_ONLY   = -0x0608,  /**< AUDIT domain has only a callback sink */
    DO_LOG_ERR_AUDIT_NO_PERSISTENT_SINK = -0x0609, /**< AUDIT domain lacks a persistent sink */

    /* --- Security / Sandbox (0x07xx): plugin execution protection --- */
    DO_LOG_ERR_SANDBOX_INIT_FAILED   = -0x0701,  /**< Sandbox init failed */
    DO_LOG_ERR_SANDBOX_VIOLATION     = -0x0702,  /**< Sandbox policy violation (syscall blocked) */
    DO_LOG_ERR_UNTRUSTED_PLUGIN      = -0x0703,  /**< Unsigned plugin in production mode */

    /* --- Sink / IO (0x08xx): local and shared-memory output --- */
    DO_LOG_ERR_SINK_WRITE_FAILED     = -0x0801,  /**< Sink write failed (full/partial) */
    DO_LOG_ERR_SINK_CONNECTION_FAILED = -0x0802, /**< Sink failed to connect its target */
    DO_LOG_ERR_SINK_CONNECTION_LOST  = -0x0803,  /**< Sink connection lost after establishment */
    DO_LOG_ERR_SINK_FORMAT_INVALID   = -0x0804,  /**< Sink output format config invalid */
    DO_LOG_ERR_SINK_CONFIG_INVALID   = -0x0805,  /**< Sink config rejected (e.g. block policy) */
    DO_LOG_ERR_SINK_NO_FALLBACK      = -0x0806,  /**< Sink does not support fallback chain */
    DO_LOG_ERR_CALLBACK_TIMEOUT      = -0x0807,  /**< Callback sink host invocation timed out */
    DO_LOG_ERR_WORM_WRITE_FAILED     = -0x0808,  /**< WORM write failed (disk full/permission) */
    DO_LOG_ERR_SHM_INIT_FAILED       = -0x0809,  /**< Shared-memory create/map failed */
    DO_LOG_ERR_SHM_RING_FULL         = -0x080A,  /**< Shared-memory ring buffer full */
    DO_LOG_ERR_AUDIT_SHM_FORBIDDEN   = -0x080B,  /**< sink_shm forbidden for AUDIT domain */

    /* --- Network / Remote (0x09xx): remote sinks --- */
    DO_LOG_ERR_CIRCUIT_OPEN          = -0x0901,  /**< Remote-sink circuit breaker OPEN */
    DO_LOG_ERR_TLS_FAILED            = -0x0902,  /**< TLS handshake/certificate failure */
    DO_LOG_ERR_SASL_FAILED           = -0x0903,  /**< SASL authentication failure */
    DO_LOG_ERR_REMOTE_TIMEOUT        = -0x0904,  /**< Remote sink operation timed out */

    /* --- Resource / Quota (0x0Axx) --- */
    DO_LOG_ERR_QUOTA_MEMORY_EXCEEDED = -0x0A01,  /**< Memory quota exceeded */
    DO_LOG_ERR_QUOTA_CPU_EXCEEDED    = -0x0A02,  /**< CPU quota exceeded */
    DO_LOG_ERR_RECURSION_DEPTH_EXCEEDED = -0x0A03, /**< Logging recursion depth exceeded */

    /* --- Compliance (0x0Bxx) --- */
    DO_LOG_ERR_COMPLIANCE_VIOLATION  = -0x0B01,  /**< Compliance violation (non-downgradable) */
    DO_LOG_ERR_AUDIT_DURABILITY_INSUFFICIENT = -0x0B02, /**< AUDIT sink durability < MEDIA */

    /* --- Clock / Time safety (0x0Cxx) --- */
    DO_LOG_ERR_TIME_BACKWARD         = -0x0C01,  /**< Monotonic clock jumped backward */

    /* --- SIF / Serialization (0x0Dxx) --- */
    DO_LOG_ERR_SIF_INVALID           = -0x0D01,  /**< SIF frame malformed / failed verification */
    DO_LOG_ERR_FATAL                 = -0x0E01  /**< Fatal engine condition */,

    /* --- Internal / Fatal (0x0Exx): engine-fatal conditions.
     *     Plugin-defined codes use the high-bit range 0x80000000-0xFFFFFFFF
     *     and are passed through without core interpretation. --- */
} dologger_error_code_t;

/* =========================================================================
 * Structured error type
 * ======================================================================== */

/**
 * @brief Error information populated by DoLogger API functions.
 *
 * When a function returns an error, this struct provides a human-readable
 * message and source location for diagnostics.
 */
typedef struct {
    int32_t  code;                  /**< Error code (@see dologger_error_code_t) */
    char     message[256];          /**< Human-readable error message */
    char     source_file[128];      /**< Source file where error originated */
    uint32_t source_line;           /**< Source line number */
    uint8_t  _reserved[12];         /**< Reserved for future use (zero-filled) */
} dologger_error_t;

/* =========================================================================
 * Domain event (sysmon)
 * ======================================================================== */

/** @brief Severity levels for domain events. */
typedef enum {
    DO_LOG_EVENT_DEBUG     = 0,  /**< Debug information */
    DO_LOG_EVENT_INFO      = 1,  /**< Informational, normal operation */
    DO_LOG_EVENT_WARN      = 2,  /**< Warning, may need attention */
    DO_LOG_EVENT_ERROR     = 3,  /**< Error, requires investigation */
    DO_LOG_EVENT_CRITICAL  = 4,  /**< Critical failure, immediate action */
    DO_LOG_EVENT_EMERGENCY = 5   /**< Emergency, system may be unstable */
} dologger_event_severity_t;

/**
 * @brief Structured domain event for diagnostics and audit.
 */
typedef struct {
    int32_t  error_code;            /**< Error code or 0 for info events */
    char     category[32];          /**< Event category (e.g., "config", "plugin") */
    char     description[512];      /**< Human-readable description */
    uint64_t timestamp_ms;          /**< Monotonic milliseconds since engine init */
    uint8_t  severity;              /**< @see dologger_event_severity_t */
    uint8_t  _reserved[7];          /**< Reserved for future use */
} dologger_domain_event_t;

/* =========================================================================
 * Record parameters (for log submission)
 * ======================================================================== */

/**
 * @brief Parameters for creating a log record.
 *
 * Passed to dologger_log() to describe the log entry. Only `level` and
 * `message` are required; all other fields are optional.
 */
typedef struct {
    dologger_level_t level;         /**< Log severity level */
    const char      *message;       /**< Log message (UTF-8, null-terminated) */

    /* Source location (optional, set to NULL or 0 to omit) */
    const char      *source_file;
    const char      *source_function;
    uint32_t         source_line;
    uint32_t         source_column;

    /* Context (optional) */
    const char      *domain;        /**< Logger domain name (NULL = default) */
    const char      *user_id;
    const char      *session_id;
    const char      *request_id;

    uint8_t          _reserved[16]; /**< Reserved, must be zero-filled */
} dologger_record_params_t;

/* =========================================================================
 * Opaque handle types (forward declarations)
 * ======================================================================== */

/** @brief Opaque handle to an initialized DoLogger instance. */
typedef struct dologger_handle dologger_handle_t;

/** @brief Opaque handle to a log record slot. */
typedef struct dologger_record dologger_record_t;

/* =========================================================================
 * Core lifecycle API
 * ======================================================================== */

/**
 * @brief Initialize the DoLogger core engine.
 *
 * Searches for and loads configuration, initializes the ring buffer,
 * object pool, background pipeline threads, and loads configured plugins.
 *
 * @param config_path  Path to config file, or NULL for auto-discovery.
 * @param err          Error output (must not be NULL on first call).
 * @return             Opaque handle on success, NULL on failure.
 *
 * @note Must be called exactly once per process. Subsequent calls return
 *       DO_LOG_ERR_ALREADY_INITIALIZED.
 */
DOLOGGER_API dologger_handle_t *dologger_init(
    const char        *config_path,
    dologger_error_t  *err
);

/**
 * @brief Submit a log record to the pipeline.
 *
 * The record is pushed into the lock-free ring buffer and returns
 * immediately. Background threads handle filtering, processing,
 * formatting, and sink output asynchronously.
 *
 * @param handle  Engine handle from dologger_init().
 * @param params  Record parameters (level, message, optional context).
 * @return        DO_LOG_OK (0) on success, negative error code on failure.
 */
DOLOGGER_API int32_t dologger_log(
    dologger_handle_t             *handle,
    const dologger_record_params_t *params
);

/**
 * @brief Gracefully shut down the DoLogger engine.
 *
 * Stops accepting new log submissions, drains the pipeline,
 * flushes all sinks, and frees all resources.
 *
 * @param handle  Engine handle from dologger_init().
 */
DOLOGGER_API void dologger_shutdown(
    dologger_handle_t *handle
);

/* =========================================================================
 * Error query API
 * ======================================================================== */

/**
 * @brief Get the last error from a DoLogger handle.
 *
 * Thread-safe: returns the last error for the calling thread.
 *
 * @param handle  Engine handle.
 * @param err     Output: populated with the last error information.
 * @return        DO_LOG_OK on success, or error code.
 */
DOLOGGER_API int32_t dologger_get_last_error(
    const dologger_handle_t *handle,
    dologger_error_t        *err
);

/**
 * @brief Copy the locale-independent lookup key for an error code.
 *
 * Applications can map this key through their own message catalog without
 * parsing human-readable error text. The result excludes the null terminator.
 *
 * @param code      DoLogger error code.
 * @param buffer    Destination buffer for the key.
 * @param capacity  Destination capacity in bytes, including the terminator.
 * @return Key length on success, or DO_LOG_ERR_BUFFER_TOO_SMALL.
 */
DOLOGGER_API int32_t dologger_error_key(
    int32_t code,
    char    *buffer,
    size_t   capacity
);

/* =========================================================================
 * Field access API (record read/write by field name)
 * ======================================================================== */

/**
 * @brief Set a field value on a record by name.
 *
 * Field access is gated by the permission ring system:
 * - Ring 0 fields: read-only (this function returns DO_LOG_ERR_FIELD_PERMISSION_DENIED)
 * - Ring 1 fields: only HostInfoProvider can write
 * - Ring 2/3 fields: plugins can write based on trust level
 *
 * @param record      Target record.
 * @param field_name  Dot-separated field path (e.g., "user.id").
 * @param value       Null-terminated string value.
 * @param err         Error output.
 * @return            DO_LOG_OK on success.
 */
DOLOGGER_API int32_t dologger_field_set(
    dologger_record_t   *record,
    const char          *field_name,
    const char          *value,
    dologger_error_t    *err
);

/**
 * @brief Get a field value from a record by name.
 *
 * @param record      Source record.
 * @param field_name  Dot-separated field path (e.g., "record.id").
 * @param buffer      Output buffer for the field value (as string).
 * @param buffer_size Size of the output buffer.
 * @param err         Error output.
 * @return            Bytes written (excluding null terminator), or negative error.
 */
DOLOGGER_API int32_t dologger_field_get(
    const dologger_record_t *record,
    const char              *field_name,
    char                    *buffer,
    size_t                   buffer_size,
    dologger_error_t        *err
);

/* =========================================================================
 * Configuration API
 * ======================================================================== */

/**
 * @brief Load configuration from a TOML string at runtime.
 *
 * Merges with existing configuration. Hot-reload capable: if the engine
 * is already running, the new config is validated and atomically swapped.
 *
 * @param handle     Engine handle.
 * @param toml_data  Null-terminated TOML configuration string.
 * @param err        Error output.
 * @return           DO_LOG_OK on success.
 */
DOLOGGER_API int32_t dologger_config_load_from_string(
    dologger_handle_t  *handle,
    const char         *toml_data,
    dologger_error_t   *err
);

/* =========================================================================
 * Memory allocation API
 * ======================================================================== */

/**
 * @brief Allocate memory through DoLogger's internal allocator.
 *
 * Plugins MUST use this function for any memory that crosses the plugin
 * boundary (e.g., returned to the core or passed to other plugins).
 *
 * @param size  Number of bytes to allocate.
 * @return      Pointer to allocated memory, or NULL.
 */
DOLOGGER_API void *dologger_alloc(size_t size);

/**
 * @brief Free memory allocated by dologger_alloc().
 *
 * @param ptr  Pointer previously returned by dologger_alloc().
 */
DOLOGGER_API void dologger_free(void *ptr);

/* =========================================================================
 * Version query
 * ======================================================================== */

/**
 * @brief Get the DoLogger core version string.
 *
 * @return Null-terminated version string (e.g., "0.0.1").
 */
DOLOGGER_API const char *dologger_version(void);

/* =========================================================================
 * Plugin ABI — Phase identifiers
 * ======================================================================== */

#define DO_LOG_PHASE_PRE_FILTER  0x0001u
#define DO_LOG_PHASE_FILTER      0x0002u
#define DO_LOG_PHASE_ASSEMBLY    0x0004u
#define DO_LOG_PHASE_PROCESSING  0x0008u
#define DO_LOG_PHASE_FORMATTING  0x0010u
#define DO_LOG_PHASE_CONFIG      0x0040u
#define DO_LOG_PHASE_KEY         0x0080u
#define DO_LOG_PHASE_HOSTINFO    0x0100u
#define DO_LOG_PHASE_SYSCALL     0x0200u
#define DO_LOG_PHASE_POLICY      0x0400u  /* deprecated, same as PRE_FILTER */
#define DO_LOG_PHASE_FIELD_PROVIDER 0x0800u  /**< FieldProvider plugins (Stage 2) */

/* =========================================================================
 * Plugin ABI — PluginInfo + nine VTable types
 * ======================================================================== */

/** @brief Opaque handle to a log record passed through the plugin pipeline. */
typedef struct dologger_record_handle dologger_record_handle_t;

/** @brief Output buffer for Formatter plugins. */
typedef struct {
    uint8_t *data;
    size_t   len;
    size_t   capacity;
} dologger_output_buffer_t;

/**
 * @brief Host-accessor bridge handed to a plugin at plugin_init().
 *
 * A plugin (especially the official bundle, which is a separate cdylib that
 * statically links its own copy of the core rlib) cannot call the host's
 * exported dologger_field_get/set symbols directly. The host fills this table
 * with function pointers into the *live* engine and passes it via
 * dologger_host_init_t. The plugin copies the (all-function-pointer) struct
 * into its own static state for use from the pipeline hot path.
 *
 * Bump @ref DO_LOG_HOST_ACCESSORS_ABI on any layout change so a plugin built
 * against an older core can detect the mismatch at plugin_init().
 */
typedef struct {
    /** Read a field into `buffer`. Returns >=0 (byte count) or negative error.
     *  `buffer` is NUL-terminated on success. */
    int32_t (*field_get)(const void *record, const char *field_name,
                         char *buffer, size_t buffer_size);
    /** Write a field. Returns 0 on success or negative error
     *  (e.g. permission denied for a Ring-0 field). */
    int32_t (*field_set)(void *record, const char *field_name, const char *value);
    /** Allocate host-owned memory (used to grow formatter buffers). */
    void *(*alloc)(size_t size);
    /** Free memory previously returned by alloc(). */
    void (*free)(void *ptr);
    /** ABI version of the accessor table (@ref DO_LOG_HOST_ACCESSORS_ABI). */
    uint32_t abi_version;
} dologger_host_accessors_t;

/** @brief Current ABI version of dologger_host_accessors_t. */
#define DO_LOG_HOST_ACCESSORS_ABI 1u

/**
 * @brief Initialisation payload the host passes to plugin_init().
 *
 * Carries BOTH the host-accessor bridge (so the plugin can touch opaque
 * records) AND the plugin's JSON config string (NULL for defaults). The host
 * owns it for the duration of the plugin_init() call; the plugin copies
 * `.accessors` into its own static state and parses `.config_json` immediately.
 */
typedef struct {
    dologger_host_accessors_t accessors;
    const char              *config_json;   /**< Plugin JSON config, or NULL. */
} dologger_host_init_t;

/** @brief Plugin information returned by plugin_query(). */
typedef struct {
    const char *name;             /**< Unique plugin identifier (UTF-8) */
    uint32_t    version;          /**< Encoded binary-compat version */
    uint32_t    abi_version;      /**< Declared core ABI version (e.g. 0x010200 = 1.2.0) */
    uint32_t    phase;            /**< Mount point (@see DO_LOG_PHASE_*) */
    void       *vtable;           /**< Pointer to the VTable for this phase */
} dologger_plugin_info_t;

/** @brief Multi-phase plugin info list (for plugin_query_multi). */
typedef struct {
    uint32_t                  count;
    dologger_plugin_info_t  **infos;
} dologger_plugin_info_list_t;

/* --- (1) Filter VTable --- */
typedef struct {
    /** Return non-zero to drop the record. MUST NOT perform I/O. */
    int (*filter)(const dologger_record_handle_t *rec, void *config);
} dologger_filter_vtable_t;

/* --- (2) FieldProvider VTable --- */
typedef struct {
    /**
     * Provide fields to the record. Returns the number of fields added,
     * or -1 on error. MUST respect Ring permission limits.
     */
    int (*provide)(dologger_record_handle_t *rec, void *config);
} dologger_field_provider_vtable_t;

/* --- (3) Processor VTable --- */
typedef struct {
    /**
     * Process/transform the record (mask, enrich, sanitise).
     * Returns 0 on success, non-zero to discard the record.
     */
    int (*process)(dologger_record_handle_t *rec, void *config);
} dologger_processor_vtable_t;

/* --- (4) Formatter VTable --- */
typedef struct {
    /**
     * Format the record into SIF (Standard Intermediate Format).
     * Output is written to `buf`. Returns 0 on success.
     */
    int (*format)(const dologger_record_handle_t *rec,
                  dologger_output_buffer_t *buf, void *config);
} dologger_formatter_vtable_t;

/* --- (5) ConfigProvider VTable --- */
typedef struct {
    int         (*open)(void *instance, void *config);
    /** Returns a TOML string; caller (core) takes ownership. */
    const char *(*read_config)(void *instance);
    int         (*close)(void *instance);
} dologger_config_provider_vtable_t;

/* --- (6) KeyProvider VTable --- */
typedef struct {
    int (*open)(void *instance, void *config);
    /** Write the Ed25519 public key (32 bytes) to out_pubkey. */
    int (*get_public_key)(void *instance, uint8_t *out_pubkey, size_t *len);
    /**
     * Optional detached sign. If NULL, core signs internally.
     * Returns 0 and writes 64-byte signature to out_sig.
     */
    int (*sign_detached)(void *instance, const uint8_t *data, size_t len,
                         uint8_t *out_sig, size_t *sig_len);
    int (*close)(void *instance);
} dologger_key_provider_vtable_t;

/* --- (7) PolicyProvider VTable --- */
typedef struct {
    /**
     * Evaluate whether the record should continue.
     * Return 0 to allow, non-zero to drop (reason logged to sysmon).
     * MUST NOT read or modify field contents.
     */
    int (*evaluate)(const dologger_record_handle_t *rec);
} dologger_policy_provider_vtable_t;

/* --- (8) HostInfoProvider — reuses FieldProviderVTable --- */
typedef dologger_field_provider_vtable_t dologger_host_info_provider_vtable_t;

/* --- (9) SystemCallBroker VTable --- */
typedef struct {
    /**
     * Proxy a system call. Returns 0 on success, -ENOSYS for unknown ops.
     */
    int (*syscall_io)(int operation, void *args);
} dologger_syscall_broker_vtable_t;

/* =========================================================================
 * Plugin lifecycle symbols
 * ========================================================================
 * A library MUST export exactly ONE of the two query contracts:
 *
 *   1. Single-plugin libraries (third-party / standalone):
 *        plugin_query(core_abi_version)                     [required]
 *        plugin_init(config) / plugin_shutdown()            [required]
 *
 *   2. Bundle libraries (official plugins, one library hosts N plugins):
 *        plugin_query_multi(core_abi_version)               [required]
 *        plugin_init(config) / plugin_shutdown()            [required, fan-out]
 *
 * The bundle contract lets a single dynamic library ship every official
 * plugin instead of one plugin per file — see the `dologger-official-plugins`
 * crate. The host registers each entry of `dologger_plugin_info_list_t` from
 * one library handle.
 * ======================================================================== */

/**
 * @brief Query plugin capabilities (single-plugin libraries, required export).
 * @param core_abi_version  The core's ABI version for compatibility check.
 * @return PluginInfo pointer, or NULL if incompatible.
 */
typedef dologger_plugin_info_t *(*dologger_plugin_query_fn)(uint32_t core_abi_version);

/**
 * @brief Query every plugin hosted by a bundle library (required export).
 * @param core_abi_version  The core's ABI version for compatibility check.
 * @return Pointer to a static plugin info list owned by the library, or NULL
 *         if incompatible. The list is valid for the library's lifetime.
 */
typedef dologger_plugin_info_list_t *(*dologger_plugin_query_multi_fn)(
    uint32_t core_abi_version);

/**
 * @brief Initialize the plugin (required export).
 * @param config  Pointer to a `dologger_host_init_t` (carries the host-accessor
 *                bridge + plugin JSON config string), or NULL for defaults.
 * @return 0 on success.
 */
typedef int (*dologger_plugin_init_fn)(const void *config);

/**
 * @brief Shutdown the plugin (required export).
 * @return 0 on success.
 */
typedef int (*dologger_plugin_shutdown_fn)(void);

/* =========================================================================
 * Record lifecycle + SIF encode/decode/validate (C ABI)
 * ======================================================================== */

/**
 * @brief Create an un-pooled, zero-initialised record for a C host to populate
 *        (via dologger_field_set) and then encode with dologger_sif_encode_record.
 * @return Opaque record handle (a `dologger_record_t`, the same opaque type
 *         field_set/get operate on), or NULL on allocation failure.
 * @sa dologger_record_destroy
 */
DOLOGGER_API dologger_record_t *dologger_record_create(void);

/**
 * @brief Destroy a record previously returned by dologger_record_create().
 * @param record  Handle from dologger_record_create(); NULL is a no-op.
 */
DOLOGGER_API void dologger_record_destroy(dologger_record_t *record);

/**
 * @brief Validate a SIF frame's magic, version, and length without decoding.
 * @param frame      SIF byte buffer.
 * @param frame_len  Buffer length in bytes.
 * @param err        Out-param for a structured error.
 * @return DO_LOG_OK if structurally valid, else a negative DO_LOG_ERR_* code
 *         (typically DO_LOG_ERR_SIF_INVALID).
 */
DOLOGGER_API int32_t dologger_sif_validate_frame(const uint8_t *frame,
                                                 size_t frame_len,
                                                 dologger_error_t *err);

/**
 * @brief Encode a record into a complete SIF frame (magic + header + KV payload).
 * @param record   Record handle to encode.
 * @param out      Out-param receiving a host-owned buffer (allocated via
 *                 dologger_alloc) on success.
 * @param out_len  Out-param receiving the buffer length in bytes.
 * @param err      Out-param for a structured error.
 * @return DO_LOG_OK on success; the caller must free *out with dologger_free().
 *         Negative DO_LOG_ERR_* on failure (*out/*out_len left untouched).
 */
DOLOGGER_API int32_t dologger_sif_encode_record(const dologger_record_t *record,
                                                uint8_t **out, size_t *out_len,
                                                dologger_error_t *err);

/**
 * @brief Decode a SIF frame into a new record.
 * @param frame       SIF byte buffer.
 * @param frame_len   Buffer length in bytes.
 * @param out_record  Out-param receiving an opaque record handle on success
 *                    (release with dologger_record_destroy).
 * @param err         Out-param for a structured error.
 * @return DO_LOG_OK on success, else a negative DO_LOG_ERR_* code.
 */
DOLOGGER_API int32_t dologger_sif_decode_record(const uint8_t *frame,
                                                size_t frame_len,
                                                dologger_record_t **out_record,
                                                dologger_error_t *err);

#ifdef __cplusplus
}
#endif

#endif /* DOLOGGER_CORE_H */
