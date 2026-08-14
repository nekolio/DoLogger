/* DoLogger site — dynamic data layer (Vue module, TypeScript).
 *
 * Fetches version / release / benchmark data from the GitHub API with a
 * localStorage cache, and falls back to build-time JSON baked by
 * peripheral/github/scripts/build-site.sh, then to the v0.1.0 manifest below. The
 * page picks up a new release with no code change.
 *
 * Resolution order per dataset:
 *   fresh cache → GitHub API → stale cache → baked JSON → hardcoded fallback
 *
 * Note: /releases/latest is NOT used — every v0.x tag is marked a
 * prerelease by release.yml, and /releases/latest 404s for prereleases.
 */
import { ref, type Ref } from 'vue'

const REPO = 'Nekolio/DoLogger'
const BASE = 'https://api.github.com'
const CACHE_TTL = 15 * 60 * 1000 // ms

/* ------------------------------------------------------------------
 * Types
 * ---------------------------------------------------------------- */
export interface ReleaseAsset {
  name: string
  browser_download_url: string
}
export interface Release {
  tag_name: string
  name: string
  prerelease: boolean
  html_url: string
  published_at: string
  assets: ReleaseAsset[]
}
export interface BenchmarkEnv {
  label: string
  p50?: string | null
  throughput?: string | null
  signed?: string | null
}
export interface CriterionRow {
  name: string
  value: string
  note?: string
}
export interface Benchmarks {
  fallback: boolean
  tag?: string | null
  environments: BenchmarkEnv[]
  criterion: CriterionRow[]
}
export interface Repo {
  stargazers_count: number | null
  forks_count: number | null
  html_url: string
}
export interface Contributor {
  login: string
  contributions: number | null
  html_url: string
  avatar_url?: string | null
}
export type Platform = { os: 'windows' | 'macos' | 'linux'; arch: string }
export interface SiteData {
  releases: Release[]
  latest: Release
  repo: Repo
  contributors: Contributor[]
  benchmarks: Benchmarks
  platform: Platform
  downloadUrl: string
}

/* ------------------------------------------------------------------
 * v0.1.0 fallback — the REAL release manifest (asset names match the
 * release.yml build matrix). Used offline / rate-limited / pre-release.
 * Includes the official-plugins bundle: ONE asset per OS/arch carrying
 * every official plugin (fmt-json, fmt-text, filter-level, field-container).
 * ---------------------------------------------------------------- */
export const ASSET_NAMES = [
  'dologctl-linux-x86_64', 'dologctl-linux-aarch64', 'dologctl-linux-i686',
  'dologctl-linux-armv7', 'dologctl-linux-riscv64',
  'dologctl-windows-x86_64.exe', 'dologctl-windows-aarch64.exe', 'dologctl-windows-i686.exe',
  'dologctl-macos-aarch64', 'dologctl-macos-x86_64',
  'libdologger_core-linux-x86_64.so', 'libdologger_core-linux-aarch64.so',
  'libdologger_core-linux-i686.so', 'libdologger_core-linux-armv7.so',
  'libdologger_core-linux-riscv64.so',
  'dologger_core-windows-x86_64.dll', 'dologger_core-windows-aarch64.dll', 'dologger_core-windows-i686.dll',
  'libdologger_core-macos-aarch64.dylib', 'libdologger_core-macos-x86_64.dylib',
  'dologger-official-plugins-linux-x86_64.so', 'dologger-official-plugins-linux-aarch64.so',
  'dologger-official-plugins-linux-i686.so', 'dologger-official-plugins-linux-armv7.so',
  'dologger-official-plugins-linux-riscv64.so',
  'dologger-official-plugins-windows-x86_64.dll', 'dologger-official-plugins-windows-aarch64.dll',
  'dologger-official-plugins-windows-i686.dll',
  'dologger-official-plugins-macos-aarch64.dylib', 'dologger-official-plugins-macos-x86_64.dylib',
  'benchmark-results.json'
]

export const FALLBACK_RELEASES: Release[] = [{
  tag_name: 'v0.1.0',
  name: 'DoLogger v0.1.0',
  prerelease: true,
  html_url: 'https://github.com/Nekolio/DoLogger/releases/tag/v0.1.0',
  published_at: '2026-08-13T00:00:00Z',
  assets: ASSET_NAMES.map(function (name) {
    return {
      name: name,
      browser_download_url: 'https://github.com/Nekolio/DoLogger/releases/download/v0.1.0/' + name
    }
  })
}]

