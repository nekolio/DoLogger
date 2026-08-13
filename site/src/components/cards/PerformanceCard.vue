<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSiteData, FALLBACK_BENCHMARKS } from '../../data'

const { t } = useI18n()
const siteData = useSiteData()

const RELEASES_URL = 'https://github.com/Nekolio/DoLogger/releases'

const benchmarks = computed(() => siteData.value?.benchmarks ?? FALLBACK_BENCHMARKS)
const envs = computed(() => {
  const e = (benchmarks.value.environments || []).filter(x => x && (x.p50 || x.throughput))
  return e.length ? e : [FALLBACK_BENCHMARKS.environments[0]]
})
const rows = (env: { p50?: string | null; throughput?: string | null; signed?: string | null }) => {
  const r: [string, string | null | undefined][] = [['perf-p50', env.p50], ['perf-thru', env.throughput]]
  if (env.signed) r.push(['perf-signed', env.signed])
  return r.filter(([, v]) => !!v)
}

/* The cyber gauge: the first environment's throughput mapped onto a
   semicircular dial (10M rec/s = full scale). The needle sweeps to the
   value on load (CSS animation driven by --gauge-angle) and the readout
   gets a brief glitch pop. Reduced-motion users see only the final
   state — both animations are gated in CSS. */
const through = computed(() => {
  const tp = envs.value[0]?.throughput ?? ''
  const m = tp.match(/^([\d.]+)\s*([MK]?)\s*(.*)$/)
  if (!m || !m[1]) return { num: '—', unit: '', pct: 0 }
  const perSec = m[2] === 'M' ? parseFloat(m[1]) * 1e6 : m[2] === 'K' ? parseFloat(m[1]) * 1e3 : parseFloat(m[1])
  const pct = Math.max(0, Math.min(100, Math.round((perSec / 10e6) * 100)))
  return { num: m[1] + m[2], unit: m[3], pct }
})
const gaugeDash = computed(() => (0.08 + 0.92 * (through.value.pct / 100)).toFixed(3))
const gaugeAngle = computed(() => (through.value.pct / 100) * 180 - 90)
const TICKS = Array.from({ length: 21 }, (_, i) => i)
</script>

<template>
  <div>
    <div class="cyber-gauge" role="img" :aria-label="(through.num !== '—' ? through.num + ' ' + (through.unit || 'rec/s') : '')">
      <svg viewBox="0 0 200 112">
        <defs>
          <linearGradient id="gauge-grad" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stop-color="#7FD5FF" />
            <stop offset="0.5" stop-color="#C792EA" />
            <stop offset="1" stop-color="#F472D0" />
          </linearGradient>
        </defs>
        <g class="gauge-ticks">
          <line v-for="i in TICKS" :key="i" :transform="`rotate(${-90 + i * 9} 100 100)`" x1="100" y1="14" x2="100" y2="24" />
        </g>
        <path class="gauge-track" d="M 22 100 A 78 78 0 0 1 178 100" pathLength="1" />
        <path class="gauge-arc" d="M 22 100 A 78 78 0 0 1 178 100" pathLength="1" :stroke-dasharray="gaugeDash + ' 1'" />
        <g class="gauge-needle" :style="{ '--gauge-angle': gaugeAngle + 'deg' }">
          <line x1="100" y1="100" x2="100" y2="34" />
          <circle cx="100" cy="100" r="5" />
        </g>
        <text class="gauge-num" x="100" y="86" text-anchor="middle">{{ through.num }}</text>
        <text class="gauge-unit" x="100" y="102" text-anchor="middle">{{ through.unit || 'rec/s' }}</text>
      </svg>
    </div>

    <template v-for="env in envs" :key="env.label">
      <div class="env-label">{{ env.label }}<template v-if="benchmarks.tag"> · {{ benchmarks.tag }}</template></div>
      <div v-for="[key, value] in rows(env)" :key="key" class="perf-row">
        <span class="k">{{ t(key) }}</span><span class="v">{{ value }}</span>
      </div>
    </template>
    <template v-if="benchmarks.criterion && benchmarks.criterion.length">
      <div class="env-label">{{ t('perf-crit') }}</div>
      <table class="crit-table">
        <tr v-for="c in benchmarks.criterion" :key="c.name">
          <td>{{ c.name }}</td>
          <td>{{ c.value }}<template v-if="c.note"> · {{ c.note }}</template></td>
        </tr>
      </table>
    </template>
    <a class="card-link" :href="RELEASES_URL">{{ t('perf-link') }}</a>
  </div>
</template>
