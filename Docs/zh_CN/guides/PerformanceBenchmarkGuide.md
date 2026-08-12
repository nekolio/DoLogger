# DoLogger 性能基准测试指南

> 🌐 **语言 / Language**: [中文](PerformanceBenchmarkGuide.md) | [English: DoLogger Performance Benchmark Guide](../../en_US/guides/PerformanceBenchmarkGuide.md)

> **版本**: v0.1.0 | **最后更新**: 2026-08-12 | **目标受众**: 核心开发者、性能工程师、插件作者
>
> **用途**: 本文档描述如何运行、解释和扩展 DoLogger 基准测试套件。涵盖基准测试框架、参考硬件、结果解释（P50/P99/P99.9 百分位和吞吐量）、用于回归检测的 CI 集成以及添加新基准测试的约定。
>
> **阅读路径**: 新贡献者应阅读[前提条件](#前提条件)和[运行基准测试](#运行基准测试)。性能工程师应重点关注[结果解释](#结果解释)和[参考硬件](#参考硬件)。CI 维护者应阅读 [CI 集成](#ci-集成)。

## 目录

1. [前提条件](#前提条件)
2. [基准测试套件](#基准测试套件)
3. [运行基准测试](#运行基准测试)
4. [参考硬件](#参考硬件)
5. [结果解释](#结果解释)
6. [CI 集成](#ci-集成)
7. [添加新基准测试](#添加新基准测试)
8. [基准测试治理](#基准测试治理)

---

## 前提条件

### 环境准备

基准测试结果对系统状态高度敏感。运行任何基准测试前，请确保满足以下条件。

**表 1：基准测试环境要求**

| 要求 | 验证方式 | 不满足的影响 |
|:-:|:-:|:-:|
| CPU 频率锁定（无 turbo/speedstep） | Linux 上 `cpupower frequency-info` | 运行间差异可达 30% |
| 无其他 CPU 密集型进程 | `htop` / 任务管理器 — CPU 空闲 > 95% | 延迟基准测试中的噪声，P99 膨胀 |
| 无磁盘 I/O 竞争 | `iostat -x 1` — 磁盘利用率 < 5% | 文件/WORM 接收器基准测试的吞吐量波动 |
| 无网络活动（本地基准测试） | `iftop` / `nethogs` — 流量接近零 | 中断处理导致的延迟测量噪声 |
| 充足 RAM（无交换） | `free -h` — 已用 Swap = 0B | 缺页导致的灾难性延迟峰值 |
| 热节流已禁用 | `sensors` — CPU 温度稳定，无节流标志 | 随时间的渐进式吞吐量下降 |

### Linux 特定设置

```bash
# 1. 禁用 CPU 频率缩放
sudo cpupower frequency-set -g performance

# 2. 禁用 turbo boost（Intel）
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# 3. 禁用 turbo boost（AMD）
echo 0 | sudo tee /sys/devices/system/cpu/cpufreq/boost

# 4. 禁用透明大页（延迟敏感的基准测试）
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/defrag

# 5. 设置 CPU 亲和性以隔离基准测试核心
sudo cset shield --cpu 2-15 --kthread=on

# 6. 增加环形缓冲区的最大锁定内存
sudo sysctl -w vm.max_map_count=262144
```

### macOS 特定设置

```bash
# 禁用电源管理（部分设置需要禁用 SIP）
sudo pmset -a disablesleep 1

# 在基准测试目录上禁用 Spotlight 索引
sudo mdutil -i off /path/to/benchmark/output/
```

### Windows 特定设置

```powershell
# 将电源计划设置为高性能
powercfg /setactive 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c

# 在基准测试目录上禁用 Windows Defender 实时扫描
Add-MpPreference -ExclusionPath "C:\benchmarks\"
```

### 工具链要求

```bash
# 支持 LTO 的 Rust nightly 或 stable
rustup show active-toolchain
# 预期：stable-x86_64-unknown-linux-gnu（或 nightly）

# 验证 criterion 可用（基准测试框架）
cargo bench --help

# 安装 perf（Linux）以进行硬件计数器性能分析
sudo apt install linux-tools-common linux-tools-generic
```

---

## 基准测试套件

DoLogger 提供三个基准测试目标，每个测量性能的不同维度。

**表 2：基准测试目标概览**

| 基准测试 | Crate / 文件 | 测量内容 | 主要指标 |
|:-:|:-:|:-:|:-:|
| `latency` | `benches/latency.rs` | 单条记录提交延迟（`single_record_submit`、`single_record_submit_with_sign`） | P50/P99 ns |
| `throughput` | `benches/throughput.rs` | 环形缓冲区推送吞吐量（`ring_buffer_push_1k`、`ring_buffer_push_batch_256`） | records/s |
| `latency_percentiles` | `benches/latency_percentiles.rs` | 跨消息大小（80B/256B/1KB）、签名开/关、1/2/4/8/16 线程的完整延迟分布（每种 200K 样本） | P50/P99/P99.9/P99.99 |

### `latency` — 提交延迟（"热路径"）

测量从 `dologger_log()` 调用到返回的时间——这是宿主应用程序的**最关键指标**，因为它代表了每个日志语句额外增加的成本。实现为 `single_record_submit`（INFO）和 `single_record_submit_with_sign`（AUDIT，Ed25519）。

- **测量内容**：对象池分配 → 字段填充 → CAS 推入环形缓冲区（基准测试在批次之间排空缓冲区）
- **不包含内容**：管道处理、格式化、接收器 I/O（这些是异步的）
- **单位**：纳秒
- **统计处理**：Criterion.rs

### 规划中：逐阶段延迟细分

逐管道阶段的细分已规划但未在 v0.1.0 中实现（随附的 `latency` 基准测试测量的是完整提交延迟）：

| 阶段 | 计时内容 |
|:-:|:-:|
| PreFilter | PolicyProvider 评估 |
| Filter | Filter 插件决策 |
| FieldProvider | 字段注入 |
| Assembly | LSN 分配 + Ed25519 签名 + CRC32C |
| Processing | Processor 插件转换 |
| Formatting | Formatter 序列化 |
| Sink | IOSink 写入调用 |

### `throughput` — 最大持续速率

测量引擎在持续负载下每秒可推入环形缓冲区的记录数（批量大小 256 和 1000）：

- **测试的配置**：全部四种性能配置文件（`dev`、`balanced`、`prod-performance`、`prod-audit`）
- **接收器变体**：Console、File（无 fsync）、File（fsync）、WORM（sign + fsync）—— 规划中的扩展
- **记录大小**：64 B、256 B、1 KB、4 KB 消息
- **生产者线程数**：1、2、4、8、16

### `latency_percentiles` — 完整分布

生成完整的延迟直方图及百分位细分。用于检测尾部延迟回归和指示锁竞争或 GC 暂停的多模态分布。

---

## 运行基准测试

### 基本用法

```bash
# 运行单个基准测试套件（v0.1.0 仓库实际提供 latency、throughput、latency_percentiles）
cargo bench --bench latency

# 运行所有基准测试套件
cargo bench

# 运行套件中的特定基准测试
cargo bench --bench latency -- bench_single_record_latency

# 以特定性能配置文件运行
DO_LOG_PERF_PROFILE=prod-audit cargo bench --bench throughput
```

### 控制基准测试参数

```bash
# 覆盖样本大小（更快，统计效力较低）
cargo bench --bench latency -- --sample-size 20

# 设置固定测量时间
cargo bench --bench latency -- --measurement-time 30

# 按名称筛选基准测试子集
cargo bench --bench throughput -- "bench_ring_buffer_push"

# 保存基线以供后续比较
cargo bench --bench latency -- --save-baseline master

# 与保存的基线比较
cargo bench --bench latency -- --baseline master
```

### 带性能分析运行

```bash
# Linux perf — 硬件计数器
perf record --call-graph dwarf --freq=997 \
    cargo bench --bench latency -- bench_single_record_latency
perf report
perf stat -e cycles,instructions,cache-references,cache-misses,branches,branch-misses \
    cargo bench --bench latency -- bench_single_record_latency

# 火焰图
perf record --call-graph dwarf --freq=997 \
    cargo bench --bench latency -- bench_single_record_latency
perf script | stackcollapse-perf.pl | flamegraph.pl > latency_flamegraph.svg

# macOS Instruments
cargo instruments -t time --bench latency

# Valgrind / Callgrind（慢，高精度）
valgrind --tool=callgrind --dump-instr=yes \
    cargo bench --bench latency -- bench_single_record_latency
kcachegrind callgrind.out.*
```

### 基线比较工作流

```bash
# 步骤 1：在当前 main 分支上建立基线
git checkout main
cargo bench --bench latency -- --save-baseline main

# 步骤 2：在功能分支上运行基准测试
git checkout feature/my-change
cargo bench --bench latency -- --baseline main

# 步骤 3：Criterion 自动报告回归/改进：
#  "Performance has improved by 3.2%"
#  "Performance has regressed by 7.1%"
```

---

## 参考硬件

### 主要参考机器

DoLogger 文档中报告的所有官方基准测试数据均在此硬件配置上测量。

**表 3：主要参考硬件**

| 组件 | 规格 |
|:-:|:-:|
| **CPU** | AMD Ryzen 9 7950X（16 核 / 32 线程，4.5 GHz 基频，5.7 GHz 加速） |
| **RAM** | 64 GB DDR5-6000（2 x 32 GB，双通道） |
| **存储** | Samsung 990 Pro 2 TB NVMe（PCIe 4.0 x4） |
| **操作系统** | Ubuntu 24.04 LTS，Linux 内核 6.8 |
| **Rust** | 最新稳定版，`RUSTFLAGS="-C target-cpu=native -C lto=fat"` |
| **编译标志** | `--release`，LTO 启用，`codegen-units=1` |

### 次要参考机器

发布前还应在以下配置上验证基准测试：

**表 4：次要参考硬件**

| 平台 | CPU | RAM | 存储 | 操作系统 |
|:-:|:-:|:-:|:-:|:-:|
| Linux ARM | AWS Graviton3（64 核） | 128 GB DDR5 | EBS gp3 | Amazon Linux 2023 |
| macOS x86\_64 | Intel Core i9-13900K | 32 GB DDR5 | Apple SSD | macOS 14 Sonoma |
| macOS aarch64 | Apple M2 Max（12 核） | 32 GB LPDDR5 | Apple SSD | macOS 14 Sonoma |
| Windows x86\_64 | AMD Ryzen 9 7950X | 64 GB DDR5 | Samsung 990 Pro | Windows 11 24H2 |

### 报告自己的基准测试结果

分享基准测试结果时，请包含：

1. **硬件**：CPU 型号、RAM 速度/容量、存储型号、操作系统和内核版本
2. **软件**：Rust 版本、编译器标志（`RUSTFLAGS`）、LTO 设置
3. **环境**：频率调节器设置、turbo boost 状态、THP 状态
4. **DoLogger 配置**：性能配置文件、`ring_buffer_size`、`batch_size`、`enable_signature`

没有这些上下文，基准测试比较是不可靠的。

---

## 结果解释

### 百分位指标解释

**表 5：百分位解释**

| 百分位 | 它告诉您什么 | 若升高应采取的措施 |
|:-:|:-:|:-:|
| **P50**（中位数） | 典型情况性能。一半的操作比这更快完成。 | 高 P50 表明存在系统性问题——热路径算法、内存分配、锁竞争。 |
| **P90** | 中等程度的性能下降。10% 的操作比这更慢。 | 升高的 P90 表明存在间歇性停顿——缓存未命中、短暂持锁、分支预测错误。 |
| **P99** | 尾部延迟。1% 的操作比这更慢。 | 高 P99 表明罕见但有影响的停顿——操作系统调度抖动、TLB 未命中、NUMA 效应。 |
| **P99.9** | 极端尾部。0.1% 的操作比这更慢。 | 高 P99.9 指向极罕见事件——缺页、热节流、GC 暂停、内核中断。 |

### 吞吐量解释

读取吞吐量数据时应注意以下注意事项：

- **持续 vs 突发**：基准测试测量 30 秒以上的持续吞吐量。突发吞吐量（前 1-3 秒）可能高出 2-3 倍。
- **记录大小很重要**：更大的记录线性降低吞吐量（每条记录需要拷贝更多内存）。
- **接收器类型主导**：在扇出模式下，最慢的启用接收器决定端到端吞吐量。
- **签名成本**：Ed25519 签名将吞吐量限制在约 58,000 条记录/秒，无论接收器如何。

**表 6：预期性能范围（参考硬件）**

| 场景 | 吞吐量范围 | P50 延迟 | P99 延迟 |
|:-:|:-:|:-:|:-:|
| Console 接收器，签名关闭 | 1.0M - 1.4M rec/s | 75 - 95 ns | 180 - 250 ns |
| File 接收器，无 fsync | 800K - 1.1M rec/s | 95 - 120 ns | 320 - 450 ns |
| File 接收器，Ed25519 签名 | 50K - 62K rec/s | 16 - 19 us | 20 - 25 us |
| WORM 接收器，签名 + fsync | 10K - 14K rec/s | 78 - 90 us | 125 - 160 us |
| 环形缓冲区 CAS 推送（原始） | N/A（微基准测试） | 95 - 110 ns | 180 - 250 ns |

### 识别异常

**表 7：常见基准测试异常**

| 症状 | 可能原因 | 调查方向 |
|:-:|:-:|:-:|
| 双峰延迟分布（两个峰值） | 快速/慢速路径交替——可能的锁竞争或 NUMA 远程访问 | 使用 `perf lock` 查找竞争；检查 NUMA 节点亲和性 |
| 随时间渐进式吞吐量下降 | 热节流或内存泄漏 | 基准测试期间监控 CPU 温度和进程 RSS |
| 高方差（运行间 >20%） | Turbo boost 未禁用，后台进程干扰 | 验证[环境准备](#环境准备)中的前提条件 |
| P99.9 峰值超过 P50 的 100 倍 | 热路径上内存分配导致的缺页 | 检查 VTable 函数中的堆分配；使用 `perf record -e page-faults` |
| N 线程时吞吐量断崖 | 环形缓冲区 CAS 竞争（单一游标） | 已知限制——分片环形缓冲区尚未实现；参见[架构参考](../ArchitectureReference.md#已知限制) |

### 统计严谨性

Criterion.rs（基准测试框架）使用：

- **预热**：测量开始前的自动预热阶段
- **异常值分类**：Tukey's fences（IQR 方法）识别和报告异常值
- **置信区间**：每个测量附 95% CI
- **回归检测**：t 检验比较基线与当前值，标志 p < 0.05 的变更

将在 95% 置信区间内的变更视为**噪声**。只有超出置信区间且达到配置噪声阈值（默认：2%）的变更才被报告为回归或改进。

---

## CI 集成

### 回归检测管道

CI 管道在每个 Pull Request 上运行基准测试，并与 `main` 分支基线比较。

**表 8：CI 基准测试任务**

| 任务 | 触发条件 | 运行时间 | 失败标准 |
|:-:|:-:|:-:|:-:|
| `bench-hot-path` | 每个 PR | ~5 分钟 | 任何指标回归 > 5% |
| `bench-latency` | 每个 PR | ~8 分钟 | 任何阶段回归 > 5% |
| `bench-throughput` | 仅 main 分支 | ~12 分钟 | 任何接收器回归 > 5% |
| `bench-percentiles` | main 分支 + 每周 | ~15 分钟 | P99.9 回归 > 10% |

### CI 配置（GitHub Actions）

（示例 CI 配置 — YAML 语法有效，但该 workflow 与 `scripts/ci/check_benchmark_regression.py` 在 v0.1.0 仓库中尚不存在）：

```yaml
# .github/workflows/benchmarks.yml
name: Benchmarks

on:
  pull_request:
    paths:
      - 'core/**'
      - 'plugins/**'
      - 'benches/**'

jobs:
  bench-hot-path:
    runs-on: [self-hosted, linux, x64, benchmark]
    steps:
      - uses: actions/checkout@v4
      - name: Setup benchmark environment
        run: |
          sudo cpupower frequency-set -g performance
          echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
      - name: Run hot_path benchmarks
        run: |
          cargo bench --bench hot_path -- --save-baseline pr
      - name: Compare against main baseline
        run: |
          git fetch origin main
          git checkout origin/main
          cargo bench --bench hot_path -- --save-baseline main
          git checkout -
          cargo bench --bench hot_path -- --baseline main --criterion-reports
      - name: Check for regressions
        run: |
          # 如果任何基准测试回归超过 5% 则失败
          python3 scripts/ci/check_benchmark_regression.py \
            --baseline main --threshold 5

  bench-latency:
    runs-on: [self-hosted, linux, x64, benchmark]
    steps:
      - uses: actions/checkout@v4
      - name: Run latency benchmarks
        run: cargo bench --bench latency
      - name: Check regressions
        run: python3 scripts/ci/check_benchmark_regression.py --threshold 5

  bench-throughput:
    runs-on: [self-hosted, linux, x64, benchmark]
    if: github.ref == 'refs/heads/main'
    steps:
      - uses: actions/checkout@v4
      - name: Run throughput benchmarks
        run: cargo bench --bench throughput
      - name: Store results as artifact
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: target/criterion/
```

### 回归响应

当 CI 检测到回归时：

1. **调查**：PR 作者检查基准测试报告以识别哪个指标回归。
2. **性能分析**：运行 `perf` / 火焰图以隔离原因。
3. **修复或接受**：要么修复回归，要么记录并接受：
   - 为**安全改进**而导致的性能回归，如果在 PR 描述中记录了权衡，则可接受。
   - 为**正确性修复**而导致的性能回归，以 `perf-regression-accepted` 标签接受。
   - 所有其他回归阻止合并。

### 自托管运行器要求

CI 基准测试需要专用的、隔离的硬件。自托管运行器必须：

- 是专用物理机器（非虚拟机，非共享）
- 无其他并发运行的 CI 任务
- 每次运行前通过[环境准备](#环境准备)检查清单
- 将 CPU 频率、温度和内存压力作为 CI 产出物与基准测试结果一起记录

---

## 添加新基准测试

### 文件结构

```text
（v0.1.0 实际布局 — 基准测试位于 core/benches/ 下）
benches/
  latency.rs               ← 单条记录提交延迟（P50/P99）
  throughput.rs            ← 环形缓冲区推送吞吐量
  latency_percentiles.rs   ← 完整分布（P50/P99/P99.9/P99.99）
  #（规划中）common/
  #   setup.rs             ← 共享框架：引擎初始化、配置、预热
  #   fixtures.rs          ← 预构建记录模板
  #   reporting.rs         ← 结果格式化
```

### 基准测试模板

（伪代码 — 模板示例：`dologger_bench_common`、`Engine::log()` 等在 v0.1.0 不存在，仓库实际基准测试用 `ring_buffer.try_push` + `engine.pool.alloc` 模式，见 `core/benches/latency.rs`）：

```rust
// benches/hot_path.rs — 示例结构

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use dologger_bench_common::{setup_engine, create_sample_record};

fn bench_single_record_submit(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_path");
    group.measurement_time(std::time::Duration::from_secs(10));
    group.sample_size(100);

    // 跨多个环形缓冲区大小测试
    for buf_size in [65536, 131072, 262144, 524288].iter() {
        let mut engine = setup_engine(*buf_size);
        let record = create_sample_record();

        group.bench_with_input(
            BenchmarkId::new("single_submit", buf_size),
            buf_size,
            |b, _| {
                b.iter(|| {
                    engine.log(black_box(&record))
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_single_record_submit);
criterion_main!(benches);
```

### 约定

1. **使用 `criterion`**：所有基准测试使用 `criterion` crate（而非 libtest `#[bench]`）。Criterion 提供统计、基线比较和 HTML 报告。
2. **对所有输入使用 `black_box`**：通过将输入包装在 `std::hint::black_box()` 或 `criterion::black_box()` 中，防止编译器优化掉基准测试代码。
3. **预热阶段**：让 criterion 处理。不要手动预热。
4. **每个基准测试一个关注点**：每个 `bench_with_input` 组测量一个操作。不要在同一测量中合并提交延迟和格式化时间。
5. **清晰命名基准测试**：使用描述性的 `BenchmarkId` 名称——`"single_submit"`、`"batch_submit_256"`、`"file_sink_no_fsync"`。
6. **记录硬件上下文**：每个基准测试文件包含一个 `// HARDWARE:` 注释，列出预期的参考配置。
7. **配置文件敏感的基准测试**：如果基准测试的行为依赖于 `performance_profile`，请在所有四种配置文件下运行，使用 `cfg` 或环境变量检测。

### 基准测试配置矩阵

新基准测试应在适用时测试以下维度：

**表 9：基准测试参数矩阵**

| 参数 | 取值 |
|:-:|:-:|
| 性能配置文件 | `dev`、`balanced`、`prod-performance`、`prod-audit` |
| 环形缓冲区大小 | 65536、131072、262144、524288 |
| 批量大小 | 32、128、256、512 |
| 记录大小 | 64 B、256 B、1 KB、4 KB |
| 生产者线程数 | 1、2、4、8、16 |
| Ed25519 签名 | 开启、关闭 |
| fsync | 开启、关闭 |

并非每个基准测试都需要每种组合。选择与所测量操作相关的维度。

### 示例：为新接收器添加吞吐量基准测试

（伪代码 — 模板示例：`setup_config`/`setup_engine_with_config`/`Engine::log()` 为占位符，仅示意结构）：

```rust
// benches/throughput.rs

fn bench_throughput_new_sink(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput/new_sink");
    group.measurement_time(std::time::Duration::from_secs(30));

    for profile in ["prod-performance", "prod-audit"].iter() {
        let mut config = setup_config(profile);
        config.enable_sink("new_sink");

        let engine = setup_engine_with_config(&config);

        group.bench_with_input(
            BenchmarkId::new("sustained", profile),
            profile,
            |b, _| {
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for i in 0..iters {
                        engine.log(black_box(&RECORD_TEMPLATE));
                    }
                    start.elapsed()
                });
            },
        );
    }
    group.finish();
}
```

---

## 基准测试治理

### 基线存储

每个发布的官方基线以 Criterion 数据形式存储在仓库中：

```text
（示意 — `benches/baselines/` 为规划中；目前仅本地存在
 target/criterion/ 数据且不提交）
target/criterion/         ← 仅本地（不提交）
benches/baselines/         ← 已提交的基线，用于 CI 比较
  v1.0.0/
    latency.json
    throughput.json
    latency_percentiles.json
```

### 可接受的性能偏差

**表 10：性能偏差阈值**

| 变更类型 | Hot Path P50 | Hot Path P99 | 吞吐量 | 行动 |
|:-:|:-:|:-:|:-:|:-:|
| 噪声范围内 | < 2% | < 3% | < 2% | 接受——无需行动 |
| 轻微改进 | 2-10% | 3-10% | 2-10% | 接受——在变更日志中注明 |
| 轻微回归 | 2-5% | 3-5% | 2-5% | 审查——如有意为之则记录 |
| 显著回归 | > 5% | > 5% | > 5% | **阻止合并**——调查并修复 |
| 严重回归 | > 10% | > 10% | > 10% | **阻止合并**——需要明确理由 |

### 报告基准测试结果

发布基准测试结果（发布说明、学术论文、营销材料）时：

1. 始终引用参考硬件配置。
2. 报告**持续**吞吐量，而非突发吞吐量。
3. 同时报告 P50、P99 和 P99.9——永远不要仅报告一个百分位。
4. 包含所使用的确切 DoLogger 配置。
5. 说明 Ed25519 签名是否启用（启用时它主导延迟）。
6. 包含产生结果的基准测试命令。
7. 披露结果是在专用基准测试机器上还是在共享系统上产生的。
