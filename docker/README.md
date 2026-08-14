# DoLogger — Docker images

Containerized build and runtime environments for DoLogger.

| File | Purpose | Status |
|:-:|:-:|:-:|
| [Dockerfile.dev](Dockerfile.dev) | Full build environment (Rust + CMake ≥ 3.20 + Conan 2 + OpenSSL) | ✅ available |
| `Dockerfile.runtime` | Minimal runtime image (libdologger_core + runtime deps) | 🔜 delivered in the v1.0.0 release-hardening milestone |

## Development image

```bash
docker build -f docker/Dockerfile.dev -t dologger-dev .
docker run --rm -it -v "$PWD":/src -w /src dologger-dev cargo build --workspace
```

The runtime image (`Dockerfile.runtime`) is the minimal image described in the
design doc §18.4 — it ships only `libdologger_core` and its runtime
dependencies, for embedding into host apps or running the `dologctl run
--sidecar` deployment mode. It is intentionally **not** part of this
structure milestone; it lands with the release-hardening work.
