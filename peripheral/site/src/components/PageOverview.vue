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
   Each card flies in from the SCREEN EDGE AT ITS OWN GRID POSITION:
   a card in the left column enters from the left edge, a top-row card
   from the top, a right-bottom card from the lower-right, etc. With a
   3-column grid (2 rows now, 3 with the reserved slots), the middle
   column cards drop from the top-center (分左中右 keeps them spread),
   and the middle row (a 9-card wall) falls from the top like a drop.
   fill:'backwards' keeps it hidden until its delay; on finish the
   animation is cancelled so the CSS tilt transform takes over. */
function gridCards(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('#page3 .card'))
}
function runFlyIn() {
  if (REDUCED_MOTION || isTouch.value) return
  const cardsEl = gridCards()
  const dist = Math.max(window.innerWidth, window.innerHeight)
  const COLS = 3
  cardsEl.forEach((card, i) => {
    const col = i % COLS                       // 0 left · 1 middle · 2 right
    const row = Math.floor(i / COLS)           // 0 top · 1 middle · 2 bottom
    const rows = Math.max(1, Math.ceil(cardsEl.length / COLS))
    /* direction from the card's own edge: left col ← left, right col ←
       right, middle col ← top (falls like a drop). Vertical origin
       follows the row: top row from above, bottom row from below, the
       middle row (a 9-card wall) drops from the top-center. */
    const dxSign = col === 0 ? -1 : col === 2 ? 1 : 0
    const dySign = row === 0 ? -1 : row === rows - 1 ? 1 : -1
    const dx = dxSign * dist * (0.6 + 0.1 * (col === 1 ? row : col))
    const dy = dySign * dist * (0.45 + 0.08 * row)
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
/* 'page' = staggered push-in on page entry; 'reflow' = the quick settle
   used when cards re-enter during expand/collapse (no stagger). */
const enterMode = ref<'page' | 'reflow'>('page')

interface MobileCard { key: string; icon: string; title: string; comp: Component; i: number; state: 'open' | 'closed' }
const mobileCards = computed<MobileCard[]>(() => {
  if (expanded.value == null) {
    return cards.map((c, i) => ({ ...c, i, state: 'closed' as const }))
  }
  const n = cards.length
  /* symmetric window: expand UP and DOWN alternately so the opened card
     stays CENTERED (one neighbour above, one below) — e.g. expanding
     Sinks (index 2) shows [1,2,3] = Security | Sinks | Architecture.
     Edge cards can't center; then the window grows toward the available
     side only. */
  let a = expanded.value, b = expanded.value
  let up = true // expand upward first, then alternate
  while (b - a + 1 < 3) {
    if (up && a > 0) { a--; up = false }
    else if (!up && b < n - 1) { b++; up = true }
    else if (a > 0) { a-- }          // up blocked → grow down first
    else if (b < n - 1) { b++ }      // down blocked → grow up
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
let gyroBase: { b: number; g: number } | null = null // pose when the card opened
function attachGyro() {
  detachGyro()
  gyroCard = document.querySelector<HTMLElement>('#page3 .mcard.open')
  if (!gyroCard) return
  const clamp = (v: number, m: number) => Math.max(-m, Math.min(m, v))
  /* The tilt is relative to the pose AT OPEN TIME, not an absolute
     "upright" reference: whatever the device orientation is the moment
     the card expands becomes the flat baseline, and the card only tilts
     as the phone moves away from that. A phone lying flat (beta≈90°)
     when tapped therefore shows a level card, not an up-small/down-big
     wedge. The first deviceorientation event right after the gesture
     is the baseline. */
  gyroBase = null
  gyroHandler = (e: DeviceOrientationEvent) => {
    const b = e.beta ?? 0
    const g = e.gamma ?? 0
    if (!gyroBase) { gyroBase = { b, g }; return } // capture the open-pose baseline
    const db = b - gyroBase.b   // pitch delta from the baseline
    const dg = g - gyroBase.g   // roll delta from the baseline
    /* gamma (+dg, rotateY) is user-confirmed correct (tilt right with the
       phone). beta drives rotateX — its sign is INVERTED so the card leans
       the same way the phone pitches (real-device check: pitch down ->
       card top recedes). */
    gyroCard?.style.setProperty('--tilt-x', clamp(-db * 0.22, 10).toFixed(2) + 'deg')
    gyroCard?.style.setProperty('--tilt-y', clamp(dg * 0.3, 10).toFixed(2) + 'deg')
  }
  window.addEventListener('deviceorientation', gyroHandler)
}
function detachGyro() {
  if (gyroHandler) window.removeEventListener('deviceorientation', gyroHandler)
  gyroHandler = null
  gyroBase = null
  /* collapsing leaves no residual tilt on the card */
  if (gyroCard) {
    gyroCard.style.removeProperty('--tilt-x')
    gyroCard.style.removeProperty('--tilt-y')
  }
  gyroCard = null
}

/* non-linear FLIP for the window changing. `.mcard` is flex-sized
   (flex-basis / flex-grow drive the main size), so a WAAPI `height`
   keyframe is ignored by flex layout — the height used to snap instantly
   while only the translate moved. The fix is a transform-only FLIP
   (translate + scale from the old rect to the new rect), which flex cannot
   override, for every KEPT card; cards that ENTER the window pop in via
   WAAPI too, so no card ever snaps into place. */
const flipAnims = new Set<Animation>()
function trackFlip(el: HTMLElement, anim: Animation) {
  flipAnims.add(anim)
  const cleanup = () => { el.style.transformOrigin = ''; flipAnims.delete(anim) }
  anim.onfinish = () => { anim.cancel(); cleanup() }
  anim.oncancel = () => { cleanup() }
}
async function flipMobile(next: number | null) {
  /* expand/collapse re-entries use the quick settle, not the staggered
     page-entry push-in */
  enterMode.value = 'reflow'
  /* A template ref on <TransitionGroup> resolves to the component instance,
     not its root DOM element, so query the element directly. */
  const listEl = document.querySelector<HTMLElement>('#page3 .mob-stack')
  if (!listEl || REDUCED_MOTION) { expanded.value = next; return }
  /* capture every CURRENT card's rect before the reactive reflow */
  const before = new Map<string, DOMRect>()
  listEl.querySelectorAll<HTMLElement>('.mcard').forEach(el => {
    before.set(el.dataset.i || '', el.getBoundingClientRect())
  })
  const collapsing = next === null
  expanded.value = next
  await nextTick()
  /* cancel any FLIP still in flight so two animations never stack */
  flipAnims.forEach(a => a.cancel())
  flipAnims.clear()
  const order = mobileCards.value.map(m => String(m.i))
  const nowEls = new Map<string, HTMLElement>()
  listEl.querySelectorAll<HTMLElement>('.mcard').forEach(el => {
    nowEls.set(el.dataset.i || '', el)
  })
  /* opening pops with a springy overshoot; collapsing settles softer */
  const pop = 'cubic-bezier(0.34, 1.56, 0.64, 1)'
  const soft = 'cubic-bezier(0.22, 1, 0.36, 1)'
  for (const key of order) {
    const el = nowEls.get(key)
    if (!el) continue
    const now = el.getBoundingClientRect()
    const old = before.get(key)
    if (old) {
      /* kept card — TRANSLATE-ONLY FLIP. Scaling the whole card would
         stretch its title (the user asked for no title deformation), so
         the card glides from its old position to the new one while the
         height change is handled by the layout reflow underneath. */
      const dx = old.left - now.left
      const dy = old.top - now.top
      if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) continue
      const anim = el.animate([
        { transform: `translate(${dx}px, ${dy}px)` },
        { transform: 'translate(0px, 0px)' }
      ], {
        duration: collapsing ? 420 : 540,
        easing: collapsing ? soft : pop,
        fill: 'both'
      })
      trackFlip(el, anim)
    } else {
      /* newly entering card (a neighbour joining the window, or a card
         returning on collapse) — springy pop-in instead of snapping */
      const fromY = collapsing ? -26 : 26
      const anim = el.animate([
        { opacity: 0, transform: `translateY(${fromY}px) scale(0.96)` },
        { opacity: 1, transform: 'translateY(0px) scale(1)' }
      ], {
        duration: collapsing ? 380 : 460,
        easing: collapsing ? soft : pop,
        fill: 'both'
      })
      trackFlip(el, anim)
    }
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

/* ── loop-scroll: every overflowing card body (vertical ping-pong).
      The architecture pipeline is NOT looped: its chain flex-wraps to
      the card width, so there is no horizontal marquee to drive. ───── */
const loops = useAutoLoopScroll()
function syncLoops() {
  nextTick(() => {
    loops.attachAll('#page3 .card-body', '')
  })
}

/* entering page 3: PC fly-in / mobile push-in */
watch(() => props.activePage, (v) => {
  if (v !== 2) { detachGyro(); return }
  if (isTouch.value) {
    expanded.value = null
    detachGyro()
    enterMode.value = 'page'
    /* push in from the direction OPPOSITE to the swipe: a swipe up
       (lastDir 1) enters page 3, so the cards rise from below */
    enterDy.value = props.lastDir === -1 ? -150 : 150
    entryKey.value++
    syncLoops()
  } else {
    runFlyIn()
    syncLoops() // PC: (re)attach the loops on every entry — idempotent
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
/* PC hover state — drives the fade-mask removal (.card.hovered). The
   loop's own hovered() check (whole-card :hover) handles the pause, so
   this class only toggles the mask; the two stay in sync because both
   fire on the same card enter/leave. */
const hoveredKeys = ref(new Set<string>())
function onCardEnter(c: { key: string }) {
  hoveredKeys.value.add(c.key)
}
function onCardLeave(c: { key: string }, e: MouseEvent) {
  hoveredKeys.value.delete(c.key)
  const card = e.currentTarget as HTMLElement
  card.style.setProperty('--tilt-x', '0deg')
  card.style.setProperty('--tilt-y', '0deg')
}

onMounted(syncLoops)
onBeforeUnmount(() => {
  detachGyro()
  loops.detachAll()
  flipAnims.forEach(a => a.cancel())
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
             :class="{ hovered: hoveredKeys.has(c.key) }"
             data-wheel-lock-hard
             :style="{ '--i': i }"
             @mouseenter="onCardEnter(c)"
             @mousemove="onSpot"
             @mouseleave="onCardLeave(c, $event)">
          <span class="card-spot" aria-hidden="true"></span>
          <h3><svg class="icon" :class="{ 'pulse-shield': c.key === 'sec' }"><use :href="c.icon"></use></svg> {{ t(c.title) }}</h3>
          <div class="card-body"><component :is="c.comp" /></div>
        </div>
      </div>

      <!-- Mobile: collapsed title stack; tap expands a window of three
           (expanded card + its two neighbours), FLIP-animated. -->
      <TransitionGroup v-else name="mcard" tag="div" class="mob-stack"
                       :class="{ reflow: enterMode === 'reflow', 'has-open': expanded !== null }"
                       :key="'mob-' + entryKey"
                       :style="{ '--enter-dy': (enterMode === 'reflow' ? 18 : enterDy) + 'px' }">
        <div v-for="m in mobileCards" :key="m.key" class="mcard" :class="m.state"
             :data-i="m.i" :style="{ '--delay': (enterMode === 'page' ? m.i * 80 : 0) + 'ms' }"
             :data-wheel-lock-hard="m.state === 'open' ? '' : null">
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

/* Expand/collapse (reflow): entering cards are animated by WAAPI in
   flipMobile, so the TransitionGroup's CSS enter keyframe is disabled —
   otherwise two drivers fight over transform/opacity. Leaving cards keep
   style.css's fade-out (and its position:absolute removal choreography). */
.mob-stack.reflow .mcard-enter-active {
  animation: none;
}

/* PC hover: drop the symmetric edge fade so the end content (e.g. the
   "每次发布的实测数据 →" release link) is fully readable. Not-hovered keeps
   style.css's mask for the loop-scroll aesthetic; the loop already pauses
   on whole-card hover (useAutoLoopScroll::hovered). Higher specificity
   (#id + .class + .class) overrides style.css's `.card-body` mask without
   editing that file. */
#page3 .card.hovered .card-body {
  mask-image: none;
  -webkit-mask-image: none;
}
</style>
