import { createI18n } from 'vue-i18n'

/* zh/en only — the READMEs are 1:1 EN/ZH, and the site's palette targets
 * only the hero.svg brand ramp (no extra themes). */

const zh = {
  'theme-label': '主题', 'theme-system': '跟随系统', 'lang-label': '语言',
  'cursor-cyber': 'Cyber 鼠标', 'cursor-native': '系统鼠标',
  'download': '下载', 'docs': '文档', 'star': 'Star',
  'panel-title': '全部平台 · 校验和 · 版本', 'panel-assets': '平台资产', 'panel-versions': '版本',
  'panel-select-version': '选择版本', 'panel-checksums': '校验和', 'panel-bench': '跑分数据',
  'view-all-releases': '查看全部发布 →',
  'tag-zero-copy': '零拷贝', 'tag-audit': 'Ed25519 审计', 'tag-plugin': '插件化', 'tag-sinks': '11 种输出',
  'project-overview': '项目概览',
  'card-perf': '性能', 'card-sec': '安全', 'card-sinks': '输出 Sinks', 'card-arch': '架构', 'card-rel': '发布', 'card-comm': '社区',
  'footer-license': 'Apache-2.0 OR MIT',
  'os-windows': 'Windows', 'os-macos': 'macOS', 'os-linux': 'Linux',
  'hint-1': '向下滚动探索',
  'nav-hero': '主屏', 'nav-demo': '演示', 'nav-overview': '概览',
  'perf-p50': '提交延迟 P50', 'perf-thru': '吞吐量', 'perf-signed': '签名提交',
  'perf-crit': 'Criterion（本机 release + LTO）', 'perf-link': '每次发布的实测数据 →',
  'sec-chain': 'Ed25519 审计链 — 每条审计记录装配时签名，按 LSN + prev_hash 链式关联',
  'sec-sandbox': '插件沙箱 — seccomp-bpf / AppContainer / Sandbox',
  'sec-trust': '插件信任分级 — Blue（官方）/ Yellow / Red',
  'sec-worm': 'WORM 存储 + 不可降级的安全策略',
  'sec-priority': '7 级优先级 + 域继承配置',
  'sinks-note': '插件化架构 · 沙箱隔离',
  'arch-hot': '无锁热路径 — CAS 环形缓冲 + Treiber 对象池，提交零堆分配',
  'arch-flow-title': '7 级管道', 'arch-link': '架构参考 →',
  'rel-empty': '暂无发布。', 'rel-prerelease': 'pre-release', 'rel-all': '全部发布 →',
  'comm-empty': '暂无贡献者数据。',
  'comm-stars': 'Stars', 'comm-forks': 'Forks', 'comm-license': '许可证',
  'comm-ci': 'CI', 'comm-commit': '次提交',
  'demo-speed': '速率', 'demo-ms': 'ms/行'
}

const en: Record<string, string> = {
  'theme-label': 'Theme', 'theme-system': 'Auto', 'lang-label': 'Language',
  'cursor-cyber': 'Cyber cursor', 'cursor-native': 'Native cursor',
  'download': 'Download', 'docs': 'Docs', 'star': 'Star',
  'panel-title': 'All platforms · checksums · versions', 'panel-assets': 'Platform assets', 'panel-versions': 'Versions',
  'panel-select-version': 'Select version', 'panel-checksums': 'Checksums', 'panel-bench': 'Benchmarks',
  'view-all-releases': 'All releases →',
  'tag-zero-copy': 'Zero-copy', 'tag-audit': 'Ed25519 Audit', 'tag-plugin': 'Pluginable', 'tag-sinks': '11 Sinks',
  'project-overview': 'Project Overview',
  'card-perf': 'Performance', 'card-sec': 'Security', 'card-sinks': 'Output Sinks', 'card-arch': 'Architecture', 'card-rel': 'Releases', 'card-comm': 'Community',
  'footer-license': 'Apache-2.0 OR MIT',
  'os-windows': 'Windows', 'os-macos': 'macOS', 'os-linux': 'Linux',
  'hint-1': 'Scroll down to explore',
  'nav-hero': 'Hero', 'nav-demo': 'Demo', 'nav-overview': 'Overview',
  'perf-p50': 'P50 submit latency', 'perf-thru': 'Throughput', 'perf-signed': 'Signed submit',
  'perf-crit': 'Criterion (local, release + LTO)', 'perf-link': 'Measured per release →',
  'sec-chain': 'Ed25519 audit chain — every audit record signed at assembly, chained by LSN + prev_hash',
  'sec-sandbox': 'Plugin sandbox — seccomp-bpf / AppContainer / Sandbox',
  'sec-trust': 'Plugin trust levels — Blue (official) / Yellow / Red',
  'sec-worm': 'WORM sink + non-downgradable security policy',
  'sec-priority': '7 priority levels + domain inheritance',
  'sinks-note': 'Plugin architecture · sandboxed',
  'arch-hot': 'Lock-free hot path — CAS ring buffer + Treiber object pool, zero heap allocation on submit',
  'arch-flow-title': '7-stage pipeline', 'arch-link': 'Architecture reference →',
  'rel-empty': 'No releases yet.', 'rel-prerelease': 'pre-release', 'rel-all': 'All releases →',
  'comm-empty': 'No contributor data.',
  'comm-stars': 'Stars', 'comm-forks': 'Forks', 'comm-license': 'License',
  'comm-ci': 'CI', 'comm-commit': 'commits',
  'demo-speed': 'rate', 'demo-ms': 'ms/line'
}

const messages = { zh, en }

export type AppLocale = keyof typeof messages

export const i18n = createI18n({
  legacy: false,
  locale: 'en' as AppLocale,
  fallbackLocale: 'en',
  messages
})
