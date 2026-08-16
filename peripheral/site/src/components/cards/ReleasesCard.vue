<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSiteData } from '../../data'

const { t } = useI18n()
const siteData = useSiteData()

const RELEASES_URL = 'https://github.com/Nekolio/DoLogger/releases'
const COMMIT_URL = 'https://github.com/Nekolio/DoLogger/commit'

interface Commit { subject: string; hash: string | null }
interface Changelog { heading: string | null; commits: Commit[] }

const latest = computed(() => siteData.value?.latest ?? null)

function fmtDate(iso?: string): string {
  if (!iso) return ''
  const d = new Date(iso)
  return isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10)
}

/* Extract the changelog portion of the release body. The body is the
 * two-section (EN/中文) markdown built by generate-release-notes.sh; the
 * changelog is the final section — "## Changelog / 更新日志" — with a bold
 * "Changes since <prev> / 自 <prev> 以来的变更" (or "Initial release / 首次
 * 发布") line followed by "- subject (hash)" commit bullets. */
function parseChangelog(body: string): Changelog {
  const lines = body.split(/\r?\n/)
  let start = -1
  for (let i = 0; i < lines.length; i++) {
    if (/^#{2,3}\s*changelog\b/i.test(lines[i])) { start = i; break }
  }
  /* no heading found → scan the whole body (a hand-written body may be a
     bare commit list); heading found → only the section after it */
  const from = start === -1 ? 0 : start + 1
  const commits: Commit[] = []
  let heading: string | null = null
  for (let i = from; i < lines.length; i++) {
    const line = lines[i]
    if (start !== -1 && i > start + 1 && /^#{2,3}\s/.test(line)) break // next section
    if (!heading) {
      const m = line.match(/^\*\*(.+?)\*\*\s*$/)
      if (m) { heading = m[1].trim(); continue }
    }
    const c = line.match(/^[-*]\s+(.+?)\s+\(([0-9a-f]{6,40})\)\s*$/i)
    if (c) { commits.push({ subject: c[1].trim(), hash: c[2] }); continue }
    const plain = line.match(/^[-*]\s+(.+?)\s*$/)
    if (plain && plain[1].trim() && !/^_?\(?no commits/i.test(plain[1].trim())) {
      commits.push({ subject: plain[1].trim(), hash: null })
    }
  }
  return { heading, commits }
}

/* Static list for the pre-load / empty-body edge case (site data not yet
 * resolved, or a cached release from before `body` was carried). Facts derive
 * from the current repo docs — the sink layer is built-in, not plugin-based. */
const FALLBACK_COMMITS: Commit[] = [
  { subject: 'feat: cross-platform high-security logging engine', hash: null },
  { subject: 'feat: Ed25519-signed audit chain (LSN + prev_hash, offline verify-log)', hash: null },
  { subject: 'feat: 11 built-in sinks — Console, File, Callback, Kafka, Syslog, Webhook, SQLite, WORM, Security, Shared Memory, OTel', hash: null },
  { subject: 'feat: 9 plugin types with Blue/Yellow/Red trust levels + sandbox isolation', hash: null },
  { subject: 'feat: lock-free hot path — CAS ring buffer + Treiber object pool (zero-alloc submit)', hash: null },
  { subject: 'feat: WORM sink + 6 non-downgradable security items', hash: null },
  { subject: 'feat: dologctl CLI — init, run, plugin, verify-log, perf, record/replay', hash: null },
  { subject: 'feat: official plugins bundle — formatter-json, formatter-text, filter-level, field-container', hash: null }
]

const changelog = computed<Changelog>(() => {
  const body = latest.value?.body
  if (body) {
    const parsed = parseChangelog(body)
    if (parsed.commits.length) return parsed
  }
  return { heading: null, commits: FALLBACK_COMMITS }
})
</script>

<template>
  <div>
    <div class="chlog-head">
      <a class="chlog-tag" :href="latest?.html_url || RELEASES_URL">{{ latest?.tag_name || 'v0.1.0' }}</a>
      <span v-if="latest?.prerelease" class="prerelease-badge">{{ t('rel-prerelease') }}</span>
      <span class="chlog-date">{{ fmtDate(latest?.published_at) }}</span>
    </div>

    <p v-if="changelog.heading" class="chlog-heading">{{ changelog.heading }}</p>
    <p v-else class="chlog-heading">{{ t('chlog-heading') }}</p>

    <ul class="chlog-list">
      <li v-for="(c, i) in changelog.commits" :key="i">
        <span class="chlog-subject">{{ c.subject }}</span>
        <a v-if="c.hash" class="chlog-hash" :href="COMMIT_URL + '/' + c.hash" :title="c.hash" target="_blank" rel="noopener">{{ c.hash }}</a>
      </li>
    </ul>

    <div class="card-caption">{{ t('chlog-caption') }}</div>
    <a class="card-link" :href="RELEASES_URL">{{ t('rel-all') }}</a>
  </div>
</template>

<style scoped>
.chlog-head {
  display: flex;
  align-items: baseline;
  gap: 0.5rem;
  margin-bottom: 0.3rem;
}
.chlog-tag {
  color: var(--accent);
  font-family: ui-monospace, 'Cascadia Code', 'JetBrains Mono', Consolas, monospace;
  font-size: var(--card-body);
  font-weight: 600;
  text-decoration: none;
}
.chlog-tag:hover { text-decoration: underline; }
.chlog-date {
  margin-left: auto;
  color: var(--text-dim);
  font-size: var(--card-meta);
  font-family: ui-monospace, 'Cascadia Code', 'JetBrains Mono', Consolas, monospace;
}
.chlog-heading {
  font-size: var(--card-meta);
  color: var(--text-dim);
  margin: 0.35rem 0 0.25rem;
}
.chlog-list {
  list-style: none;
  padding: 0;
  margin: 0;
}
.chlog-list li {
  display: flex;
  align-items: baseline;
  gap: 0.5rem;
  padding: 0.22rem 0;
  border-bottom: 1px solid var(--row-border);
  font-size: var(--card-body);
}
.chlog-list li:last-child { border-bottom: none; }
.chlog-subject {
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--text);
}
.chlog-hash {
  flex: none;
  color: var(--text-dim);
  font-family: ui-monospace, 'Cascadia Code', 'JetBrains Mono', Consolas, monospace;
  font-size: var(--card-meta);
  text-decoration: none;
}
.chlog-hash:hover { color: var(--accent); text-decoration: underline; }
</style>
