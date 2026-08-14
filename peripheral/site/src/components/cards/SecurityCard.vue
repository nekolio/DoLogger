<script setup lang="ts">
import { useI18n } from 'vue-i18n'

defineProps<{ expanded?: boolean }>()
const { t } = useI18n()
const items = ['sec-chain', 'sec-sandbox', 'sec-trust', 'sec-worm', 'sec-priority']
/* illustrative prev_hash tail per link — real hashes are 64 hex chars,
   so the native chain shows a short form the way explorers do */
const chain = [
  { lsn: 1, hash: 'a1f3…9c4d' },
  { lsn: 2, hash: '7b92…d08e' },
  { lsn: 3, hash: 'c40a…6f71' }
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
      <div class="chain-native" role="img" :aria-label="t('sec-chain-detail')">
        <div v-for="b in chain" :key="b.lsn" class="chain-node">
          <span class="chain-dot"></span>
          <span class="chain-lsn">LSN {{ b.lsn }}</span>
          <span class="chain-hash">{{ b.hash }}</span>
          <span class="chain-signed">✓ {{ t('sec-chain-signed') }}</span>
        </div>
      </div>
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
