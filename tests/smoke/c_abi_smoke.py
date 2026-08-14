#!/usr/bin/env python3
"""C ABI smoke test for libdologger_core.

Loads the DoLogger core shared library through ctypes and exercises the
public C ABI lifecycle: version query, init, log submission (INFO + AUDIT),
last-error query, alloc/free, runtime config load, and shutdown.

Usage:
    python3 c_abi_smoke.py <path-to-libdologger_core>

Exit code 0 = all checks passed; 1 = failure (details on stdout).
"""

import ctypes
import sys
import tempfile
import os

# ── Error struct (dologger_error_t) ────────────────────────────────
class DologgerError(ctypes.Structure):
    _fields_ = [
        ("code", ctypes.c_int32),
        ("message", ctypes.c_char * 256),
        ("source_file", ctypes.c_char * 128),
        ("source_line", ctypes.c_uint32),
        ("_reserved", ctypes.c_uint8 * 12),
    ]


# ── Record params (dologger_record_params_t) ───────────────────────
class RecordParams(ctypes.Structure):
    _fields_ = [
        ("level", ctypes.c_int32),  # dologger_level_t
        ("message", ctypes.c_char_p),
        ("source_file", ctypes.c_char_p),
        ("source_function", ctypes.c_char_p),
        ("source_line", ctypes.c_uint32),
        ("source_column", ctypes.c_uint32),
        ("domain", ctypes.c_char_p),
        ("user_id", ctypes.c_char_p),
        ("session_id", ctypes.c_char_p),
        ("request_id", ctypes.c_char_p),
        ("_reserved", ctypes.c_uint8 * 16),
    ]


# Log levels
DO_LOG_INFO = 2
DO_LOG_AUDIT = 6
DO_LOG_OK = 0

checks = 0


def ok(cond, label):
    global checks
    checks += 1
    if cond:
        print(f"  [PASS] {label}")
    else:
        print(f"  [FAIL] {label}")
        raise SystemExit(1)


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    lib_path = os.path.abspath(sys.argv[1])

    print(f"== C ABI smoke test: {lib_path}")
    lib = ctypes.CDLL(lib_path)

    # ── Signatures ─────────────────────────────────────────────────
    lib.dologger_version.restype = ctypes.c_char_p
    lib.dologger_init.argtypes = [ctypes.c_char_p, ctypes.POINTER(DologgerError)]
    lib.dologger_init.restype = ctypes.c_void_p
    lib.dologger_log.argtypes = [ctypes.c_void_p, ctypes.POINTER(RecordParams)]
    lib.dologger_log.restype = ctypes.c_int32
    lib.dologger_shutdown.argtypes = [ctypes.c_void_p]
    lib.dologger_shutdown.restype = None
    lib.dologger_get_last_error.argtypes = [ctypes.c_void_p, ctypes.POINTER(DologgerError)]
    lib.dologger_get_last_error.restype = ctypes.c_int32
    lib.dologger_alloc.argtypes = [ctypes.c_size_t]
    lib.dologger_alloc.restype = ctypes.c_void_p
    lib.dologger_free.argtypes = [ctypes.c_void_p]
    lib.dologger_free.restype = None
    lib.dologger_config_load_from_string.argtypes = [
        ctypes.c_void_p,
        ctypes.c_char_p,
        ctypes.POINTER(DologgerError),
    ]
    lib.dologger_config_load_from_string.restype = ctypes.c_int32

    # ── 1. Version query ────────────────────────────────────────────
    version = lib.dologger_version()
    ok(version is not None, "dologger_version() returned a string")
    print(f"       core version = {version.decode()}")

    # ── 2. Init with default config (NULL path) ─────────────────────
    err = DologgerError()
    handle = lib.dologger_init(None, ctypes.byref(err))
    ok(handle, f"dologger_init(NULL) succeeded (err.code={err.code})")

    # ── 3. Submit an INFO record ────────────────────────────────────
    params = RecordParams(
        level=DO_LOG_INFO,
        message=b"cabi-smoke-test info record",
        source_file=b"c_abi_smoke.py",
        source_function=b"main",
        source_line=123,
        source_column=4,
        domain=b"smoke",
        user_id=b"u-1",
        session_id=b"s-1",
        request_id=b"r-1",
    )
    rc = lib.dologger_log(handle, ctypes.byref(params))
    ok(rc == DO_LOG_OK, f"dologger_log(INFO) returned {rc}")

    # ── 4. Submit an AUDIT record ───────────────────────────────────
    audit = RecordParams(
        level=DO_LOG_AUDIT,
        message=b"cabi-smoke-test audit record",
        source_file=None,
        source_function=None,
        source_line=0,
        source_column=0,
        domain=None,
        user_id=None,
        session_id=None,
        request_id=None,
    )
    rc = lib.dologger_log(handle, ctypes.byref(audit))
    ok(rc == DO_LOG_OK, f"dologger_log(AUDIT) returned {rc}")

    # ── 5. Last-error query (must not crash on a clean handle) ──────
    err2 = DologgerError()
    rc = lib.dologger_get_last_error(handle, ctypes.byref(err2))
    ok(rc == DO_LOG_OK or rc < 0, f"dologger_get_last_error() returned {rc}")

    # ── 6. Allocator roundtrip ──────────────────────────────────────
    ptr = lib.dologger_alloc(64)
    ok(bool(ptr), "dologger_alloc(64) returned non-NULL")
    lib.dologger_free(ptr)
    print("  [PASS] dologger_free(ptr) did not crash")

    # ── 7. Runtime config load ──────────────────────────────────────
    toml = b"[sinks.console]\ntype = \"sink_console\"\nenabled = true\n"
    rc = lib.dologger_config_load_from_string(handle, toml, ctypes.byref(err))
    ok(rc == DO_LOG_OK, f"dologger_config_load_from_string() returned {rc}")

    # ── 8. Graceful shutdown ────────────────────────────────────────
    lib.dologger_shutdown(handle)
    print("  [PASS] dologger_shutdown() did not crash")

    print(f"== All {checks} checks passed")
    sys.exit(0)


if __name__ == "__main__":
    main()
