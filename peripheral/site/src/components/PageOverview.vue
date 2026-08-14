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

/* ── game-inspect expand ─────────────────────────────────────────────
 * Click expands: the card leaves its grid slot (position: fixed, with
 * `.grid { perspective }` as containing block) and flies to the viewport
 * center — height ≈ 85svh, a Z-axis push (translateZ 160px), shadow
 * deepening, all WAAPI over individual translate/scale properties so the
 * hover tilt (transform) composes freely. Neighbors 补位推进: leaving
 * flow lets the grid reflow them into the vacated cells, FLIP-animated
 * with a non-linear ease; they dim slightly (brightness) but are never
 * hidden. Depth-of-field: a --dof var ramps 0→1 during the fly-out,
 * driving the backdrop's blur + darken on .grid::after. Click again,
 * outside-mousedown, Esc or × collapse; the wheel is hard-locked on
 * #page3 while expanded. Reduced-motion: instant jump, no flight. */

const expanded = ref<number | null>(null)
const settled = ref<number | null>(null)

const EASE_POP = 'cubic-bezier(0.34, 1.56, 0.64, 1)'
const EASE_OUT = 'cubic-bezier(0.22, 1, 0.36, 1)'
const DUR = 550
const BASE_SHADOW = '0 4px 15px rgba(0, 0, 0, 0.1)'

function gridEl(): HTMLElement {
  return document.querySelector<HTMLElement>('#page3 .grid')!
}
function gridCards(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('#page3 .card'))
}
/* the expanded card's end-state shadow — theme-varred (light theme gets
   a gentler drop), WAAPI needs concrete values so we resolve the vars */
function focusShadow(): string {
  const main = getComputedStyle(document.documentElement).getPropertyValue('--focus-shadow-main').trim()
    || '0 24px 70px rgba(0, 0, 0, 0.45)'
  const glow = getComputedStyle(document.documentElement).getPropertyValue('--card-glow').trim()
    || 'rgba(127, 213, 255, 0.1)'
  return main + ', 0 0 30px ' + glow
}

function setOverlay(el: HTMLElement, r: DOMRect) {
  const g = gridEl().getBoundingClientRect()
  el.style.position = 'fixed'
  el.style.left = (r.left - g.left) + 'px'
  el.style.top = (r.top - g.top) + 'px'
  el.style.width = r.width + 'px'
  el.style.height = r.height + 'px'
}
function clearOverlay(el: HTMLElement) {
  el.style.position = ''
  el.style.left = ''
  el.style.top = ''
  el.style.width = ''
  el.style.height = ''
  el.style.translate = ''
  el.style.scale = ''
}

/** Overlay flight from `from` to `to` (viewport rects). `persist` keeps
 *  the fixed geometry + final transform (the inspected card); otherwise
 *  the element returns to flow at the end (fly-back / collapse). */
function flyTo(el: HTMLElement, from: DOMRect,
               to: { left: number; top: number; width: number; height: number },
               ease: string, persist: boolean, zEnd: number, onfinish?: () => void) {
  setOverlay(el, from)
  const dx = to.left - from.left
  const dy = to.top - from.top
  const sx = to.width > 0 ? to.width / from.width : 1
  const sy = to.height > 0 ? to.height / from.height : 1
  const anim = el.animate([
    { transformOrigin: 'top left', translate: '0px 0px 0px', scale: '1 1', boxShadow: BASE_SHADOW },
    { transformOrigin: 'top left', translate: dx + 'px ' + dy + 'px ' + zEnd + 'px', scale: sx + ' ' + sy, boxShadow: focusShadow() }
  ], { duration: DUR, easing: ease, fill: 'forwards' })
  anim.onfinish = () => {
    if (persist) {
      el.style.translate = dx + 'px ' + dy + 'px ' + zEnd + 'px'
      el.style.scale = sx + ' ' + sy
    } else {
      clearOverlay(el)
    }
    anim.cancel()
    onfinish?.()
  }
}

/** small translate FLIP for the neighbors reflowing 补位 into the
 *  vacated cells — translate-only (their size doesn't change). */
function flipRect(el: HTMLElement, from: DOMRect, to: DOMRect) {
  const dx = from.left - to.left
  const dy = from.top - to.top
  if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) return
  el.animate([
    { translate: dx + 'px ' + dy + 'px' },
    { translate: '0px 0px' }
  ], { duration: DUR, easing: EASE_POP, fill: 'both' })
}

function targetRect(g: DOMRect) {
  const width = Math.min(g.width * 0.9, 980)
  const height = Math.min(window.innerHeight * 0.85, 920)
  return {
    left: (window.innerWidth - width) / 2,
    top: (window.innerHeight - height) / 2,
    width,
    height
  }
}

/* ── depth-of-field backdrop ramp (--dof on .grid drives .grid::after) */
let dofRaf = 0
function setDof(v: number) {
  gridEl().style.setProperty('--dof', String(v))
}
function dofRamp(target: number) {
  if (dofRaf) { cancelAnimationFrame(dofRaf); dofRaf = 0 }
  if (REDUCED_MOTION) { setDof(target); return }
  const grid = gridEl()
  const from = parseFloat(grid.style.getPropertyValue('--dof') || '0')
  const t0 = performance.now()
  const step = (now: number) => {
    const p = Math.min(1, (now - t0) / DUR)
    const eased = 1 - Math.pow(1 - p, 3)
    grid.style.setProperty('--dof', (from + (target - from) * eased).toFixed(3))
    if (p < 1) dofRaf = requestAnimationFrame(step)
    else dofRaf = 0
  }
  dofRaf = requestAnimationFrame(step)
}

