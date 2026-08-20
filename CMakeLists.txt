# ==============================================================================
# DoLogger — Top-level CMake build orchestrator
# ==============================================================================
# This CMakeLists.txt orchestrates the build of the entire DoLogger project:
#   1. Rust core engine (libdologger_core) via Cargo
#   2. Non-Rust plugins (C/CPP) via CMake
#   3. CLI tool (dologctl) via Cargo
#   4. Language adapters
#   5. Tests and benchmarks
#
# Usage:
#   cmake -B build
#   cmake --build build
# ==============================================================================

cmake_minimum_required(VERSION 3.20)

# Project definition
project(DoLogger
    VERSION 0.0.1
    DESCRIPTION "Cross-platform, high-security, plugin-based logging engine"
    LANGUAGES C CXX
)

# C/C++ standard
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ==========================================================================
# Conan / C dependency detection
# ==========================================================================
# Include the Conan toolchain helper which auto-detects conan_toolchain.cmake
# and makes C libraries (librdkafka, sqlite3, libsodium) available via
# find_package() when Conan has been set up.
# ==========================================================================

include(cmake/conan_toolchain.cmake)
dologger_setup_conan()

# ==========================================================================
# Rust / Cargo integration
# ==========================================================================

# Locate Cargo (prefer the one associated with rustup)
find_program(CARGO_EXECUTABLE cargo REQUIRED)
message(STATUS "Found Cargo: ${CARGO_EXECUTABLE}")

# Determine the target triple
if(NOT DEFINED DOLOGGER_TARGET_TRIPLE)
    if(WIN32)
        set(DOLOGGER_TARGET_TRIPLE "x86_64-pc-windows-msvc")
    elseif(APPLE)
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
message(STATUS "Target triple: ${DOLOGGER_TARGET_TRIPLE}")

# Cargo build flags
set(CARGO_BUILD_FLAGS "--target" "${DOLOGGER_TARGET_TRIPLE}")

if(CMAKE_BUILD_TYPE STREQUAL "Release")
    list(APPEND CARGO_BUILD_FLAGS "--release")
    set(CARGO_PROFILE_DIR "release")
else()
    set(CARGO_PROFILE_DIR "debug")
endif()

# Determine library prefix/suffix for the target platform
if(WIN32)
    set(DYNAMIC_LIB_PREFIX "")
    set(DYNAMIC_LIB_SUFFIX ".dll")
elseif(APPLE)
    set(DYNAMIC_LIB_PREFIX "lib")
    set(DYNAMIC_LIB_SUFFIX ".dylib")
else()
    set(DYNAMIC_LIB_PREFIX "lib")
    set(DYNAMIC_LIB_SUFFIX ".so")
endif()

# ==========================================================================
# Build targets
# ==========================================================================

# --- Core Engine (libdologger_core) ---

set(CORE_BUILD_DIR "${CMAKE_SOURCE_DIR}/core")
set(CARGO_TARGET_DIR "${CMAKE_SOURCE_DIR}/target")
set(CORE_LIB_NAME "${DYNAMIC_LIB_PREFIX}dologger_core${DYNAMIC_LIB_SUFFIX}")
set(CORE_LIB_PATH "${CARGO_TARGET_DIR}/${DOLOGGER_TARGET_TRIPLE}/${CARGO_PROFILE_DIR}/${CORE_LIB_NAME}")

add_custom_command(
    OUTPUT ${CORE_LIB_PATH}
    COMMAND ${CARGO_EXECUTABLE} build ${CARGO_BUILD_FLAGS} --manifest-path "${CORE_BUILD_DIR}/Cargo.toml"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
    COMMENT "Building libdologger_core (Rust → cdylib)..."
    USES_TERMINAL
)

add_custom_target(dologger_core ALL
    DEPENDS ${CORE_LIB_PATH}
)

# --- CLI Tool (dologctl) ---

set(CLI_BUILD_DIR "${CMAKE_SOURCE_DIR}/cli")
set(CLI_BIN_NAME "dologctl")
if(WIN32)
    set(CLI_BIN_NAME "dologctl.exe")
