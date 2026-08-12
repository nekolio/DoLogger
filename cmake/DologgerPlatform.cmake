# ==============================================================================
# DologgerPlatform.cmake — Platform detection and cross-compilation helpers
# ==============================================================================
# Provides:
#   DOLOGGER_PLATFORM          — Current platform (windows, linux, macos)
#   DOLOGGER_TARGET_TRIPLE     — Rust target triple
#   DOLOGGER_SHM_SUPPORTED     — Whether POSIX SHM is available
#   DOLOGGER_IOURING_SUPPORTED — Whether io_uring is available (linux >= 5.1)
#   DOLOGGER_IOCP_SUPPORTED    — Whether IOCP is available (windows)
#   DOLOGGER_KQUEUE_SUPPORTED  — Whether kqueue is available (macos)
# ==============================================================================

# Platform detection
if(WIN32)
    set(DOLOGGER_PLATFORM "windows")
    set(DOLOGGER_IOCP_SUPPORTED TRUE)
    set(DOLOGGER_IOURING_SUPPORTED FALSE)
    set(DOLOGGER_KQUEUE_SUPPORTED FALSE)
    set(DOLOGGER_SHM_SUPPORTED FALSE) # Windows uses named file mapping instead
elseif(APPLE)
    set(DOLOGGER_PLATFORM "macos")
    set(DOLOGGER_KQUEUE_SUPPORTED TRUE)
    set(DOLOGGER_IOURING_SUPPORTED FALSE)
    set(DOLOGGER_IOCP_SUPPORTED FALSE)
    set(DOLOGGER_SHM_SUPPORTED TRUE)
else()
    set(DOLOGGER_PLATFORM "linux")
    set(DOLOGGER_IOURING_SUPPORTED TRUE)
    set(DOLOGGER_IOCP_SUPPORTED FALSE)
    set(DOLOGGER_KQUEUE_SUPPORTED FALSE)
    set(DOLOGGER_SHM_SUPPORTED TRUE)
endif()

# Target triple detection
if(NOT DOLOGGER_TARGET_TRIPLE)
    if(DOLOGGER_PLATFORM STREQUAL "windows")
        set(DOLOGGER_TARGET_TRIPLE "x86_64-pc-windows-msvc")
    elseif(DOLOGGER_PLATFORM STREQUAL "macos")
        if(CMAKE_SYSTEM_PROCESSOR STREQUAL "arm64")
            set(DOLOGGER_TARGET_TRIPLE "aarch64-apple-darwin")
        else()
            set(DOLOGGER_TARGET_TRIPLE "x86_64-apple-darwin")
        endif()
    else()
        if(CMAKE_SYSTEM_PROCESSOR STREQUAL "aarch64")
            set(DOLOGGER_TARGET_TRIPLE "aarch64-unknown-linux-gnu")
        else()
            set(DOLOGGER_TARGET_TRIPLE "x86_64-unknown-linux-gnu")
        endif()
    endif()
endif()

message(STATUS "DoLogger platform: ${DOLOGGER_PLATFORM}")
message(STATUS "Target triple: ${DOLOGGER_TARGET_TRIPLE}")
message(STATUS "  io_uring: ${DOLOGGER_IOURING_SUPPORTED}")
message(STATUS "  IOCP:     ${DOLOGGER_IOCP_SUPPORTED}")
message(STATUS "  kqueue:   ${DOLOGGER_KQUEUE_SUPPORTED}")
message(STATUS "  SHM:      ${DOLOGGER_SHM_SUPPORTED}")