/* README "Performance Snapshot" — measured on the same code (release + LTO). */
export const FALLBACK_BENCHMARKS: Benchmarks = {
  fallback: true,
  tag: 'v0.1.0',
  environments: [
    {
      label: 'GitHub runner — AMD EPYC 7763',
      p50: '120 ns', throughput: '5.06M rec/s', signed: '19.8 µs'
    },
    {
      label: 'Local — Windows 11 LTSC, Intel i5-12400F',
      p50: '102 ns', throughput: '9.78M rec/s', signed: '16.96 µs'
    }
  ],
  criterion: [
    { name: 'single_record_submit', value: '102 ns', note: '~9.78M rec/s' },
    { name: 'ring_buffer_push_1k', value: '121 µs', note: '~8.26M rec/s' },
    { name: 'ring_buffer_push_batch_256', value: '19.2 µs', note: '~13.3M rec/s' }
  ]
}

const FALLBACK_REPO: Repo = {
  stargazers_count: null, forks_count: null,
  html_url: 'https://github.com/Nekolio/DoLogger'
}

const FALLBACK_CONTRIBUTORS: Contributor[] = [
  { login: 'Nekolio', contributions: null, html_url: 'https://github.com/Nekolio' }
]

/* ------------------------------------------------------------------ */

/* A stalled API request must not stall the whole page: every external
 * fetch gets a hard timeout so the chain (API → cache → baked JSON →
 * hardcoded fallback) always terminates. */
const FETCH_TIMEOUT_MS = 8000
function fetchT(url: string, init?: RequestInit): Promise<Response> {
  return fetch(url, { ...init, signal: AbortSignal.timeout(FETCH_TIMEOUT_MS) })
}

async function apiGet<T>(path: string): Promise<T | null> {
  try {
    const r = await fetchT(BASE + path, { headers: { Accept: 'application/vnd.github+json' } })
    return r.ok ? (r.json() as Promise<T>) : null
  } catch { return null }
}

async function fetchJson<T>(url: string): Promise<T | null> {
  try {
    const r = await fetchT(url, { cache: 'no-cache' })
    return r.ok ? (r.json() as Promise<T>) : null
  } catch { return null }
}

function cacheRead<T>(key: string): { t: number; d: T } | null {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return null
    const parsed = JSON.parse(raw) as { t: number; d: T }
    return typeof parsed?.t === 'number' && 'd' in parsed ? parsed : null
  } catch { return null }
}
function cacheWrite(key: string, data: unknown): void {
  try { localStorage.setItem(key, JSON.stringify({ t: Date.now(), d: data })) } catch { /* private mode */ }
}
function isFresh(entry: { t: number } | null): boolean {
  return !!entry && (Date.now() - entry.t) < CACHE_TTL
}

async function withCache<T>(key: string, apiFn: () => Promise<T | null>, bakedUrl: string | null, fallback: T): Promise<T> {
  const cached = cacheRead<T>(key) // the {t, d} wrapper, written by cacheWrite below
  if (cached && isFresh(cached)) return cached.d
  const fresh = await apiFn()
  // empty arrays cache badly — a fresh publish would stay invisible
  if (fresh && (!Array.isArray(fresh) || fresh.length > 0)) { cacheWrite(key, fresh); return fresh }
  if (cached) return cached.d // stale but usable
  const baked = bakedUrl ? await fetchJson<T>(bakedUrl) : null
  if (baked && (Array.isArray(baked) ? baked.length > 0 : true)) return baked
  return fallback
}

/* ------------------------------------------------------------------ */

function getReleases(): Promise<Release[]> {
  /* per_page=100 — the filter popup's version selector lists every
     release, not just the newest handful. */
  return withCache<Release[]>('dologger:releases',
    () => apiGet<Release[]>('/repos/' + REPO + '/releases?per_page=100'),
    './data/releases.json', FALLBACK_RELEASES)
}
function getRepo(): Promise<Repo> {
  return withCache<Repo>('dologger:repo',
    () => apiGet<Repo>('/repos/' + REPO),
    null, FALLBACK_REPO)
}
function getContributors(): Promise<Contributor[]> {
  return withCache<Contributor[]>('dologger:contributors',
    () => apiGet<Contributor[]>('/repos/' + REPO + '/contributors?per_page=12'),
    './data/contributors.json', FALLBACK_CONTRIBUTORS)
}

interface BakedPerf { latency_ns?: { p50?: number; signed?: number }; throughput_rec_per_sec?: number }
interface BakedBenchmarks {
  fallback?: boolean
  tag?: string
  cpu?: string
  perf?: BakedPerf
  criterion?: Record<string, { value: number; unit: string }>
}

/* benchmark-results.json is baked into the artifact at build time by
 * peripheral/github/scripts/build-site.sh (server-side — the browser cannot fetch
 * release assets reliably). Normalize the release.yml schema into the same
 * shape as the fallback. */
async function getBenchmarks(): Promise<Benchmarks> {
  const baked = await fetchJson<BakedBenchmarks>('./data/benchmarks.json')
  if (baked && !baked.fallback) return normalizeBakedBenchmarks(baked)
  return FALLBACK_BENCHMARKS
}

