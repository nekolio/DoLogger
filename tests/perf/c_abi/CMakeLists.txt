cmake_minimum_required(VERSION 3.16)
project(dologger-c-abi-bench LANGUAGES C)

# C ABI log-throughput benchmark — drives libdologger_core through the
# public dologger_* API. The core library is loaded at runtime (dlopen /
# LoadLibrary), so there is no link step against the cdylib: point the bench
# at any release-artifact directory.
add_executable(c_abi_bench
    c_abi_bench.c
)

# Include path for dologger_core.h
target_include_directories(c_abi_bench PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../../../core/include
)

# C11 with no GNU extensions for portability
set_target_properties(c_abi_bench PROPERTIES
    C_STANDARD 11
    C_STANDARD_REQUIRED ON
    C_EXTENSIONS OFF
)

# Runtime loader: -ldl on Linux (glibc <2.34); macOS/Windows need none.
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    target_link_libraries(c_abi_bench PRIVATE dl)
endif()

# Install target (optional)
install(TARGETS c_abi_bench
    RUNTIME DESTINATION bin/dologger/bench
)
