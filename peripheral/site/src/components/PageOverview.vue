<script setup lang="ts">
/* PageOverview — page 3: the project-overview card wall.
 *
 * PC (fine pointers):
 *   - no click-to-expand and no gauge animations anymore. Every card
 *     always shows its full content; overflow loops inside the card
 *     (vertical loop-scroll for card bodies, a horizontal marquee for
 *     the architecture pipeline) — nothing truncates behind an
 *     ellipsis, nothing needs a click.
 *   - entering the page plays a "home-screen unlock" fly-in: each card
 *     flies in from off-screen in its own direction, big → small, with
 *     a springy non-linear settle and staggered, independent timings.
 *     The animation never fights the Steam-style tilt: WAAPI owns
 *     transform during the flight and hands it back to CSS on finish.
 *   - after settling, the 3D tilt + spotlight stay, strengthened at the
 *     corners and edges (still readable).
 *   - leaving the page plays no exit animation.
 *
 * Touch / narrow (mobile):
 *   - cards start as a collapsed title-only stack. Entering the page
 *     pushes them in one by one from the direction OPPOSITE to the
 *     swipe (Q-bounce, staggered).
 *   - the only interaction is tap-to-expand: a window of THREE cards
 *     fills the page (expanded + its two neighbours, edges padded),
 *     with non-linear FLIP animations as cards auto-open / 补位.
 *   - the expanded card's content loop-scrolls; hovering decelerates
 *     and pauses the loop; wheeling scrolls the card natively while it
 *     has room and falls through to the page nav at the edges (the
 *     dynamic hot-override lives in usePageNav's scroller walk).
 *   - if the browser supports it, the expanded card tilts with the
 *     device gyroscope, mirroring the PC 3D effect.
 */
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount, type Component } from 'vue'
import { useI18n } from 'vue-i18n'
import { useAutoLoopScroll } from '../composables/useAutoLoopScroll'
import PerformanceCard from './cards/PerformanceCard.vue'
import SecurityCard from './cards/SecurityCard.vue'
import SinksCard from './cards/SinksCard.vue'
import ArchitectureCard from './cards/ArchitectureCard.vue'
import ReleasesCard from './cards/ReleasesCard.vue'
import CommunityCard from './cards/CommunityCard.vue'

const props = defineProps<{ activePage: number; lastDir?: 1 | -1 | 0 }>()

const { t } = useI18n()
const REPO_URL = 'https://github.com/Nekolio/DoLogger'

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')
/* mobile layout follows the input modality (touch / emulated touch) —
   a narrow desktop window with a mouse keeps the PC grid + fly-in */
const isTouch = computed(() => !finePointer.matches)

const cards: { key: string; icon: string; title: string; comp: Component }[] = [
  { key: 'perf', icon: './assets/icons.svg#icon-gauge', title: 'card-perf', comp: PerformanceCard },
  { key: 'sec', icon: './assets/icons.svg#icon-shield', title: 'card-sec', comp: SecurityCard },
  { key: 'sinks', icon: './assets/icons.svg#icon-plug', title: 'card-sinks', comp: SinksCard },
  { key: 'arch', icon: './assets/icons.svg#icon-branch', title: 'card-arch', comp: ArchitectureCard },
  { key: 'rel', icon: './assets/icons.svg#icon-tag', title: 'card-changelog', comp: ReleasesCard },
  { key: 'comm', icon: './assets/icons.svg#icon-users', title: 'card-comm', comp: CommunityCard }
]

/* ── PC fly-in: home-screen-unlock entrance, once per entry ──────────
   Each card flies from its own off-screen direction, overshoots a
   little and settles with independent timings. fill:'backwards' keeps
   it hidden until its delay; on finish the animation is cancelled so
   the CSS tilt transform takes over untouched. */
const DIRS: [number, number][] = [
  [-1, -1.1], [0, -1.5], [1, -1.1],
  [-1, 0.4], [1, 0.4], [-0.6, 1.4]
]
function gridCards(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('#page3 .card'))
}
function runFlyIn() {
  if (REDUCED_MOTION || isTouch.value) return
  const cardsEl = gridCards()
  const dist = Math.max(window.innerWidth, window.innerHeight)
  cardsEl.forEach((card, i) => {
    const d = DIRS[i % DIRS.length]
    const dx = d[0] * dist * (0.55 + 0.12 * (i % 3))
    const dy = d[1] * dist * (0.42 + 0.1 * (i % 2))
    const delay = 60 + i * 130
    const dur = 760 + (i % 3) * 120
    const anim = card.animate([
      { transform: `translate(${dx}px, ${dy}px) scale(2.1)`, opacity: 0 },
      { transform: 'translate(0, 0) scale(0.93)', opacity: 1, offset: 0.6 },
      { transform: 'scale(1.045)', opacity: 1, offset: 0.8 },
      { transform: 'scale(1)', opacity: 1, offset: 1 }
    ], { duration: dur, delay, easing: 'cubic-bezier(0.25, 0.82, 0.32, 1)', fill: 'backwards' })
    anim.onfinish = () => anim.cancel() // hand transform back to the CSS tilt
  })
}

