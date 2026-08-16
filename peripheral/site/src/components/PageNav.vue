<script setup lang="ts">
/* PageNav — right-edge navigation. Dot + label are ONE unit: a pill whose
 * label is ALWAYS readable while the nav is awake — it never collapses to
 * hide its text; instead the pill's padding + font-size scale fluidly so
 * the label can't overflow or clip. A single absolutely-positioned
 * indicator pill glides (non-linear easing) to the active button to give
 * the highlight, replacing the old per-item opacity cross-fade. The whole
 * nav idles out (non-linear fade + slide) after 1.8s without interaction —
 * any mousemove / wheel / key / page change pokes it awake again; hovering
 * it keeps it awake. Skipped under reduced motion (always visible, static).
 *
 * While the pointer is over the page-2 terminal (whose lower area must
 * stay clean) the nav is suppressed entirely.
 *
 * All presentation lives in src/style.css (the `.page-nav` / nav section):
 * the component owns NO <style> block, so there is a single source of
 * truth for the pill unit, the always-readable label, and the sliding
 * indicator — no specificity arms race between a component style and the
 * global stylesheet.
 */
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps<{ count: number; active: number }>()
const emit = defineEmits<{ (e: 'go', i: number): void }>()
const { t } = useI18n()
const LABELS = ['nav-hero', 'nav-demo', 'nav-overview']

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const idle = ref(false)
const suppressed = ref(false)
const navEl = ref<HTMLElement | null>(null)
const indicatorEl = ref<HTMLElement | null>(null)
/* arms the glide transition only AFTER the first placement, so the
   indicator doesn't fly in from the top-left corner on first paint */
const indicatorReady = ref(false)
const IDLE_MS = 1800
let idleTimer = 0
let pokeRaf = 0

function poke() {
  if (REDUCED_MOTION) return
  idle.value = false
  clearTimeout(idleTimer)
  idleTimer = window.setTimeout(() => { idle.value = true }, IDLE_MS)
}
/* rAF-throttled mousemove poke + terminal suppression check */
function onMove(e: MouseEvent) {
  if (pokeRaf) return
  pokeRaf = requestAnimationFrame(() => {
    pokeRaf = 0
    const tgt = e.target instanceof Element ? e.target : null
    suppressed.value = !!tgt && !!tgt.closest('.ide-terminal')
    poke()
  })
}
function onWheel() { poke() }
function onKey() { poke() }
function navEnter() {
  if (REDUCED_MOTION) return
  idle.value = false
  clearTimeout(idleTimer)
}

/* Slide the indicator pill onto the active button. Measured relative to the
 * nav so the nav's own translateY(-50%) never skews the offset; a transform
 * glide keeps it cheap (no layout thrash). */
function syncIndicator() {
  const nav = navEl.value
  const ind = indicatorEl.value
  if (!nav || !ind) return
  const btn = nav.querySelectorAll<HTMLButtonElement>('button')[props.active]
  if (!btn) return
  const navRect = nav.getBoundingClientRect()
  const btnRect = btn.getBoundingClientRect()
  ind.style.width = `${btnRect.width}px`
  ind.style.height = `${btnRect.height}px`
  ind.style.transform = `translateY(${btnRect.top - navRect.top}px)`
}

watch(() => props.active, () => {
  poke()
  nextTick(syncIndicator)
})
onMounted(() => {
  window.addEventListener('mousemove', onMove)
  window.addEventListener('wheel', onWheel, { passive: true })
  window.addEventListener('keydown', onKey)
  window.addEventListener('resize', syncIndicator)
  poke()
  nextTick(() => {
    syncIndicator()
    requestAnimationFrame(() => { indicatorReady.value = true })
  })
  /* re-measure once webfonts settle — label width changes with the font */
  document.fonts.ready.then(syncIndicator)
})
onBeforeUnmount(() => {
  clearTimeout(idleTimer)
  cancelAnimationFrame(pokeRaf)
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('wheel', onWheel)
  window.removeEventListener('keydown', onKey)
  window.removeEventListener('resize', syncIndicator)
})
</script>

<template>
  <nav ref="navEl" class="page-nav" :class="{ idle, suppressed }" aria-label="sections" @mouseenter="navEnter" @mouseleave="poke">
    <span ref="indicatorEl" class="page-nav-indicator" :class="{ ready: indicatorReady }" aria-hidden="true"></span>
    <button v-for="i in count" :key="i"
            type="button"
            :class="{ active: active === i - 1 }"
            :aria-label="t(LABELS[i - 1])"
            :aria-current="active === i - 1 ? 'true' : undefined"
            @click="emit('go', i - 1)">
      <span class="dot"></span>
      <span class="label">{{ t(LABELS[i - 1]) }}</span>
    </button>
  </nav>
</template>
