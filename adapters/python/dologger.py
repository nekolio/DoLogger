"""
DoLogger Python Adapter — ctypes wrapper for libdologger_core.

Minimal FFI glue that loads the DoLogger shared library and exposes a
Pythonic ``DoLogger`` class with info/warn/error convenience methods.

Requires the DoLogger core library to be built and available in the
system library search path.

Example usage::

    from dologger import DoLogger

    log = DoLogger()               # auto-discover config
    log.info("Hello from Python")
    log.warn("Disk usage warning")
    log.error("Something went wrong")
    log.shutdown()

    # With a custom config file:
    log = DoLogger(config_path="/etc/dologger/config.toml")
    log.audit("User performed admin action")
    log.shutdown()

Build Requirements
------------------

Before using this module, build ``libdologger_core``::

    cd core/
    cargo build --release

On Linux, copy the shared library to a standard path or set
``DO_LOGGER_LIB_PATH``::

    export DO_LOGGER_LIB_PATH=./target/release/libdologger_core.so

On Windows, use the DLL environment variable or place the DLL alongside
this script.
"""

import ctypes
import os
import platform
from ctypes import (
    CDLL,
    POINTER,
    Structure,
    byref,
    c_char_p,
    c_int32,
    c_size_t,
    c_uint8,
    c_uint32,
    c_void_p,
)

# ---------------------------------------------------------------------------
# C type definitions (must match core/src/ffi.rs and core/include/dologger_core.h)
# ---------------------------------------------------------------------------


class DologgerError(Structure):
    """Matches `dologger_error_t` from the C ABI."""
    _fields_ = [
        ("code", c_int32),
        ("message", c_char * 256),
        ("source_file", c_char * 128),
        ("source_line", c_uint32),
        ("_reserved", c_uint8 * 12),
    ]


class DologgerLogParams(Structure):
    """Matches `dologger_log_params` as used by the `dologger_log` FFI entry point.

    The `dologger_log` symbol expects a struct with: level, message pointer,
    source_file pointer, source_line, and 16 bytes of reserved padding.
    """
    _fields_ = [
        ("level", c_uint8),
        ("message", c_char_p),
        ("source_file", c_char_p),
        ("source_line", c_uint32),
        ("_reserved", c_uint8 * 16),
    ]


# Opaque handle type
DologgerHandle = c_void_p

# ---------------------------------------------------------------------------
# Log level constants (must match record.rs LogLevel enum)
# ---------------------------------------------------------------------------

DO_LOG_TRACE = 0
DO_LOG_DEBUG = 1
DO_LOG_INFO = 2
DO_LOG_WARN = 3
DO_LOG_ERROR = 4
DO_LOG_FATAL = 5
DO_LOG_AUDIT = 6

# Error code
DO_LOG_OK = 0

# ---------------------------------------------------------------------------
# Library loading
# ---------------------------------------------------------------------------


def _find_library() -> str:
    """Locate the DoLogger shared library.

    Search order:
    1. ``DO_LOGGER_LIB_PATH`` environment variable.
    2. Platform-specific default names in the system search path.
    """
    env_path = os.environ.get("DO_LOGGER_LIB_PATH")
    if env_path:
        return env_path

    system = platform.system()
    if system == "Windows":
        return "dologger_core.dll"
    elif system == "Darwin":
        return "libdologger_core.dylib"
    else:
        return "libdologger_core.so"


# ---------------------------------------------------------------------------
# Logger class
# ---------------------------------------------------------------------------


