/* demo/logs.ts — the two terminal streams on page 2.
 *
 * Timestamps are NOT stored here: PageDemo stamps every line at push
 * time so the terminal reads like a live process. Both streams get the
 * same live wall clock (HH:MM:SS.mmm). The after stream's human-
 * readable format is provided by a DoLogger plugin — dologger's
 * text rendering is pluggable via the formatter plugins, which is
 * exactly the point of the formatter/field_container plugin lines in
 * the boot canon below.
 *
 * BEFORE: the pre-migration stream. Style A — plain timestamped lines,
 * slow, and full of the failure modes the demo is about: dropped
 * records, blocked event loops, plaintext secrets, no integrity chain,
 * SLO misses, unbounded disk growth. Severity is visible at a glance:
 * ERROR lines glow red, WARN lines amber.
 *
 * AFTER: the DoLogger stream. Real format `[ns] [LEVEL] [component]`
 * from core/src/sys/internal_log.rs (wall-clock stamping shown live),
 * colored levels, fast, and every line is a DoLogger strength: the
 * boot canon, plugin trust levels, the Ed25519 audit chain, 7-stage
 * pipeline + priority levels, measured perf, the sink fan-out, and the
 * shutdown `ok` line.
 */

export interface DemoLog {
  /** Which stream this line belongs to. */
  side: 'before' | 'after'
  /** Line body without the timestamp prefix (stamped live on push). */
  text: string
  /** Class added to the line element for coloring. */
  cls: string
}

export const BEFORE_LOGS: DemoLog[] = [
  { side: 'before', cls: 'error', text: '[ERROR] ring-buffer: dropped 128 records — queue overflow' },
  { side: 'before', cls: 'warn',  text: '[WARN]  file-writer: flush took 2.4s, blocked the event loop' },
  { side: 'before', cls: 'error', text: '[ERROR] tls: handshake timeout after 5s — connection reset' },
  { side: 'before', cls: 'warn',  text: '[WARN]  auth: password fields written to plaintext log' },
  { side: 'before', cls: 'error', text: '[ERROR] crash: plugin_http segfaulted — no recovery path' },
  { side: 'before', cls: 'warn',  text: '[WARN]  audit: no integrity chain — tampering undetectable' },
  { side: 'before', cls: 'error', text: '[ERROR] perf: p99 latency 3.2s — SLO exceeded' },
  { side: 'before', cls: 'warn',  text: '[WARN]  disk: log grew 8.4 GB in the last hour' },
  { side: 'before', cls: 'error', text: '[ERROR] replay: duplicate events after crash — not idempotent' },
  { side: 'before', cls: 'warn',  text: '[WARN]  shard-3: rebalancing lost 42 messages' }
]

export const AFTER_LOGS: DemoLog[] = [
  { side: 'after', cls: 'info',   text: '[INFO]  [core]     DoLogger v0.0.1 — ring buffer + Treiber pool, zero heap on submit' },
  { side: 'after', cls: 'plugin', text: '[INFO]  [plugin]   trust gate: Ed25519 signatures verified · Blue/Yellow/Red' },
  { side: 'after', cls: 'plugin', text: '[INFO]  [plugin]   4 plugins loaded · trust BLUE · api v3' },
  { side: 'after', cls: 'plugin', text: '[INFO]  [plugin]   formatter_text: attached — human-readable text format' },
  { side: 'after', cls: 'info',   text: '[INFO]  [audit]    Ed25519 chain armed — genesis prev_hash 0000…, every record signed' },
  { side: 'after', cls: 'info',   text: '[INFO]  [pipeline] 7-stage pipeline online · 7 priority levels, domain inheritance' },
  { side: 'after', cls: 'info',   text: '[INFO]  [perf]     p50 102 ns · 9.78M rec/s — lock-free hot path, zero heap allocation' },
  { side: 'after', cls: 'info',   text: '[INFO]  [perf]     GH runner: p50 120 ns · 5.06M rec/s' },
  { side: 'after', cls: 'info',   text: '[INFO]  [sink]     fan-out: console · file · kafka · WORM — 11 sinks, non-blocking' },
  { side: 'after', cls: 'audit',  text: '[AUDIT] [audit]    LSN 7: user 42 deleted record #7 — signed Ed25519, prev_hash 0x9f2e…, WORM' },
  { side: 'after', cls: 'plugin', text: '[INFO]  [plugin]   field_container: structured fields attached' },
  { side: 'after', cls: 'warn',   text: '[WARN]  [policy]   non-downgradable policy — downgrade attempt rejected' },
  { side: 'after', cls: 'info',   text: '[INFO]  [core]     never drop · never block · never lie' },
  { side: 'after', cls: 'ok',     text: '[INFO]  [core]     Shutdown: flushed 0 remaining records — chain intact' }
]

/** Line-render interval per side, ms/line. The after stream is ~6× faster. */
export const SPEED: Record<DemoLog['side'], number> = { before: 240, after: 40 }
