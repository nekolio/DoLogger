/* demo/logs.ts — the two terminal streams on page 2.
 *
 * BEFORE: the pre-migration stream. Style A — plain timestamped lines,
 * rendered grey and uncolored, slow, and full of the failure modes the
 * demo is about: dropped records, blocked event loops, plaintext
 * secrets, no integrity chain, SLO misses, unbounded disk growth.
 *
 * AFTER: the DoLogger stream. Real format `[ns] [LEVEL] [component]`
 * from core/src/sys/internal_log.rs, colored levels, fast, and every
 * line is a DoLogger strength: real measured numbers, the boot canon
 * from hero.svg, the audit chain.
 *
 * The after-stream is intentionally all INFO (one AUDIT) — the demo is
 * "everything is healthy now".
 */

export interface DemoLog {
  /** Which stream this line belongs to. */
  side: 'before' | 'after'
  /** Full formatted line, pre-tokenized (levels colored via CSS). */
  text: string
  /** Class added to the line element for coloring. */
  cls: string
}

export const BEFORE_LOGS: DemoLog[] = [
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:01.327 [ERROR] ring-buffer: dropped 128 records — queue overflow' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:02.881 [WARN]  file-writer: flush took 2.4s, blocked the event loop' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:03.102 [ERROR] tls: handshake timeout after 5s — connection reset' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:03.910 [WARN]  auth: password fields written to plaintext log' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:04.257 [ERROR] crash: plugin_http segfaulted — no recovery path' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:05.603 [WARN]  audit: no integrity chain — tampering undetectable' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:06.114 [ERROR] perf: p99 latency 3.2s — SLO exceeded' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:07.775 [WARN]  disk: log grew 8.4 GB in the last hour' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:08.340 [ERROR] replay: duplicate events after crash — not idempotent' },
  { side: 'before', cls: 'old',   text: '2026-08-14 12:00:09.012 [WARN]  shard-3: rebalancing lost 42 messages' }
]

export const AFTER_LOGS: DemoLog[] = [
  { side: 'after', cls: 'info',   text: '[0]  [INFO]  [core]     Hello DoLogger — zero-copy ring buffer armed' },
  { side: 'after', cls: 'info',   text: '[1]  [INFO]  [plugin]   4 sandboxed plugins loaded · trust BLUE' },
  { side: 'after', cls: 'info',   text: '[2]  [INFO]  [audit]    Ed25519 chain armed — every record signed at assembly' },
  { side: 'after', cls: 'info',   text: '[3]  [INFO]  [pipeline] 7-stage pipeline online' },
  { side: 'after', cls: 'info',   text: '[4]  [INFO]  [perf]     p50 102 ns · 9.78M rec/s — zero heap allocation' },
  { side: 'after', cls: 'info',   text: '[5]  [INFO]  [perf]     GH runner: p50 120 ns · 5.06M rec/s' },
  { side: 'after', cls: 'info',   text: '[6]  [INFO]  [sink]     fan-out console · file · WORM · kafka — non-blocking' },
  { side: 'after', cls: 'audit',  text: '[7]  [AUDIT] [audit]    user 42 deleted record #7 — signed + WORM, non-downgradable' },
  { side: 'after', cls: 'info',   text: '[8]  [INFO]  [core]     never drop · never block · never lie' },
  { side: 'after', cls: 'info',   text: '[9]  [INFO]  [core]     Shutdown: flushed 0 remaining records' }
]

/** Line-render interval per side, ms/line. The after stream is ~6× faster. */
export const SPEED: Record<DemoLog['side'], number> = { before: 240, after: 40 }
