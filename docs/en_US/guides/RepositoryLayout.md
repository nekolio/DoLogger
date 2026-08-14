# Repository Layout

> The authoritative map of the repository root. It answers one question:
> **which root entries are the product, which are required to build it, and
> which are peripheral to both.**

## 1. The six zones

```
DoLogger/
│  ① Platform entry (pinned by GitHub conventions)
├── README.md  README.zh_CN.md     ← landing page (GitHub renders root README)
├── LICENSE-APACHE  LICENSE-MIT    ← license detection
├── NOTICE  SECURITY.md            ← third-party notices / security policy
├── .github/                       ← Actions, issue/PR templates, CODEOWNERS
│   .gitignore                     ← local-only (intentionally untracked)
│
│  ② Build entry (auto-detected by cargo/cmake/conan)
├── Cargo.toml  Cargo.lock         ← Rust workspace root
├── rustfmt.toml  deny.toml        ← cargo fmt / cargo-deny config
├── CMakeLists.txt                 ← `cmake -S .` convention
├── conanfile.py                   ← `conan install .` convention
├── .cargo/                        ← cargo config
│
│  ③ Product (the DoLogger three-layer architecture)
├── core/                          ← stable kernel (libdologger_core, stable C ABI)
├── cli/                           ← dologctl
├── plugins/                       ← plugin ecosystem (official / examples / community)
├── adapters/                      ← language SDKs (C, Rust, Python, Go)
├── compliance/                    ← GDPR / HIPAA / PCI-DSS templates
├── config/                        ← example configuration
├── examples/                      ← minimal host-app examples (C ABI consumers)
├── tests/                         ← test suites (common / release-smoke / security)
│
│  ④ Documentation (content)
├── docs/                          ← EN + zh docs, auto-synced to the wiki
│
│  ⑤ Build infrastructure (source — required at build time)
├── cmake/                         ← CMake helper modules
├── docker/                        ← container images (Dockerfile.dev; runtime in v1.0.0)
├── .conan/                        ← cross-compile profiles
├── scripts/                       ← build / CI / release / setup scripts
│
│  ⑥ Peripheral (non-product, non-build)
└── peripheral/
    ├── site/                      ← GitHub Pages marketing site (Vue 3)
    └── tools/                     ← maintainer-only utilities (hero-svg)
```

## 2. What is pinned and why

Zones ① and ② cannot move without silently breaking the platform:

| Entry | Pinned by |
|:-:|:-:|
| `README.md` | GitHub renders the root README as the landing page |
| `LICENSE-*` | GitHub license detection / license API |
| `.github/` | GitHub Actions only runs from the root `.github/` |
| `Cargo.toml` / `Cargo.lock` | Cargo workspace root convention |
| `rustfmt.toml` / `deny.toml` | `cargo fmt` / cargo-deny auto-detection |
| `CMakeLists.txt` | `cmake -S .` convention |
| `conanfile.py` | `conan install .` convention |
| `.cargo/` | Cargo auto-discovers `.cargo/config.toml` from the root |

These stay at the root and are **documented as the platform entry**, not as
part of the product. Moving any of them makes the landing page, license badge,
CI, or build tooling stop working — usually without an error.

## 3. The distinction that matters

- Zone ⑤ (**build infrastructure**) is **source**: `cmake/`, `docker/`,
  `.conan/`, `scripts/` are required to build and ship the product. They are
  not "extras" — they live at the root like the product directories.
- Zone ⑥ (**peripheral**) is the only truly non-product content: the
  marketing site and maintainer tools. Neither is shipped with the product,
  neither is required to build it. Both live under `peripheral/` so the root
  reads product-first.
- Zones ① and ② are platform overhead. They are unavoidable, and their role
  is documented so no one mistakes `LICENSE` for a source file.

## 4. What moved in the layout alignment

| Before | After | Why |
|:-:|:-:|:-:|
| `site/` (root) | `peripheral/site/` | non-product: marketing |
| `tools/` (root) | `peripheral/tools/` | non-product: maintainer utilities |
| `Docs/` | `docs/` | lowercase, aligns with the design doc §3.3 and fixes the case-sensitive `.gitignore` mismatch |

Deployment paths were updated in lockstep: `pages.yml` / `wiki-sync.yml`
`paths:` filters, `scripts/build-site.sh`, `scripts/sync-wiki.sh`,
`peripheral/tools/hero-svg/hero_gen.py`.

## 5. Rule of thumb for new entries

Ask: **is it shipped with the product, or required to build it?**

- Product / build-required → top-level product or zone ⑤ directory.
- Neither → `peripheral/`.
- Platform metadata (license, README, CI) → root, documented as platform
  entry in this file.
