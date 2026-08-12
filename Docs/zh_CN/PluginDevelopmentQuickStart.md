# DoLogger 插件开发快速入门

> 🌐 **语言 / Language**: [中文](PluginDevelopmentQuickStart.md) | [English: DoLogger Plugin Development QuickStart](../en_US/PluginDevelopmentQuickStart.md)

> **版本**: v0.1.0 | **目标受众**: 非 Rust 插件开发者（C、C++、Go）
>
> **用途**: 从零开始用您选择的语言创建一个可工作的 DoLogger 插件。涵盖完整的构建链——Conan → CMake → Rust 核心 → 您的插件。

---

## 目录

1. [构建链如何工作](#构建链如何工作)
2. [前提条件](#前提条件)
3. [选择您的语言](#选择您的语言)
4. [C 插件演练](#c-插件演练)
5. [C++ 插件演练](#c-插件演练)
6. [Go 插件演练](#go-插件演练)
7. [跨平台编译](#跨平台编译)
8. [链接 Rust 核心](#链接-rust-核心)
9. [Conan Profile 参考](#conan-profile-参考)
10. [故障排查](#故障排查)

---

## 构建链如何工作

理解完整的编译链对于调试构建问题至关重要：

```mermaid
flowchart LR
    W["开发者工作流<br/>1. bash scripts/setup-conan.sh（安装 C 库）<br/>2. bash scripts/build-all.sh（构建一切）<br/>在底层，build-all.sh 按顺序运行："] --> A
    A["setup-conan.sh（Conan 2.x）<br/>安装 C 库：librdkafka、sqlite3、libsodium<br/>→ 生成 conan_toolchain.cmake"] -->|"conan_toolchain.cmake"| B["cargo build（Rust 编译）<br/>core/ → libdologger_core.{so,dll}<br/>cli/ → dologctl"]
    B -->|"libdologger_core + dologger_core.h"| C["cmake --build（使用 Conan 工具链）<br/>C/C++ 插件编译：<br/>plugins/examples/filter/c/<br/>plugins/examples/formatter/cpp/"]
    C --> D["go build -buildmode=c-shared<br/>Go 插件编译：<br/>plugins/examples/filter/go/<br/>→ dologger-plugin-*.{so,dll}"]
    D --> E["最终输出：build/plugins/*.so<br/>（或 .dll / .dylib）"]
```

### Conan 实际做什么？

Conan 是一个 **C/C++ 包管理器**（类似于 Node.js 的 npm 或 Python 的 pip）。在 DoLogger 中：

| 没有 Conan | 使用 Conan |
|:-:|:-:|
| 您必须通过系统包管理器（`apt`、`brew`、vcpkg）手动安装 `librdkafka`、`sqlite3`、`libsodium` | Conan 从锁定的配方自动下载和构建它们 |
| 每个开发者有不同的库版本 → "在我机器上没问题" 的 bug | 所有开发者获得 `conanfile.py` 中声明的相同版本 |
| 交叉编译需要手动 sysroot 设置 | Conan profiles 处理交叉编译（`--profile:host=...`） |
| CMake `find_package()` 可能找不到库 | `conan_toolchain.cmake` 确保 `find_package()` 始终解析 |

**Conan 不用于 Rust 核心。** Rust 核心（`dologger-core`）是纯 Rust 并由 Cargo 编译。Conan 仅管理非 Rust 插件可能链接的 C 库。

---

## 前提条件

| 工具 | 版本 | 检查 | 安装 |
|:-:|:-:|:-:|:-:|
| Rust | >= 1.70 | `rustc --version` | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| CMake | >= 3.20 | `cmake --version` | `apt install cmake` / `brew install cmake` |
| Conan | >= 2.0 | `conan --version` | `pipx install conan`（推荐） |
| Go | >= 1.21 | `go version` | 仅 Go 插件需要 |

一条命令完成设置：

```bash
# Linux / macOS
bash scripts/dologger-setup-dev

# Windows（PowerShell）
pwsh scripts/dologger-env-check.ps1
```

---

## 选择您的语言

| 语言 | 插件类型 | 构建系统 | 通过 Conan 的 C 依赖 | 编译输出 |
|:-:|:-:|:-:|:-:|:-:|
| **C** | Filter、Formatter、IOSink、Processor | CMake | 是 | `.so` / `.dll` / `.dylib` |
| **C++** | Filter、Formatter、IOSink、Processor | CMake | 是 | `.so` / `.dll` / `.dylib` |
| **Go** | Filter、Formatter、Processor | `go build -buildmode=c-shared` | 否（纯 Go） | `.so` / `.dll` / `.dylib` |
| **Rust** | 全部 10 种 VTable 类型 | Cargo | 通过 `-sys` crates | `.so` / `.dll` / `.dylib` |

**快速决策指南：**
- 您需要最大可移植性 → **C**（C11，无扩展）
- 您需要 C++ 生态（Protobuf、gRPC、Kafka 客户端） → **C++**（C++17）
- 您想要快速迭代、内存安全 → **Go**（cgo，通过 `import "C"` 的 C ABI）
- 您正在扩展引擎本身 → **Rust**（原生 Cargo workspace 成员）

---

## C 插件演练

我们将创建一个 Filter 插件，丢弃低于最低严重级别的消息。

### 步骤 1：目录结构

```text
plugins/examples/filter/c/my_filter/
├── CMakeLists.txt
├── my_filter.c
└── PluginManifest.toml
```

### 步骤 2：编写插件

**my_filter.c** — 实现 DoLogger Filter VTable：

```c
#include "dologger_core.h"   // 来自 core/include/ 的 C ABI 头文件

// 插件状态
static uint32_t g_min_level = DO_LOG_LEVEL_WARN;

// VTable filter 函数
// 返回 0 保留记录，非零丢弃记录。
static int my_filter_fn(const void *record, void *config) {
    uint32_t level = dologger_record_level(record);
    return (level < g_min_level) ? 1 : 0;
}

// -- C ABI 导出 ------------------------------------------

// 加载时调用一次。返回插件身份 + VTable。
DO_LOG_EXPORT const dologger_plugin_info_t *plugin_query(void) {
    static dologger_filter_vtable_t vtable = { .filter = my_filter_fn };
    static dologger_plugin_info_t info = {
        .name        = "my-filter",
        .version     = 0x000100,
        .abi_version = DO_LOG_CORE_ABI_VERSION,
        .phase       = DO_LOG_PHASE_FILTER,
        .trust_level = DO_LOG_TRUST_RED,   // 开发模式，无签名
        .vtable      = &vtable,
    };
    return &info;
}

// 在 plugin_query 之后、首次 filter 调用之前调用。
DO_LOG_EXPORT int plugin_init(const char *config_json) {
    // config_json: {"min_level": 3}（3 = WARN）
    if (config_json) {
        // 解析 JSON 并设置 g_min_level
    }
    return DO_LOG_OK;
}

// 库卸载之前调用。
DO_LOG_EXPORT int plugin_shutdown(void) {
    return DO_LOG_OK;
}
```

### 步骤 3：编写 CMakeLists.txt

```cmake
cmake_minimum_required(VERSION 3.16)
project(my-filter LANGUAGES C)

add_library(my_filter SHARED my_filter.c)

target_include_directories(my_filter PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../../../../../core/include
)

# 平台特定输出
if(WIN32)
    set_target_properties(my_filter PROPERTIES PREFIX "" SUFFIX ".dll")
elseif(APPLE)
    set_target_properties(my_filter PROPERTIES SUFFIX ".dylib")
else()
    set_target_properties(my_filter PROPERTIES SUFFIX ".so")
endif()

set_target_properties(my_filter PROPERTIES
    C_STANDARD 11
    C_STANDARD_REQUIRED ON
    C_EXTENSIONS OFF
)
```

### 步骤 4：构建

```bash
# 从项目根目录：
bash scripts/build-plugins.sh --filter c
```

输出：`build/plugins/my_filter/my_filter.so`（或 Windows 上 `.dll`）

---

## C++ 插件演练

结构同 C 相同，但导出符号需使用 `extern "C"`：

```cpp
#include "dologger_core.h"
#include <string>
#include <regex>

extern "C" {

static int regex_filter_fn(const void *record, void *config) {
    const char *msg = dologger_record_message(record);
    // ... C++ std::regex 匹配 ...
    return 0;
}

DO_LOG_EXPORT const dologger_plugin_info_t *plugin_query(void) {
    static dologger_filter_vtable_t vtable = { .filter = regex_filter_fn };
    static dologger_plugin_info_t info = {
        .name = "regex-filter", /* ... */
        .vtable = &vtable,
    };
    return &info;
}

DO_LOG_EXPORT int plugin_init(const char *config_json) { return DO_LOG_OK; }
DO_LOG_EXPORT int plugin_shutdown(void) { return DO_LOG_OK; }

} // extern "C"
```

CMakeLists.txt 使用 `CXX_STANDARD 17` 而非 `C_STANDARD 11`。

---

## Go 插件演练

Go 插件使用 `cgo`（`import "C"`）导出 C ABI 符号。不需要 CMake。

### 步骤 1：编写插件

```go
package main

/*
#include "dologger_core.h"

typedef struct { int (*filter)(const void*, void*); } my_filter_vtable_t;
extern int goFilter(const void*, void*);

static my_filter_vtable_t vtable = { .filter = goFilter };
*/
import "C"
import "unsafe"

//export plugin_query
func plugin_query() *C.dologger_plugin_info_t {
    info := (*C.dologger_plugin_info_t)(C.malloc(C.size_t(unsafe.Sizeof(C.dologger_plugin_info_t{}))))
    info.name = C.CString("go-filter")
    info.version = 0x000100
    info.abi_version = C.DO_LOG_CORE_ABI_VERSION
    info.phase = C.DO_LOG_PHASE_FILTER
    info.vtable = unsafe.Pointer(&C.vtable)
    return info
}

//export plugin_init
func plugin_init(config *C.char) C.int {
    return C.int(0)
}

//export plugin_shutdown
func plugin_shutdown() C.int {
    return C.int(0)
}

//export goFilter
func goFilter(rec unsafe.Pointer, cfg unsafe.Pointer) C.int {
    level := C.dologger_record_level(rec)
    if level < C.DO_LOG_LEVEL_WARN {
        return 1 // 丢弃
    }
    return 0 // 通过
}

func main() {}
```

### 步骤 2：构建

```bash
cd plugins/examples/filter/go/my_filter
CGO_ENABLED=1 go build -buildmode=c-shared -o dologger-plugin-my_filter.so .
```

或从项目根目录使用统一脚本：

```bash
bash scripts/build-plugins.sh --filter go
```

---

## 跨平台编译

### 问题

不同开发平台的差异：

| 开发者 | 平台 | 编译器 | 标准库 |
|:-:|:-:|:-:|:-:|
| 开发者 A | macOS ARM | clang | libc++ |
| 开发者 B | Linux x86 | gcc | libstdc++11 |
| 开发者 C | Windows | MSVC | dynamic CRT |

每个平台都有不同的：编译器标志、ABI 约定、库路径、动态链接器名称

### 解决方案：Conan Profiles

DoLogger 在 `.conan/profiles/` 中附带**5 个预配置的 Conan profile**：

| Profile | OS | 编译器 | 架构 |
|:-:|:-:|:-:|:-:|
| `linux-gcc-x86_64` | Linux | GCC 12 | x86_64 |
| `linux-clang-x86_64` | Linux | Clang 16 | x86_64 |
| `macos-clang-x86_64` | macOS | Apple Clang 15 | x86_64 |
| `macos-clang-arm64` | macOS | Apple Clang 15 | ARM64 |
| `windows-msvc-x86_64` | Windows | MSVC 194 | x86_64 |

### 使用 Profiles

```bash
# 自动检测您的平台并安装 C 依赖
bash scripts/setup-conan.sh

# 为交叉编译显式指定 profile（Linux → Windows）
bash scripts/setup-conan.sh --profile windows-msvc-x86_64

# 预览但不安装
bash scripts/setup-conan.sh --dry-run

# 仅打印将使用哪个 profile
bash scripts/setup-conan.sh --detect
```

Conan profile 确保 `librdkafka`、`sqlite3` 和 `libsodium` 针对与您的插件**完全相同**的目标编译——相同的编译器、相同的架构、相同的 ABI。

### 添加自定义 Profile

```bash
# 为 Raspberry Pi 交叉编译创建 profile
cp .conan/profiles/linux-gcc-x86_64 .conan/profiles/linux-gcc-arm64
# 编辑：更改 arch=armv8，添加工具链前缀
```

---

## 链接 Rust 核心

所有插件链接到**一个头文件**：

```
core/include/dologger_core.h
```

此头文件声明：

| 类别 | 符号 |
|:-:|:-:|
| **类型** | `dologger_plugin_info_t`、`dologger_filter_vtable_t`、`dologger_formatter_vtable_t`、... |
| **错误码** | `DO_LOG_OK`、`DO_LOG_ERR_INVALID_ARG`、`DO_LOG_ERR_NOT_SUPPORTED`、... |
| **阶段常量** | `DO_LOG_PHASE_FILTER`、`DO_LOG_PHASE_FORMATTING`、`DO_LOG_PHASE_SINK`、... |
| **信任级别** | `DO_LOG_TRUST_BLUE`、`DO_LOG_TRUST_YELLOW`、`DO_LOG_TRUST_RED` |
| **日志级别** | `DO_LOG_LEVEL_TRACE` 到 `DO_LOG_LEVEL_AUDIT` |
| **记录访问器** | `dologger_record_level()`、`dologger_record_message()`、... |
| **ABI 版本** | `DO_LOG_CORE_ABI_VERSION`（packed uint32） |

### 插件 ABI 契约

每个插件必须恰好导出这三个符号：

```c
// 1. 身份 + VTable —— 加载时调用一次
const dologger_plugin_info_t *plugin_query(void);

// 2. 配置 —— 在 plugin_query 之后、首次使用之前调用
int plugin_init(const char *config_json);

// 3. 清理 —— 库卸载之前调用
int plugin_shutdown(void);
```

引擎通过 `plugin_query()` 返回的 VTable 指针发现所有插件函数。运行时没有动态符号查找——只有三个入口点通过 `dlsym` / `GetProcAddress` 查找。

### 内存模型

```mermaid
flowchart TD
    A["宿主应用程序<br/>（C、C++、Python、Go、Rust——任何语言）"] -->|"dologger_log() ← C ABI"| B
    subgraph B["libdologger_core.{so,dll}（Rust cdylib——单一共享库）"]
        B1["管道（7 阶段）"]
        B2["缓冲区（无锁）"]
        B3["安全（Ed25519、AES-256）"]
    end
    B -->|"dlopen / LoadLibrary"| C["C 插件（.so）"]
    B -->|"dlopen / LoadLibrary"| D["C++ 插件（.so）"]
    B -->|"dlopen / LoadLibrary"| E["Go 插件（.so）"]
```

插件位于独立的共享库中。它们在构建时从不链接到核心——VTable 指针间接意味着零构建时耦合。在运行时，引擎通过 `dlopen` 加载插件并通过 VTable 调用。

---

## Conan Profile 参考

### Profile 文件格式

`.conan/profiles/` 中的每个 profile 遵循 Conan 的标准格式：

```ini
[settings]
os=Linux
arch=x86_64
compiler=gcc
compiler.version=12
compiler.libcxx=libstdc++11
build_type=Release

[options]
librdkafka/*:shared=False
sqlite3/*:shared=False
libsodium/*:shared=False

[conf]
tools.cmake.cmaketoolchain:generator=Ninja

[buildenv]
PKG_CONFIG_PATH=$PKG_CONFIG_PATH
```

### Profile 如何被选择

```mermaid
flowchart TD
    A["bash scripts/setup-conan.sh"] --> B{"提供 --profile <name>？"}
    B -->|"是"| P["使用该 profile"]
    B -->|"否"| C{"--detect？"}
    C -->|"是"| Q["打印检测到的 profile，退出"]
    C -->|"否"| R["自动检测：<br/>uname -s → Linux / macOS / Windows<br/>uname -m → x86_64 / arm64<br/>检测编译器 → gcc / clang / msvc"]
    P --> S["conan install . --profile:host=<profile> --profile:build=<profile>"]
    R --> S
```

---

## 故障排查

| 症状 | 原因 | 解决方案 |
|:-:|:-:|:-:|
| `cmake: include could not find load file: conan_toolchain.cmake` | 尚未运行 Conan | `bash scripts/setup-conan.sh` |
| `fatal error: dologger_core.h: No such file` | include 路径错误 | 检查 CMakeLists.txt 中的 `target_include_directories` |
| `undefined reference to dologger_record_level` | 尝试链接（不要！） | 插件绝不链接核心——仅使用 VTable |
| `go build: import "C" requires cgo` | 未设置 CGO_ENABLED | `export CGO_ENABLED=1` 或 `$env:CGO_ENABLED=1` |
| `plugin_query symbol not found`（运行时） | 缺少 `DO_LOG_EXPORT` 或 `//export` | 确保符号可见性（GCC：`-fvisibility=default`，Go：`//export`） |
| `conan: command not found` | Conan 未安装 | `pipx install conan`（隔离）或 `pip install conan` |
| `librdkafka/2.8.0: not found in remote` | 未配置 Conan center | `conan remote add conancenter https://center.conan.io` |

### 快速诊断

```bash
# 我的平台是什么？
bash scripts/dologger-env-check

# Conan 就绪了吗？
bash scripts/setup-conan.sh --detect
bash scripts/setup-conan.sh --dry-run

# 构建产出了什么？
find build/plugins -name "*.so" -o -name "*.dll" -o -name "*.dylib"
```

---

## 下一步

| 您想要... | 阅读 |
|:-:|:-:|
| 了解完整的 VTable API | [插件开发指南](guides/PluginDevelopmentGuide.md) |
| 查看可工作的示例 | `plugins/examples/` — C、C++、Go、Rust |
| 部署您的插件 | [运维与安全指南](OperationsAndSecurity.md) |
| 让您的插件获得签名（Blue 信任） | [安全白皮书](guides/SecurityWhitepaper.md) |
| 发布到插件索引 | `plugins/index/index.toml` |

---

*完整的架构规范请参见 [DoLogger 核心设计企划书](~/DoLogger/spec/DoLogger核心设计企划书.md)。*
