<script setup lang="ts">
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
/* sec-chain and sec-trust move out of this list: they are the section
   labels above the audit chain and the trust chips respectively. */
const items = ['sec-sandbox', 'sec-worm', 'sec-priority']
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
    <ul class="feat-list">
      <li v-for="key in items" :key="key">{{ t(key) }}</li>
    </ul>

    <div class="card-label">{{ t('sec-chain') }}</div>
    <!-- Ed25519 audit chain: LSN boxes linked by prev_hash — the
         signed-at-assembly guarantee made visible. The chain glows
         sequentially in an infinite loop (LSN 1 → 2 → 3). -->
    <div class="chain-native" role="img" :aria-label="t('sec-chain-detail')">
      <div v-for="b in chain" :key="b.lsn" class="chain-node">
        <span class="chain-dot"></span>
        <span class="chain-lsn">LSN {{ b.lsn }}</span>
        <span class="chain-hash">{{ b.hash }}</span>
        <span class="chain-signed">✓ {{ t('sec-chain-signed') }}</span>
      </div>
    </div>
    <div class="card-label">{{ t('sec-trust') }}</div>
    <div class="trust-chips">
      <span class="trust-chip blue">Blue</span>
      <span class="trust-chip amber">Yellow</span>
      <span class="trust-chip red">Red</span>
    </div>
    <div class="card-caption">{{ t('sec-chain-detail') }}</div>
  </div>
</template>

<style scoped>
/* the trust-level chips now follow their own .card-label (the shared label
   already provides the section gap), so only the internal chip spacing is
   scoped here */
.trust-chips {
  gap: 0.5rem;
  row-gap: 0.45rem;
  align-items: center;
}
.trust-chip { line-height: 1.5; }
</style>