function normalizeBakedBenchmarks(b: BakedBenchmarks): Benchmarks {
  const env: BenchmarkEnv[] = b.perf && b.perf.latency_ns
    ? [{
        label: 'GitHub runner — ' + (b.cpu || '') + ', ' + (b.tag || ''),
        p50: fmtLatency(b.perf.latency_ns.p50),
        throughput: fmtRate(b.perf.throughput_rec_per_sec),
        signed: b.perf.latency_ns.signed != null ? fmtLatency(b.perf.latency_ns.signed) : null
      }]
    : []
  const crit: CriterionRow[] = []
  if (b.criterion) {
    for (const name in b.criterion) {
      const c = b.criterion[name]
      crit.push({ name: name, value: fmtLatency(c.value, c.unit) ?? '' })
    }
  }
  return { fallback: false, tag: b.tag || null, environments: env, criterion: crit }
}

function fmtLatency(v: number | undefined, unit?: string): string | null {
  if (typeof v !== 'number') return null
  const us = unit === 'µs' || unit === 'us' ? v : v / 1000
  if (us >= 1000) return (us / 1000).toFixed(2) + ' ms'
  if (us >= 1) return (Math.round(us * 100) / 100) + ' µs'
  return Math.round(v) + ' ns'
}
function fmtRate(v: number | undefined): string | null {
  if (typeof v !== 'number') return null
  if (v >= 1e6) return (v / 1e6).toFixed(2) + 'M rec/s'
  if (v >= 1e3) return Math.round(v / 1e3) + 'K rec/s'
  return Math.round(v) + ' rec/s'
}

/* ------------------------------------------------------------------
 * Platform detection. arch comes from (in order): Client Hints
 * (Chrome/Edge), UA (Firefox), WebGL renderer (Safari on Apple Silicon
 * still claims MacIntel — a maxTouchPoints check detects iPads, not
 * Apple Silicon, so it is not used).
 * ---------------------------------------------------------------- */
interface NavUAData { getHighEntropyValues(hints: string[]): Promise<{ architecture?: string }> }

async function detectPlatform(): Promise<Platform> {
  const ua = navigator.userAgent
  const os: Platform['os'] = ua.includes('Windows') ? 'windows'
    : /Mac OS X|Macintosh/.test(ua) ? 'macos'
    : 'linux'
  let arch = 'x86_64'
  try {
    const hints = await (navigator as Navigator & { userAgentData?: NavUAData }).userAgentData
      ?.getHighEntropyValues(['architecture'])
    if (/arm|aarch64/i.test(hints?.architecture || '')) arch = 'aarch64'
  } catch { /* Safari/Firefox */ }
  if (/arm64|aarch64/i.test(ua)) arch = 'aarch64'
  if (os === 'macos' && arch === 'x86_64') {
    try {
      const gl = document.createElement('canvas').getContext('webgl')
      const dbg = gl && gl.getExtension('WEBGL_debug_renderer_info')
      const renderer = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : ''
      if (/Apple M\d/.test(renderer)) arch = 'aarch64'
    } catch { /* no WebGL */ }
  }
  return { os, arch }
}

/* ------------------------------------------------------------------
 * Asset matching. Names are matched by PREFIX + SUFFIX, never by exact
 * string, so both naming schemes resolve:
 *   legacy:    dologctl-linux-x86_64          (pre-v0.1.0 published assets)
 *   versioned: dologctl-v0.1.0-linux-x86_64   (release.yml)
 * ------------------------------------------------------------------ */
const CLI_PREFIX = 'dologctl'
const LIB_PREFIXES = ['libdologger_core', 'dologger_core']
/* Official plugins ship as ONE bundle library per platform
   (naming rule: dologger-official-plugins-{tag}-{os}-{arch}.{ext}),
   which hosts every official plugin — no per-plugin files. */
const PLUGIN_PREFIX = 'dologger-official-plugins'
export const OS_ORDER: Platform['os'][] = ['linux', 'windows', 'macos']
export const ARCH_ORDER = ['x86_64', 'aarch64', 'i686', 'armv7', 'riscv64']
const ARCH_RE = /(?:x86_64|aarch64|i686|armv7|riscv64)/
const ASSET_TAIL = new RegExp('-((?:linux|windows|macos))-(' + ARCH_RE.source + ')(?:\\.exe|\\.so|\\.dll|\\.dylib)?$')

/** The CLI asset for this platform in a given release (either naming scheme). */
export function assetFor(release: Release, os: Platform['os'], arch: string): ReleaseAsset | null {
  const assets = (release.assets && release.assets.length)
    ? release.assets
    : FALLBACK_RELEASES[0].assets
  const want = '-' + os + '-' + arch + (os === 'windows' ? '.exe' : '')
  const hit = assets.find(function (a) { return a.name.startsWith(CLI_PREFIX) && a.name.endsWith(want) })
  return hit || null
}

