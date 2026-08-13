<script setup lang="ts">
import { useI18n } from 'vue-i18n'

defineProps<{ expanded?: boolean }>()
const { t } = useI18n()
const items = ['sec-chain', 'sec-sandbox', 'sec-trust', 'sec-worm', 'sec-priority']
const chain = [
  { lsn: 1, x: 10 },
  { lsn: 2, x: 116 },
  { lsn: 3, x: 222 }
]
</script>

<template>
  <div>
    <ul>
      <li v-for="key in items" :key="key">{{ t(key) }}</li>
    </ul>

    <div v-if="expanded" class="card-detail">
      <!-- Ed25519 audit chain: LSN boxes linked by prev_hash — the
           signed-at-assembly guarantee made visible -->
      <svg class="sec-chain" viewBox="0 0 320 104" role="img" :aria-label="t('sec-chain-detail')">
        <defs>
          <linearGradient id="chain-grad" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stop-color="#7FD5FF" />
            <stop offset="0.5" stop-color="#C792EA" />
            <stop offset="1" stop-color="#F472D0" />
          </linearGradient>
          <marker id="chain-arrow" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto">
            <path d="M0 0 L7 3.5 L0 7 Z" class="chain-arrowhead" />
          </marker>
        </defs>
        <template v-for="(b, i) in chain" :key="b.lsn">
          <rect :x="b.x" y="22" width="92" height="44" rx="9" class="chain-box" />
          <text :x="b.x + 46" y="40" text-anchor="middle" class="chain-lsn">LSN {{ b.lsn }}</text>
          <text :x="b.x + 46" y="55" text-anchor="middle" class="chain-hash">prev_hash</text>
          <path v-if="i < chain.length - 1" :d="`M ${b.x + 92} 44 H ${b.x + 106}`"
                class="chain-link" marker-end="url(#chain-arrow)" />
        </template>
      </svg>
      <div class="trust-chips">
        <span class="trust-chip blue">Blue</span>
        <span class="trust-chip violet">Yellow</span>
        <span class="trust-chip magenta">Red</span>
        <span class="trust-caption">{{ t('sec-trust') }}</span>
      </div>
      <div class="card-caption">{{ t('sec-chain-detail') }}</div>
    </div>
  </div>
</template>
