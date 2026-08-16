<script setup lang="ts">
import { useI18n } from 'vue-i18n'

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
    <div class="card-label">{{ t('arch-hot') }}</div>
    <!-- the pipeline plays as a horizontal marquee (track = stages × 2,
         loop-scrolled seamlessly by useAutoLoopScroll). Full names are
         never truncated — the track is content-sized. -->
    <div class="pipe-marquee" role="img" :aria-label="t('arch-detail')">
      <div class="pipe-track">
        <template v-for="dup in 2" :key="dup">
          <div v-for="(stage, i) in stages" :key="dup + '-' + stage" class="pipe-stage">
            <span class="pipe-dot"></span>
            <span class="pipe-name">{{ stage }}</span>
            <span class="pipe-sub">{{ subs[i] }}</span>
          </div>
        </template>
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
    <a class="card-link" :href="WIKI_URL + '/en_US-ArchitectureReference'">{{ t('arch-link') }}</a>
  </div>
</template>

<style scoped>
/* The pipeline chain wraps to fit its card — flex-wrap with a tracked
   width means every stage stays fully readable at ANY viewport width
   (the chain is content, not a decorative marquee; the duplicated
   marquee copy is dropped so the chain reads once). `min-width: 0`
   beats style.css's `min-width: max-content` — without it the track
   refuses to shrink and the chain overflows instead of wrapping. */
.pipe-marquee { overflow: visible; }
.pipe-track {
  display: flex;
  flex-wrap: wrap;
  gap: 0.35rem 0.6rem;
  min-width: 0;
  width: 100%;
}
.pipe-stage:nth-child(n + 8) { display: none; } /* drop the duplicated copy */
/* In the wrapped layout the inter-stage arrows would dangle at line
   ends (they're drawn on every non-last DOM child, and after wrapping
   the first row's last stage is NOT the DOM :last-child). The chain
   reads as a flow — drop the arrows entirely. */
.pipe-stage::after { display: none; }
</style>