class DoLogger:
    """Pythonic wrapper around the DoLogger C ABI.

    Create one instance per process.  Call :meth:`shutdown` before the
    process exits to flush all pending records.

    Parameters
    ----------
    config_path : str or None
        Path to a TOML configuration file, or ``None`` to use
        auto-discovery and hardcoded defaults.
    """

    def __init__(self, config_path: str | None = None):
        lib_path = _find_library()
        try:
            self._lib = CDLL(lib_path)
        except OSError as exc:
            raise RuntimeError(
                f"Cannot load DoLogger shared library '{lib_path}': {exc}\n"
                f"Build it first: cd core/ && cargo build --release\n"
                f"Or set DO_LOGGER_LIB_PATH to the full path of the shared library."
            ) from exc

        # Set function signatures for type safety
        self._lib.dologger_init.argtypes = [c_char_p, POINTER(DologgerError)]
        self._lib.dologger_init.restype = c_void_p

        self._lib.dologger_log.argtypes = [c_void_p, POINTER(DologgerLogParams)]
        self._lib.dologger_log.restype = c_int32

        self._lib.dologger_shutdown.argtypes = [c_void_p]
        self._lib.dologger_shutdown.restype = None

        self._lib.dologger_get_last_error.argtypes = [c_void_p, POINTER(DologgerError)]
        self._lib.dologger_get_last_error.restype = c_int32

        self._lib.dologger_version.argtypes = []
        self._lib.dologger_version.restype = c_char_p

        # Safely get the version string for diagnostics
        version_c = self._lib.dologger_version()
        self._version = version_c.decode("utf-8") if version_c else "unknown"

        # Initialize the engine
        c_config = c_char_p(config_path.encode("utf-8")) if config_path else None
        err = DologgerError()
        self._handle = self._lib.dologger_init(c_config, byref(err))

        if not self._handle:
            msg = err.message.decode("utf-8", errors="replace") if err.code else "unknown"
            raise RuntimeError(
                f"dologger_init() failed (code={err.code}): {msg}"
            )

    # -- Convenience methods ------------------------------------------------

    def trace(self, message: str) -> None:
        """Log at TRACE level."""
        self._log(DO_LOG_TRACE, message)

    def debug(self, message: str) -> None:
        """Log at DEBUG level."""
        self._log(DO_LOG_DEBUG, message)

    def info(self, message: str) -> None:
        """Log at INFO level."""
        self._log(DO_LOG_INFO, message)

    def warn(self, message: str) -> None:
        """Log at WARN level."""
        self._log(DO_LOG_WARN, message)

    def error(self, message: str) -> None:
        """Log at ERROR level."""
        self._log(DO_LOG_ERROR, message)

    def fatal(self, message: str) -> None:
        """Log at FATAL level."""
        self._log(DO_LOG_FATAL, message)

    def audit(self, message: str) -> None:
        """Log at AUDIT level (non-repudiable, WORM, signed)."""
        self._log(DO_LOG_AUDIT, message)

    def _log(self, level: int, message: str) -> None:
        """Submit a log record through the C ABI."""
        msg_bytes = message.encode("utf-8")
        params = DologgerLogParams(
            level=level,
            message=ctypes.c_char_p(msg_bytes),
            source_file=None,
            source_line=0,
        )
        rc = self._lib.dologger_log(self._handle, byref(params))
        if rc != DO_LOG_OK:
            err = DologgerError()
            self._lib.dologger_get_last_error(self._handle, byref(err))
            msg = err.message.decode("utf-8", errors="replace")
            # Logging failure is not fatal for the caller — just note it
            import sys
            print(f"[dologger] log dropped (code={rc}): {msg}", file=sys.stderr)

    # -- Lifecycle ----------------------------------------------------------

    def shutdown(self) -> None:
        """Gracefully shut down the engine and free resources."""
        if self._handle:
            self._lib.dologger_shutdown(self._handle)
            self._handle = None

    # -- Properties ---------------------------------------------------------

    @property
    def version(self) -> str:
        """Return the DoLogger core library version string."""
        return self._version

    def __del__(self):
        """Best-effort shutdown on garbage collection."""
        if hasattr(self, "_handle") and self._handle:
            try:
                self._lib.dologger_shutdown(self._handle)
            except Exception:
                pass  # interpreter may already be tearing down

    # Prevent accidental copying
    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.shutdown()