function expand(i: number) {
  const cardsEl = gridCards()
  const before = cardsEl.map(c => c.getBoundingClientRect())
  const old = expanded.value
  settled.value = null
  expanded.value = i

  if (REDUCED_MOTION) {
    const el = cardsEl[i]
    const g = gridEl().getBoundingClientRect()
    const target = targetRect(g)
    setOverlay(el, target)
    if (old !== null && old !== i) clearOverlay(cardsEl[old])
    settled.value = i
    setDof(1)
    return
  }

  nextTick(() => {
    /* final grid configuration: the new card out of flow, the old one
       back in its slot — then measure where everything landed */
    setOverlay(cardsEl[i], before[i])
    if (old !== null && old !== i) clearOverlay(cardsEl[old])
    const after = cardsEl.map(c => c.getBoundingClientRect())

    for (let k = 0; k < cardsEl.length; k++) {
      if (k === i) continue
      if (k === old) flyTo(cardsEl[k], before[k], after[k], EASE_OUT, false, 0) // flies back with scale
      else flipRect(cardsEl[k], before[k], after[k]) // reflowed neighbor
    }
    const g = gridEl().getBoundingClientRect()
    flyTo(cardsEl[i], before[i], targetRect(g), EASE_POP, true, 160, () => { settled.value = i })
    dofRamp(1)
  })
}

function collapse(i: number) {
  const cardsEl = gridCards()
  const el = cardsEl[i]
  settled.value = null
  if (REDUCED_MOTION) {
    clearOverlay(el)
    expanded.value = null
    setDof(0)
    return
  }
  const pulled = cardsEl.map(c => c.getBoundingClientRect())
  const overlay = pulled[i]
  /* temporarily return to flow to measure the landing slot, then put
     the overlay geometry back — all pre-paint, no flicker */
  clearOverlay(el)
  const land = el.getBoundingClientRect()
  setOverlay(el, overlay)
  flyTo(el, overlay, land, EASE_OUT, false, 0, () => {
    expanded.value = null
    nextTick(() => {
      const restored = cardsEl.map(c => c.getBoundingClientRect())
      for (let k = 0; k < cardsEl.length; k++) {
        if (k === i) continue
        flipRect(cardsEl[k], pulled[k], restored[k]) // 补位 flows back
      }
    })
  })
  dofRamp(0)
}

function onCardClick(i: number, e: MouseEvent) {
  if (e.target instanceof Element && e.target.closest('a, button')) return // links/close keep their own behavior
  if (expanded.value === i) collapse(i)
  else expand(i)
}
function onClose() {
  if (expanded.value !== null) collapse(expanded.value)
}

/* ── collapse on outside mousedown / Esc while expanded ───────────── */
function onDocDown(e: MouseEvent) {
  if (expanded.value === null) return
  const tgt = e.target as Node | null
  if (tgt instanceof Element && tgt.closest('#page3 .card')) return
  collapse(expanded.value)
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && expanded.value !== null) collapse(expanded.value)
}

watch(expanded, (v, oldV) => {
  document.documentElement.classList.toggle('card-expanded', v !== null)
  if (v !== null && oldV === null) {
    document.addEventListener('mousedown', onDocDown)
    document.addEventListener('keydown', onKey)
  } else if (v === null && oldV !== null) {
    document.removeEventListener('mousedown', onDocDown)
    document.removeEventListener('keydown', onKey)
  }
})

/* ── spotlight + Steam-style tilt (hover only; expanded freezes tilt) ─
   --edge is the distance from the center (0 center → 1 rim), boosted
   non-linearly (edge^2.2) so the corners/edges light up while the
   center stays a soft bloom. Tilt is clamped to ±6°, inverted Y. */
function onSpot(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  const rect = card.getBoundingClientRect()
  const x = (e.clientX - rect.left) / rect.width
  const y = (e.clientY - rect.top) / rect.height
  card.style.setProperty('--mx', (x * 100).toFixed(1) + '%')
  card.style.setProperty('--my', (y * 100).toFixed(1) + '%')
  const edge = 1 - 2 * Math.min(Math.min(x, 1 - x), Math.min(y, 1 - y))
  card.style.setProperty('--edge', Math.pow(Math.max(0, edge), 2.2).toFixed(3))
  if (!finePointer.matches) return
  card.style.setProperty('--tilt-x', ((y - 0.5) * 12).toFixed(2) + 'deg')
  card.style.setProperty('--tilt-y', ((0.5 - x) * 12).toFixed(2) + 'deg')
}
function onLeave(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  card.style.setProperty('--tilt-x', '0deg')
  card.style.setProperty('--tilt-y', '0deg')
}

onBeforeUnmount(() => {
  if (dofRaf) cancelAnimationFrame(dofRaf)
  document.removeEventListener('mousedown', onDocDown)
  document.removeEventListener('keydown', onKey)
  document.documentElement.classList.remove('card-expanded')
})
</script>

<template>
  <section class="page" id="page3" :data-wheel-lock="expanded !== null ? '' : undefined"
           :data-wheel-lock-hard="expanded !== null ? '' : undefined">
    <div class="container">
      <h2>
        <svg class="icon"><use href="./assets/icons.svg#icon-cubes"></use></svg>
        {{ t('project-overview') }}
      </h2>

      <div class="grid" :class="{ 'has-expanded': expanded !== null }">
        <div v-for="(c, i) in cards" :key="c.key" class="card"
             :class="{ expanded: expanded === i, settled: settled === i }"
             @click="onCardClick(i, $event)" @mousemove="onSpot" @mouseleave="onLeave">
          <span class="card-spot" aria-hidden="true"></span>
          <button v-if="expanded === i" type="button" class="card-close" :aria-label="t('card-close')"
                  @click.stop="onClose">×</button>
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