/** Human-readable asset name for the popup rows: strip the os-arch tail
 *  (already conveyed by the group headers) and, for versioned names, the
 *  -{tag} segment. The bundle asset keeps its full stem
 *  (`dologger-official-plugins`); the complete name stays in `title`. */
export function shortName(name: string, tag: string): string {
  let base = name.replace(ASSET_TAIL, '')
  if (base.endsWith('-' + tag)) base = base.slice(0, base.length - tag.length - 1)
  return base
}

export interface AssetRow {
  os: Platform['os']
  cli?: ReleaseAsset
  lib?: ReleaseAsset
  /** Official plugins shipped with this release (several per OS/arch). */
  plugins?: ReleaseAsset[]
}
export interface ArchGroup { arch: string; rows: AssetRow[] }

/** Group a release's assets by architecture → OS (both naming schemes). */
export function groupAssets(release: Release): ArchGroup[] {
  const assets = (release.assets && release.assets.length)
    ? release.assets
    : FALLBACK_RELEASES[0].assets
  const byArch = new Map<string, Map<Platform['os'], AssetRow>>()

  for (const a of assets) {
    const isCli = a.name.startsWith(CLI_PREFIX)
    const isLib = LIB_PREFIXES.some(function (p) { return a.name.startsWith(p) })
    const isPlugin = a.name.startsWith(PLUGIN_PREFIX)
    if (!isCli && !isLib && !isPlugin) continue
    const m = a.name.match(ASSET_TAIL)
    if (!m) continue
    const os = m[1] as Platform['os']
    const arch = m[2]
    let osMap = byArch.get(arch)
    if (!osMap) { osMap = new Map(); byArch.set(arch, osMap) }
    const row = osMap.get(os) || { os }
    if (isCli) row.cli = a
    else if (isLib) row.lib = a
    else (row.plugins || (row.plugins = [])).push(a)
    osMap.set(os, row)
  }

  const groups: ArchGroup[] = []
  for (const arch of ARCH_ORDER) {
    const osMap = byArch.get(arch)
    if (!osMap) continue
    const rows = OS_ORDER.filter(function (os) { return osMap.has(os) })
      .map(function (os) { return osMap.get(os) as AssetRow })
    groups.push({ arch, rows })
  }
  // any arch outside the fixed order (future-proof)
  for (const [arch, osMap] of byArch) {
    if (ARCH_ORDER.indexOf(arch) >= 0) continue
    groups.push({ arch, rows: Array.from(osMap.values()) })
  }
  return groups
}

function resolveDownload(releases: Release[] | null, os: Platform['os'], arch: string): string {
  const latest = releases && releases.length ? releases[0] : FALLBACK_RELEASES[0]
  const hit = assetFor(latest, os, arch)
  if (hit && hit.browser_download_url) return hit.browser_download_url
  return latest.html_url || 'https://github.com/Nekolio/DoLogger/releases'
}

async function init(): Promise<SiteData> {
  const [releases, repo, contributors, benchmarks] = await Promise.all([
    getReleases(), getRepo(), getContributors(), getBenchmarks()
  ])
  const latest = releases.length ? releases[0] : FALLBACK_RELEASES[0]
  const platform = await detectPlatform()
  return {
    releases,
    latest,
    repo,
    contributors,
    benchmarks,
    platform,
    downloadUrl: resolveDownload(releases, platform.os, platform.arch)
  }
}

/* ------------------------------------------------------------------
 * Vue store — a module-level ref shared by every component.
 * ---------------------------------------------------------------- */
const siteData: Ref<SiteData | null> = ref(null)

export function useSiteData(): Ref<SiteData | null> {
  return siteData
}

export async function loadSiteData(): Promise<SiteData> {
  if (!siteData.value) siteData.value = await init()
  return siteData.value
}

/* ------------------------------------------------------------------
 * Version selection — which release the page currently targets. The
 * hero's download button, os label, and the grouped asset panel all
 * derive from this; default is the latest release. Module-level ref so
 * every component follows the choice reactively.
 * ---------------------------------------------------------------- */
const selectedTag = ref<string | null>(null)

export function useSelectedTag(): Ref<string | null> {
  return selectedTag
}

export function selectRelease(tag: string | null): void {
  selectedTag.value = tag
}

/** The release currently targeted (selected, else latest). */
export function pickRelease(releases: Release[] | null): Release {
  const rels = releases && releases.length ? releases : FALLBACK_RELEASES
  if (!selectedTag.value) return rels[0]
  return rels.find(function (r) { return r.tag_name === selectedTag.value }) || rels[0]
}
