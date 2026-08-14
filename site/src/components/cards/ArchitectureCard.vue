<script setup lang="ts">
import { useI18n } from 'vue-i18n'

defineProps<{ expanded?: boolean }>()
const { t } = useI18n()
const WIKI_URL = 'https://github.com/Nekolio/DoLogger/wiki'
const stages = ['pre-filter', 'filter', 'field provider', 'assembly', 'process', 'format', 'sink fan-out']
/* one-liner per stage, shown under its name inside the pipe tile */
const subs = [
  'thread-safe', 'sync', 'per-event', 'Ed25519 signed',
  'lock-free · zero-alloc', 'pluggable', '11 sinks'
]
</script>

<template>
  <div>
    <div class="arch-hot">{{ t('arch-hot') }}</div>
    <div class="arch-flow">
      <template v-for="(stage, i) in stages" :key="stage">
        <span v-if="i > 0" class="sep">→</span>
        <span :style="{ '--i': i }">{{ stage }}</span>
      </template>
    </div>
    <a class="card-link" :href="WIKI_URL + '/en_US-ArchitectureReference'">{{ t('arch-link') }}</a>

    <div v-if="expanded" class="card-detail">
      <!-- native pipeline: numbered stage tiles linked by arrows -->
      <div class="arch-pipe" role="img" :aria-label="t('arch-detail')">
        <div v-for="(stage, i) in stages" :key="stage" class="pipe-stage">
          <span class="pipe-dot"></span>
          <span class="pipe-name">{{ stage }}</span>
          <span class="pipe-sub">{{ subs[i] }}</span>
        </div>
      </div>
      <!-- the lock-free hot path, called out as a chip strip -->
      <div class="arch-hotpath">
        <span class="hp-tag">{{ t('arch-hot-tag') }}</span>
        <span class="hp-chip">{{ t('arch-hot-cas') }}</span>
        <span class="hp-chip">{{ t('arch-hot-treiber') }}</span>
        <span class="hp-chip">{{ t('arch-hot-zero') }}</span>
      </div>
      <div class="card-caption">{{ t('arch-detail') }}</div>
    </div>
  </div>
</template>
