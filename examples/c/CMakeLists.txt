cmake_minimum_required(VERSION 3.20)
project(dologger_c_example C)

# The core library is produced by Cargo (`cargo build --release` in the repo
# root). This example links against that prebuilt cdylib; build it first.
set(DOLOGGER_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../..")
set(DOLOGGER_INCLUDE_DIR "${DOLOGGER_ROOT}/core/include")

# Locate the core shared library. Prefer the Cargo release output, then debug.
find_library(DOLOGGER_CORE_LIB
    NAMES dologger_core
    HINTS "${DOLOGGER_ROOT}/target/release" "${DOLOGGER_ROOT}/target/debug"
)
if(NOT DOLOGGER_CORE_LIB)
    message(FATAL_ERROR
        "libdologger_core not found. Run `cargo build --release` in the repo root first.")
endif()

add_executable(dologger-c-example main.c)
target_include_directories(dologger-c-example PRIVATE "${DOLOGGER_INCLUDE_DIR}")
target_link_libraries(dologger-c-example PRIVATE "${DOLOGGER_CORE_LIB}")
