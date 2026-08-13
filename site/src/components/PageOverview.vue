<script setup lang="ts">
import { ref, watch, nextTick, onBeforeUnmount, type Component } from 'vue'
import { useI18n } from 'vue-i18n'
import PerformanceCard from './cards/PerformanceCard.vue'
import SecurityCard from './cards/SecurityCard.vue'
import SinksCard from './cards/SinksCard.vue'
import ArchitectureCard from './cards/ArchitectureCard.vue'
import ReleasesCard from './cards/ReleasesCard.vue'
import CommunityCard from './cards/CommunityCard.vue'

const { t } = useI18n()
const REPO_URL = 'https://github.com/Nekolio/DoLogger'

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')

const cards: { key: string; icon: string; title: string; comp: Component }[] = [
  { key: 'perf', icon: './assets/icons.svg#icon-gauge', title: 'card-perf', comp: PerformanceCard },
  { key: 'sec', icon: './assets/icons.svg#icon-shield', title: 'card-sec', comp: SecurityCard },
  { key: 'sinks', icon: './assets/icons.svg#icon-plug', title: 'card-sinks', comp: SinksCard },
  { key: 'arch', icon: './assets/icons.svg#icon-branch', title: 'card-arch', comp: ArchitectureCard },
  { key: 'rel', icon: './assets/icons.svg#icon-tag', title: 'card-rel', comp: ReleasesCard },
  { key: 'comm', icon: './assets/icons.svg#icon-users', title: 'card-comm', comp: CommunityCard }
]

/* ── expand state machine (bubble-like but stable) ──────────────────
 * Enter with a 250ms dwell so sweeping the mouse never trips a card;
 * leaving costs a 350ms grace. Moving between cards cancels both
 * timers and switches directly — one expanded card at all times.
 * The frosted surface (`settled`) applies only after the FLIP motion
 * ends: backdrop-filter re-samples every frame, so animating through
 * it would be per-frame blur churn. */
const expanded = ref<number | null>(null)
const settled = ref<number | null>(null)
let enterTimer = 0
let exitTimer = 0
let settleTimer = 0

function clearTimers() {
  if (enterTimer) { clearTimeout(enterTimer); enterTimer = 0 }
  if (exitTimer) { clearTimeout(exitTimer); exitTimer = 0 }
  if (settleTimer) { clearTimeout(settleTimer); settleTimer = 0 }
}

const FLIP_EASE = 'cubic-bezier(0.22, 1, 0.36, 1)'

function flip(commit: () => void) {
  if (REDUCED_MOTION || !finePointer.matches) { commit(); settleSoon(); return }
  const cardsEl = Array.from(document.querySelectorAll<HTMLElement>('#page3 .card'))
  const first = cardsEl.map(c => c.getBoundingClientRect())
  commit()
  nextTick(() => {
    const second = cardsEl.map(c => c.getBoundingClientRect())
    for (let i = 0; i < cardsEl.length; i++) {
      const el = cardsEl[i]
      const f = first[i]
      const s = second[i]
      const dx = f.left - s.left
      const dy = f.top - s.top
      const sx = s.width > 0 ? f.width / s.width : 1
      const sy = s.height > 0 ? f.height / s.height : 1
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1 && Math.abs(sx - 1) < 0.01 && Math.abs(sy - 1) < 0.01) continue
      const kf: Keyframe[] = [{ transformOrigin: 'top left', translate: dx + 'px ' + dy + 'px', scale: sx + ' ' + sy },
                              { transformOrigin: 'top left', translate: '0px 0px', scale: '1 1' }]
      el.animate(kf, { duration: 450, easing: FLIP_EASE })
    }
    settleSoon()
  })
}

function settleSoon() {
  clearTimeout(settleTimer)
  settleTimer = window.setTimeout(() => {
    settleTimer = 0
    settled.value = expanded.value // null on collapse — frosted surface off
  }, REDUCED_MOTION ? 0 : 480)
}

function onEnter(i: number) {
  if (!finePointer.matches) return
  clearTimers()
  if (expanded.value === i) return // already open
  if (expanded.value !== null) { flip(() => { expanded.value = i }); return } // direct switch
  enterTimer = window.setTimeout(() => { enterTimer = 0; flip(() => { expanded.value = i }) }, 250)
}
function onLeave(i: number) {
  if (!finePointer.matches) return
  if (expanded.value !== i) return
  clearTimers()
  exitTimer = window.setTimeout(() => { exitTimer = 0; flip(() => { expanded.value = null }) }, 350)
}

/* spotlight: no transforms — just CSS vars. --edge is the distance
   from the center (0 at center → 1 at the rim), boosted non-linearly
   (edge^2.2) so the rim of the card lights up sharply while the
   center stays a soft bloom. */
function onSpot(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  const rect = card.getBoundingClientRect()
  const x = (e.clientX - rect.left) / rect.width
  const y = (e.clientY - rect.top) / rect.height
  card.style.setProperty('--mx', (x * 100).toFixed(1) + '%')
  card.style.setProperty('--my', (y * 100).toFixed(1) + '%')
  const edge = 1 - 2 * Math.min(Math.min(x, 1 - x), Math.min(y, 1 - y))
  card.style.setProperty('--edge', Math.pow(Math.max(0, edge), 2.2).toFixed(3))
}

watch(expanded, (v) => {
  document.documentElement.classList.toggle('card-expanded', v !== null)
})
onBeforeUnmount(() => {
  clearTimers()
  document.documentElement.classList.remove('card-expanded')
})
</script>

<template>
  <section class="page" id="page3" :data-wheel-lock="expanded !== null ? '' : undefined">
    <div class="container">
      <h2>
        <svg class="icon"><use href="./assets/icons.svg#icon-cubes"></use></svg>
        {{ t('project-overview') }}
      </h2>

      <div class="grid" :class="{ 'has-expanded': expanded !== null }">
        <div v-for="(c, i) in cards" :key="c.key" class="card"
             :class="{ expanded: expanded === i, settled: settled === i }"
             @mouseenter="onEnter(i)" @mouseleave="onLeave(i)" @mousemove="onSpot">
          <span class="card-spot" aria-hidden="true"></span>
          <h3><svg class="icon" :class="{ 'pulse-shield': c.key === 'sec' }"><use :href="c.icon"></use></svg> {{ t(c.title) }}</h3>
          <div class="card-body"><component :is="c.comp" :expanded="expanded === i" /></div>
        </div>
      </div>

      <footer class="site-footer">
        <a :href="REPO_URL" target="_blank" rel="noopener">
          <svg class="icon"><use href="./assets/icons.svg#icon-github"></use></svg> Nekolio/DoLogger
        </a>
        <span>·</span>
        <span>{{ t('footer-license') }}</span>
        <span>·</span>
        <a href="mailto:nekoliowork+DoLogger@gmail.com">nekoliowork+DoLogger@gmail.com</a>
      </footer>
    </div>
  </section>
</template>
