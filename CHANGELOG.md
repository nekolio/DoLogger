# Changelog

> 🌐 **语言 / Language**: [English](CHANGELOG.md) | [中文：更新日志](CHANGELOG.zh_CN.md)

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-12

Initial public release of DoLogger.

### Added

- **Core engine** (`dologger-core`): 7-stage pipeline, lock-free MPSC ring buffer with cooperative helping, Treiber stack object pool (zero malloc on submission)
- **Audit chain**: Ed25519-signed records with LSN + `prev_hash` linkage and external anchoring
- **11 output sinks**: console, file, callback, Kafka, syslog, webhook, SQLite, WORM, security file, shared memory, OpenTelemetry
- **Plugin system**: 10 VTable plugin types with 3-color trust model (Blue/Yellow/Red)
- **Plugin sandboxing**: seccomp-bpf (Linux), AppContainer (Windows), Sandbox (macOS)
- **4 performance profiles**: Dev / ProdPerformance / ProdAudit / Balanced
- **Compliance templates**: GDPR, HIPAA, PCI-DSS
- **Language adapters**: Rust SDK, Python ctypes, Go cgo
- **C ABI** (`dologger_*`): ABI-version-gated plugin interface with stability guarantees
- **`dologctl` CLI**: init / run / plugin / config / verify-log / perf / record / shm and more
- **Conan 2.x integration** for C/C++ plugin dependencies (librdkafka, sqlite3, libsodium)
- **Cross-platform CI**: Linux (x86\_64 + aarch64), macOS (aarch64), Windows (x86\_64)
- **Bilingual documentation** (English / 中文) with Mermaid diagrams

### Security

- 15 implemented security tests: sandbox escape, chain tamper, secret detection, WORM enforcement
- Non-downgradable security configuration items (signature, WORM, fsync, TLS, Ring 2 signing)
- Emergency AES-256-GCM encrypted mmap spill buffer for ring overflow
- Circuit breaker for remote sink fault isolation
- 14 secret-detection rules (AWS, GCP, GitHub tokens, private keys)
