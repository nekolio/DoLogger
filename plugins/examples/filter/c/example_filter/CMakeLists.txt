cmake_minimum_required(VERSION 3.16)
project(dologger-filter-c-example LANGUAGES C)

# Shared library target — builds the DoLogger Filter plugin.
add_library(example_filter SHARED
    filter.c
)

# Include path for dologger_core.h
target_include_directories(example_filter PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../../../../../core/include
)

# Platform-specific output names
if(WIN32)
    set_target_properties(example_filter PROPERTIES
        PREFIX ""           # no "lib" prefix
        SUFFIX ".dll"
    )
elseif(APPLE)
    set_target_properties(example_filter PROPERTIES
        SUFFIX ".dylib"
    )
else()
    set_target_properties(example_filter PROPERTIES
        SUFFIX ".so"
    )
endif()

# C11 with no GNU extensions for portability
set_target_properties(example_filter PROPERTIES
    C_STANDARD 11
    C_STANDARD_REQUIRED ON
    C_EXTENSIONS OFF
)

# Ensure exported symbols are visible (GCC/Clang default is visible;
# this guards against builds that inject -fvisibility=hidden).
if(UNIX)
    target_compile_options(example_filter PRIVATE -fvisibility=default)
endif()

# Install target (optional)
install(TARGETS example_filter
    LIBRARY DESTINATION lib/dologger/plugins
)