/* ── mobile: collapsed stack + window-of-3 expand ──────────────────── */
const expanded = ref<number | null>(null)
const entryKey = ref(0)
const enterDy = ref(0)

interface MobileCard { key: string; icon: string; title: string; comp: Component; i: number; state: 'open' | 'closed' }
const mobileCards = computed<MobileCard[]>(() => {
  if (expanded.value == null) {
    return cards.map((c, i) => ({ ...c, i, state: 'closed' as const }))
  }
  const n = cards.length
  let a = expanded.value, b = expanded.value
  while (b - a + 1 < 3) {
    if (a > 0) a--
    else if (b < n - 1) b++
    else break
  }
  const out: MobileCard[] = []
  for (let k = a; k <= b; k++) {
    out.push({ ...cards[k], i: k, state: k === expanded.value ? 'open' : 'closed' })
  }
  return out
})

/* iOS 13+ requires an explicit permission request from a user gesture —
   the tap that expands a card is exactly that gesture. */
function enableGyroIfAvailable() {
  if (typeof window.DeviceOrientationEvent === 'undefined') return
  const DOE = window.DeviceOrientationEvent as unknown as { requestPermission?: () => Promise<string> }
  if (typeof DOE.requestPermission === 'function') {
    DOE.requestPermission().then(p => { if (p === 'granted') attachGyro() }).catch(() => { /* denied */ })
  } else {
    attachGyro()
  }
}
let gyroHandler: ((e: DeviceOrientationEvent) => void) | null = null
let gyroCard: HTMLElement | null = null
function attachGyro() {
  detachGyro()
  gyroCard = document.querySelector<HTMLElement>('#page3 .mcard.open')
  if (!gyroCard) return
  const clamp = (v: number, m: number) => Math.max(-m, Math.min(m, v))
  gyroHandler = (e: DeviceOrientationEvent) => {
    const b = e.beta ?? 0
    const g = e.gamma ?? 0
    /* beta ~45° is "upright"; map pitch and roll into a ±10° tilt */
    gyroCard?.style.setProperty('--tilt-x', clamp((b - 45) * 0.22, 10).toFixed(2) + 'deg')
    gyroCard?.style.setProperty('--tilt-y', clamp(g * 0.3, 10).toFixed(2) + 'deg')
  }
  window.addEventListener('deviceorientation', gyroHandler)
}
function detachGyro() {
  if (gyroHandler) window.removeEventListener('deviceorientation', gyroHandler)
  gyroHandler = null
  gyroCard = null
}

/* non-linear FLIP for the window changing: existing cards animate
   height + position from their old rects to the new ones; entering
   cards are covered by the TransitionGroup enter animation. */
let flipAnim: Animation | null = null
async function flipMobile(next: number | null) {
  /* A template ref on <TransitionGroup> resolves to the component instance,
     not its root DOM element, so query the element directly. */
  const listEl = document.querySelector<HTMLElement>('#page3 .mob-stack')
  if (!listEl || REDUCED_MOTION) { expanded.value = next; return }
  const items = Array.from(listEl.querySelectorAll<HTMLElement>('.mcard'))
  const before = new Map(items.map(el => [el.dataset.i, el.getBoundingClientRect()]))
  expanded.value = next
  await nextTick()
  /* the cards that stay in the window are the only ones the FLIP
     animates — cards leaving the window play their own leave
     transition instead (no competing animations) */
  const kept = new Set(mobileCards.value.map(m => String(m.i)))
  const afterItems = Array.from(listEl.querySelectorAll<HTMLElement>('.mcard'))
  for (const el of afterItems) {
    const key = el.dataset.i || ''
    if (!kept.has(key)) continue
    const old = before.get(key)
    if (!old) continue // newly entered — its own enter animation plays
    const now = el.getBoundingClientRect()
    const dx = old.left - now.left
    const dy = old.top - now.top
    const dh = old.height - now.height
    const dw = old.width - now.width
    if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5 && Math.abs(dh) < 0.5 && Math.abs(dw) < 0.5) continue
    flipAnim?.cancel()
    const anim = el.animate([
      { height: old.height + 'px', transform: `translate(${dx}px, ${dy}px)` },
      { height: now.height + 'px', transform: 'translate(0, 0)' }
    ], { duration: 540, easing: 'cubic-bezier(0.34, 1.56, 0.64, 1)', fill: 'both' })
    flipAnim = anim
    anim.onfinish = () => { anim.cancel(); flipAnim = null }
  }
  syncLoops()
}

