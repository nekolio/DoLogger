cmake_minimum_required(VERSION 3.16)
project(dologger-formatter-cpp-example LANGUAGES CXX)

# Shared library target — builds the DoLogger Formatter plugin.
add_library(example_formatter SHARED
    example_formatter.cpp
)

# Include path for dologger_core.h
target_include_directories(example_formatter PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../../../../../core/include
)

# Platform-specific output names
if(WIN32)
    set_target_properties(example_formatter PROPERTIES
        PREFIX ""           # no "lib" prefix on Windows
        SUFFIX ".dll"
    )
elseif(APPLE)
    set_target_properties(example_formatter PROPERTIES
        SUFFIX ".dylib"
    )
else()
    set_target_properties(example_formatter PROPERTIES
        SUFFIX ".so"
    )
endif()

# C++17 with no GNU extensions for portability
set_target_properties(example_formatter PROPERTIES
    CXX_STANDARD 17
    CXX_STANDARD_REQUIRED ON
    CXX_EXTENSIONS OFF
)

# Ensure exported C symbols are visible (GCC/Clang default is visible;
# this guards against builds that inject -fvisibility=hidden).
if(UNIX)
    target_compile_options(example_formatter PRIVATE -fvisibility=default)
endif()

# Install target (optional)
install(TARGETS example_formatter
    LIBRARY DESTINATION lib/dologger/plugins
)
