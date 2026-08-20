#!/usr/bin/env python3
"""
Conan 2.x recipe for DoLogger C/C++ build dependencies.

Usage:
    conan install . --output-folder=build --build=missing
    cmake -B build -DCMAKE_TOOLCHAIN_FILE=build/conan_toolchain.cmake
    cmake --build build

This recipe declares the C libraries that optional DoLogger features
link against.  The Rust core crate uses these indirectly via `-sys`
crates (rdkafka-sys, libsqlite3-sys) or via `#[link]` FFI directives.

Generated toolchain files ensure `find_package` and `pkg-config`
resolve correctly regardless of the platform package manager.
"""

from conan import ConanFile
from conan.tools.cmake import cmake_layout


class DologgerConan(ConanFile):
    name = "dologger"
    version = "0.0.1"
    license = "Apache-2.0 OR MIT"
    author = "DoLogger Contributors <nekoliowork+DoLogger@gmail.com>"
    url = "https://github.com/Nekolio/DoLogger"
    description = "Cross-platform, high-security logging engine — C dependency package"
    topics = ("logging", "security", "audit", "observability")

    # --- Build-system integration ---
    settings = "os", "compiler", "build_type", "arch"
    generators = "CMakeToolchain", "CMakeDeps", "PkgConfigDeps"

    # --- Default options ---
    default_options = {
        # librdkafka: enable all common protocols
        "librdkafka/*:ssl": True,
        "librdkafka/*:sasl": True,
        "librdkafka/*:lz4": True,
        "librdkafka/*:zstd": True,
        # libsodium: minimal build (we only need Ed25519 benchmark reference)
        "libsodium/*:shared": False,
        # sqlite3: thread-safe, no TCL
        "sqlite3/*:threadsafe": 1,
        "sqlite3/*:enable_column_metadata": True,
    }

    def layout(self):
        cmake_layout(self)

    def requirements(self):
        # Kafka Sink (feature: sink-kafka)
        self.requires("librdkafka/2.8.0")
        # Ed25519 external benchmark reference
        self.requires("libsodium/1.0.20")
        # SQLite Sink (feature: sink-sqlite), WORM local index
        self.requires("sqlite3/3.48.0")

    def configure(self):
        # Optional dependencies — API consumers may skip them
        self.options["librdkafka/*"].shared = False
        self.options["sqlite3/*"].shared = False
