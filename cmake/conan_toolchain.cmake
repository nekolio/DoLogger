# ==============================================================================
# DoLogger — Conan Toolchain Detection Helper
# ==============================================================================
# Include this file in your CMakeLists.txt to automatically detect and use
# the Conan-generated toolchain file.
#
# Usage in CMakeLists.txt:
#   include(cmake/conan_toolchain.cmake)
#   dologger_setup_conan()
#
# After calling dologger_setup_conan(), the Conan-installed packages
# (librdkafka, sqlite3, libsodium) are available via find_package().
# ==============================================================================

# ---------------------------------------------------------------------------
# dologger_find_conan_toolchain
#   Searches for conan_toolchain.cmake in common locations.
#   Returns the path in DOLOGGER_CONAN_TOOLCHAIN, or empty string.
# ---------------------------------------------------------------------------
function(dologger_find_conan_toolchain)
    set(candidates
        "${CMAKE_BINARY_DIR}/conan_toolchain.cmake"
        "${CMAKE_SOURCE_DIR}/build/conan_toolchain.cmake"
        "${CMAKE_SOURCE_DIR}/build/Release/conan_toolchain.cmake"
        "${CMAKE_SOURCE_DIR}/build/Debug/conan_toolchain.cmake"
    )

    foreach(candidate ${candidates})
        if(EXISTS "${candidate}")
            set(DOLOGGER_CONAN_TOOLCHAIN "${candidate}" PARENT_SCOPE)
            return()
        endif()
    endforeach()

    set(DOLOGGER_CONAN_TOOLCHAIN "" PARENT_SCOPE)
endfunction()

# ---------------------------------------------------------------------------
# dologger_setup_conan
#   Detects the Conan toolchain and provides status output.
#   Sets DOLOGGER_CONAN_READY to TRUE if Conan dependencies are available.
# ---------------------------------------------------------------------------
function(dologger_setup_conan)
    dologger_find_conan_toolchain()

    if(DOLOGGER_CONAN_TOOLCHAIN)
        message(STATUS "Conan toolchain found: ${DOLOGGER_CONAN_TOOLCHAIN}")
        include("${DOLOGGER_CONAN_TOOLCHAIN}")

        # Now find_package will resolve Conan-installed packages
        find_package(librdkafka QUIET)
        find_package(SQLite3 QUIET)
        find_package(libsodium QUIET)

        set(DOLOGGER_CONAN_READY TRUE PARENT_SCOPE)
        message(STATUS "Conan packages ready — librdkafka, SQLite3, libsodium available")
    else()
        set(DOLOGGER_CONAN_READY FALSE PARENT_SCOPE)
        message(STATUS "Conan toolchain not found — C dependencies will use system packages")
        message(STATUS "Run 'bash scripts/setup-conan.sh' to install C dependencies via Conan")
    endif()
endfunction()

# ---------------------------------------------------------------------------
# dologger_require_conan
#   Like dologger_setup_conan, but FATAL_ERROR if the toolchain is missing.
#   Use this when building plugins that MUST have C dependencies.
# ---------------------------------------------------------------------------
function(dologger_require_conan)
    dologger_setup_conan()
    if(NOT DOLOGGER_CONAN_READY)
        message(FATAL_ERROR
            "Conan toolchain required but not found.\n"
            "Run: bash scripts/setup-conan.sh\n"
            "Then re-run cmake: cmake -B build -DCMAKE_TOOLCHAIN_FILE=build/conan_toolchain.cmake"
        )
    endif()
endfunction()
