# ==============================================================================
# FindRust.cmake — Locate Rust toolchain (rustc, cargo)
# ==============================================================================
# Provides:
#   RUSTC_EXECUTABLE   — Path to rustc
#   CARGO_EXECUTABLE   — Path to cargo
#   RUSTC_VERSION      — rustc version string
#   RUST_FOUND         — TRUE if both tools are found
# ==============================================================================

find_program(RUSTC_EXECUTABLE rustc)
find_program(CARGO_EXECUTABLE cargo)

if(RUSTC_EXECUTABLE AND CARGO_EXECUTABLE)
    # Extract version
    execute_process(
        COMMAND ${RUSTC_EXECUTABLE} --version
        OUTPUT_VARIABLE RUSTC_VERSION
        OUTPUT_STRIP_TRAILING_WHITESPACE
    )
    set(RUST_FOUND TRUE)
    message(STATUS "Found Rust: ${RUSTC_VERSION}")
else()
    set(RUST_FOUND FALSE)
    if(NOT RUSTC_EXECUTABLE)
        message(WARNING "rustc not found — Rust targets will be skipped")
    endif()
    if(NOT CARGO_EXECUTABLE)
        message(WARNING "cargo not found — Rust targets will be skipped")
    endif()
endif()

# Handle find_package requirements
include(FindPackageHandleStandardArgs)
find_package_handle_standard_args(Rust
    REQUIRED_VARS RUSTC_EXECUTABLE CARGO_EXECUTABLE
    VERSION_VAR RUSTC_VERSION
)