async function toggleMobile(i: number) {
  if (expanded.value === i) {
    detachGyro()
    await flipMobile(null)
  } else {
    /* gyro attaches only after the FLIP renders the open card */
    await flipMobile(i)
    enableGyroIfAvailable()
  }
}

/* ── loop-scroll: every overflowing card body + the arch marquee ───── */
const loops = useAutoLoopScroll()
function syncLoops() {
  nextTick(() => {
    loops.attachAll('#page3 .card-body', '#page3 .pipe-marquee')
  })
}

/* entering page 3: PC fly-in / mobile push-in */
watch(() => props.activePage, (v) => {
  if (v !== 2) { detachGyro(); return }
  if (isTouch.value) {
    expanded.value = null
    detachGyro()
    enterDy.value = props.lastDir === 1 ? -150 : props.lastDir === -1 ? 150 : 0
    entryKey.value++
    syncLoops()
  } else {
    runFlyIn()
  }
}, { immediate: true })

watch(isTouch, () => { syncLoops() })
watch(mobileCards, () => { syncLoops() })

/* ── spotlight + Steam-style tilt (PC; expanded mobile card keeps a
   gyro-driven tilt instead). --edge is the distance from the center
   (0 center → 1 rim), boosted non-linearly so the corners/edges light
   up while the center stays a soft bloom. Tilt clamped to ±8°. */
function onSpot(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  const rect = card.getBoundingClientRect()
  const x = (e.clientX - rect.left) / rect.width
  const y = (e.clientY - rect.top) / rect.height
  card.style.setProperty('--mx', (x * 100).toFixed(1) + '%')
  card.style.setProperty('--my', (y * 100).toFixed(1) + '%')
  const edge = 1 - 2 * Math.min(Math.min(x, 1 - x), Math.min(y, 1 - y))
  card.style.setProperty('--edge', Math.pow(Math.max(0, edge), 2.6).toFixed(3))
  if (!finePointer.matches) return
  card.style.setProperty('--tilt-x', ((y - 0.5) * 16).toFixed(2) + 'deg')
  card.style.setProperty('--tilt-y', ((0.5 - x) * 16).toFixed(2) + 'deg')
}
function onLeave(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  card.style.setProperty('--tilt-x', '0deg')
  card.style.setProperty('--tilt-y', '0deg')
}

onMounted(syncLoops)
onBeforeUnmount(() => {
  detachGyro()
  loops.detachAll()
  flipAnim?.cancel()
})
</script>

<template>
  <section class="page" id="page3">
    <div class="container">
      <h2>
        <svg class="icon"><use href="./assets/icons.svg#icon-cubes"></use></svg>
        {{ t('project-overview') }}
      </h2>

      <!-- PC: the card wall. No click interaction — content loops inside. -->
      <div v-if="!isTouch" class="grid">
        <div v-for="(c, i) in cards" :key="c.key" class="card"
             :style="{ '--i': i }" @mousemove="onSpot" @mouseleave="onLeave">
          <span class="card-spot" aria-hidden="true"></span>
          <h3><svg class="icon" :class="{ 'pulse-shield': c.key === 'sec' }"><use :href="c.icon"></use></svg> {{ t(c.title) }}</h3>
          <div class="card-body"><component :is="c.comp" /></div>
        </div>
      </div>

      <!-- Mobile: collapsed title stack; tap expands a window of three
           (expanded card + its two neighbours), FLIP-animated. -->
      <TransitionGroup v-else name="mcard" tag="div" class="mob-stack"
                       :key="'mob-' + entryKey"
                       :style="{ '--enter-dy': enterDy + 'px' }">
        <div v-for="m in mobileCards" :key="m.key" class="mcard" :class="m.state"
             :data-i="m.i" :style="{ '--delay': m.i * 80 + 'ms' }">
          <div class="mcard-title"
               role="button" tabindex="0"
               :aria-expanded="m.state === 'open' ? 'true' : 'false'"
               :aria-label="t(m.title)"
               @click="toggleMobile(m.i)"
               @keydown.enter.prevent="toggleMobile(m.i)"
               @keydown.space.prevent="toggleMobile(m.i)">
            <svg class="icon" :class="{ 'pulse-shield': m.key === 'sec' }"><use :href="m.icon"></use></svg>
            <span>{{ t(m.title) }}</span>
            <svg class="icon mcard-chev" :class="{ open: m.state === 'open' }"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
          </div>
          <div class="card-body"><component :is="m.comp" /></div>
        </div>
      </TransitionGroup>

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

<style scoped>
/* the header is the interactive element (tap to expand / collapse); the
   body stays a plain scroll surface */
.mcard-title { cursor: pointer; }
.mcard .card-body { cursor: auto; }
</style>