endif()
set(CLI_BIN_PATH "${CARGO_TARGET_DIR}/${DOLOGGER_TARGET_TRIPLE}/${CARGO_PROFILE_DIR}/${CLI_BIN_NAME}")

add_custom_command(
    OUTPUT ${CLI_BIN_PATH}
    COMMAND ${CARGO_EXECUTABLE} build ${CARGO_BUILD_FLAGS} --manifest-path "${CLI_BUILD_DIR}/Cargo.toml"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
    COMMENT "Building dologctl (Rust CLI)..."
    USES_TERMINAL
)

add_custom_target(dologctl ALL
    DEPENDS ${CLI_BIN_PATH}
)

# --- Combined build target ---

add_custom_target(dologger_all ALL
    DEPENDS dologger_core dologctl
)

# ==========================================================================
# Non-Rust Plugin Builds (C/C++)
# ==========================================================================
# Each plugin directory under plugins/ with a CMakeLists.txt is added as a
# subdirectory.  Plugins that require C dependencies (librdkafka, etc.) are
# only built when the Conan toolchain is available (DOLOGGER_CONAN_READY).
# ==========================================================================

# Gather all C/C++ plugin build targets into one meta-target
add_custom_target(dologger_plugins)

file(GLOB plugin_cmake_dirs
    LIST_DIRECTORIES true
    RELATIVE "${CMAKE_SOURCE_DIR}"
    "${CMAKE_SOURCE_DIR}/plugins/examples/*/*/"
    "${CMAKE_SOURCE_DIR}/plugins/official/*/"
)

foreach(plugin_rel_dir ${plugin_cmake_dirs})
    set(plugin_dir "${CMAKE_SOURCE_DIR}/${plugin_rel_dir}")
    if(EXISTS "${plugin_dir}/CMakeLists.txt")
        # Extract a unique target-name-safe identifier
        get_filename_component(plugin_name "${plugin_dir}" NAME)
        string(MAKE_C_IDENTIFIER "${plugin_name}" plugin_target_suffix)
        set(plugin_target "dologger_plugin_${plugin_target_suffix}")

        # Build each plugin in an isolated build directory
        set(plugin_binary_dir "${CMAKE_BINARY_DIR}/plugins/${plugin_name}")
        add_subdirectory("${plugin_dir}" "${plugin_binary_dir}")

        # Defer the dependency check — plugin CMakeLists.txt files that
        # need find_package(librdkafka) etc. can guard with:
        #   if(NOT DOLOGGER_CONAN_READY)
        #       message(WARNING "Skipping ${plugin_name} — Conan deps missing")
        #       return()
        #   endif()
        if(TARGET "${plugin_target}")
            add_dependencies(dologger_plugins "${plugin_target}")
            message(STATUS "  Plugin target: ${plugin_target}")
        else()
            message(STATUS "  Plugin skipped (no target): ${plugin_rel_dir}")
        endif()
    endif()
endforeach()

message(STATUS "Plugin build target: cmake --build . --target dologger_plugins")

# ==========================================================================
# Install rules (stub — expanded in M3+)
# ==========================================================================

include(GNUInstallDirs)

install(FILES ${CORE_LIB_PATH}
    TYPE LIB
    COMPONENT runtime
)

install(FILES ${CLI_BIN_PATH}
    TYPE BIN
    COMPONENT runtime
)

# ==========================================================================
# Tests (via CTest)
# ==========================================================================

enable_testing()

add_test(
    NAME cargo_test_core
    COMMAND ${CARGO_EXECUTABLE} test --manifest-path "${CORE_BUILD_DIR}/Cargo.toml"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
)

# ==========================================================================
# Status summary
# ==========================================================================

message(STATUS "")
message(STATUS "DoLogger build configuration complete:")
message(STATUS "  Core library:  ${CORE_LIB_PATH}")
message(STATUS "  CLI tool:      ${CLI_BIN_PATH}")
message(STATUS "  Cargo:         ${CARGO_EXECUTABLE}")
message(STATUS "  Target:        ${DOLOGGER_TARGET_TRIPLE}")
message(STATUS "  Build type:    ${CMAKE_BUILD_TYPE}")
message(STATUS "")
