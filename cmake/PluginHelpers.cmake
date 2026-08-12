# ==============================================================================
# PluginHelpers.cmake — Utility functions for building DoLogger plugins
# ==============================================================================
# Provides:
#   dologger_add_plugin(name SOURCES ... [MANIFEST plugin.toml])
#       Add a C/CPP plugin target with standard DoLogger conventions.
#   dologger_add_rust_plugin(name MANIFEST_DIR dir)
#       Build a Rust plugin via Cargo and register it.
# ==============================================================================

# Add a C/CPP plugin
function(dologger_add_plugin PLUGIN_NAME)
    set(options "")
    set(one_value_args MANIFEST)
    set(multi_value_args SOURCES LINK_LIBS)
    cmake_parse_arguments(PLUGIN "${options}" "${one_value_args}" "${multi_value_args}" ${ARGN})

    # Create shared library target
    add_library(${PLUGIN_NAME} SHARED ${PLUGIN_SOURCES})
    target_include_directories(${PLUGIN_NAME} PRIVATE
        "${CMAKE_SOURCE_DIR}/core/include"
    )

    if(PLUGIN_LINK_LIBS)
        target_link_libraries(${PLUGIN_NAME} PRIVATE ${PLUGIN_LINK_LIBS})
    endif()

    # Set output name
    set_target_properties(${PLUGIN_NAME} PROPERTIES
        PREFIX "lib"
        SUFFIX "${DYNAMIC_LIB_SUFFIX}"
        LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/plugins"
        RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/plugins"
    )

    # Copy manifest to output directory
    if(PLUGIN_MANIFEST)
        add_custom_command(TARGET ${PLUGIN_NAME} POST_BUILD
            COMMAND ${CMAKE_COMMAND} -E copy_if_different
                "${PLUGIN_MANIFEST}"
                "${CMAKE_BINARY_DIR}/plugins/"
        )
    endif()

    message(STATUS "Plugin: ${PLUGIN_NAME} (C/CPP)")
endfunction()

# Add a Rust plugin (built as cdylib via Cargo)
function(dologger_add_rust_plugin PLUGIN_NAME)
    set(one_value_args MANIFEST_DIR)
    cmake_parse_arguments(PLUGIN "" "${one_value_args}" "" ${ARGN})

    if(NOT PLUGIN_MANIFEST_DIR)
        set(PLUGIN_MANIFEST_DIR "${CMAKE_SOURCE_DIR}/plugins/${PLUGIN_NAME}")
    endif()

    add_custom_command(
        OUTPUT "${CARGO_TARGET_DIR}/${DOLOGGER_TARGET_TRIPLE}/${CARGO_PROFILE_DIR}/lib${PLUGIN_NAME}${DYNAMIC_LIB_SUFFIX}"
        COMMAND ${CARGO_EXECUTABLE} build ${CARGO_BUILD_FLAGS}
            --manifest-path "${PLUGIN_MANIFEST_DIR}/Cargo.toml"
        WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
        COMMENT "Building Rust plugin: ${PLUGIN_NAME}"
        USES_TERMINAL
    )

    message(STATUS "Plugin: ${PLUGIN_NAME} (Rust)")
endfunction()
