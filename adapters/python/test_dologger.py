"""
Test script for the DoLogger Python adapter.

Usage:
    python test_dologger.py

Requires libdologger_core to be built and available.
Set DO_LOGGER_LIB_PATH if the library is not in the system search path.
"""

import os
import sys

# Add the current directory to the path so we can import dologger
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from dologger import DoLogger


def main():
    print("=== DoLogger Python Adapter Test ===")
    print()

    # Test 1: Basic lifecycle
    print("[Test 1] Init/Shutdown lifecycle...")
    try:
        log = DoLogger()
        print(f"  Version: {log.version}")
        print("  Init OK")
        log.shutdown()
        print("  Shutdown OK")
        print("  PASSED")
    except RuntimeError as e:
        print(f"  SKIPPED (library not available): {e}")
        return

    print()

    # Test 2: Log all levels
    print("[Test 2] Log all levels...")
    log = DoLogger()
    log.trace("trace message from Python")
    log.debug("debug message from Python")
    log.info("info message from Python")
    log.warn("warn message from Python")
    log.error("error message from Python")
    log.fatal("fatal message from Python")
    log.audit("audit message from Python")
    print("  All levels submitted OK")
    log.shutdown()
    print("  PASSED")

    print()

    # Test 3: Stress test
    print("[Test 3] Stress test (1000 messages)...")
    log = DoLogger()
    for i in range(1000):
        log.info(f"Python stress test message #{i}")
    log.shutdown()
    print("  PASSED")

    print()

    # Test 4: Context manager
    print("[Test 4] Context manager...")
    with DoLogger() as log:
        log.info("Inside context manager")
    print("  PASSED")

    print()
    print("=== All tests passed ===")


if __name__ == "__main__":
    main()
