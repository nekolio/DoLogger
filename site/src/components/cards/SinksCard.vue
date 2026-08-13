<script setup lang="ts">
import { useI18n } from 'vue-i18n'

defineProps<{ expanded?: boolean }>()
const { t } = useI18n()
const sinks = [
  'Console', 'File', 'Callback', 'Kafka', 'Syslog', 'Webhook',
  'SQLite', 'WORM', 'Security', 'Shared Memory', 'OTel'
]
</script>

<template>
  <div>
    <div class="sink-tags">
      <span v-for="(name, i) in sinks" :key="name" :style="{ '--i': i }">{{ name }}</span>
    </div>
    <div class="sinks-note">{{ t('sinks-note') }}</div>

    <div v-if="expanded" class="card-detail">
      <!-- fan-out: one core → every sink, exactly how the pipeline ends -->
      <svg class="sink-fan" viewBox="0 0 320 132" role="img" :aria-label="t('sinks-detail')">
        <defs>
          <linearGradient id="fan-grad" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stop-color="#7FD5FF" />
            <stop offset="0.5" stop-color="#C792EA" />
            <stop offset="1" stop-color="#F472D0" />
          </linearGradient>
        </defs>
        <circle cx="30" cy="66" r="17" class="fan-core" />
        <text x="30" y="70" text-anchor="middle" class="fan-core-label">core</text>
        <template v-for="(name, i) in sinks" :key="name">
          <line :x1="47" :y1="66" :x2="86 + (i % 4) * 60 + 27" :y2="14 + Math.floor(i / 4) * 40 + 12" class="fan-line" />
          <rect :x="86 + (i % 4) * 60" :y="14 + Math.floor(i / 4) * 40" width="54" height="24" rx="6" class="fan-node" />
          <text :x="86 + (i % 4) * 60 + 27" :y="30 + Math.floor(i / 4) * 40" text-anchor="middle" class="fan-node-label">{{ name }}</text>
        </template>
      </svg>
      <div class="card-caption">{{ t('sinks-detail') }}</div>
    </div>
  </div>
</template>
