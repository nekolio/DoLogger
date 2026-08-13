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
</script>

<template>
  <div>
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
