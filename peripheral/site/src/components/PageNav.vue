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

<style>
/* ================================================================
   PageNav — component-owned (non-scoped) styles. The `nav.page-nav …`
   selectors carry one extra type selector so they beat the legacy
   style.css nav rules regardless of stylesheet order.
   ================================================================ */

/* single sliding indicator pill — glides + resizes to the active button */
.page-nav-indicator {
  position: absolute;
  top: 0;
  right: 0;
  z-index: 0;
  border-radius: 999px;
  border: 1px solid var(--accent);
  background: rgba(127, 213, 255, 0.08);
  box-shadow: 0 0 14px var(--accent-glow);
  pointer-events: none;
  transition: none;
  will-change: transform;
}
.page-nav-indicator.ready {
  transition: transform 0.55s cubic-bezier(0.22, 1, 0.36, 1),
              width 0.55s cubic-bezier(0.22, 1, 0.36, 1),
              height 0.55s cubic-bezier(0.22, 1, 0.36, 1);
}

/* the pill unit — dot + label as one. Fluid padding so the label never
   overflows; the button stays transparent and lets the indicator highlight
   the active one (no per-item background). */
nav.page-nav button {
  display: flex;
  align-items: center;
  gap: clamp(0.28rem, 0.5vw + 0.1rem, 0.5rem);
  padding: clamp(0.18rem, 0.4vw + 0.08rem, 0.32rem)
           clamp(0.4rem, 0.9vw + 0.1rem, 0.7rem)
           clamp(0.18rem, 0.4vw + 0.08rem, 0.32rem)
           clamp(0.16rem, 0.4vw + 0.04rem, 0.35rem);
  border: 1px solid transparent;
  border-radius: 999px;
  background: transparent;
  cursor: pointer;
  position: relative;
  z-index: 1;
  transition: background 0.45s cubic-bezier(0.22, 1, 0.36, 1),
              border-color 0.45s cubic-bezier(0.22, 1, 0.36, 1);
}
nav.page-nav button:hover {
  background: rgba(127, 213, 255, 0.06);
  border-color: var(--border);
}
nav.page-nav button.active {
  background: transparent;
  border-color: transparent;
  box-shadow: none;
}

/* label is always readable while awake: fluid font-size, no max-width
   collapse, no overflow clip, no per-item fade (never display:none). */
nav.page-nav button .label {
  white-space: nowrap;
  font-size: clamp(0.52rem, 0.45vw + 0.42rem, 0.65rem);
  color: var(--text-dim);
  color: color-mix(in srgb, var(--text-dim) 70%, transparent);
  text-transform: uppercase;
  letter-spacing: 1.5px;
  line-height: 1;
  opacity: 1;
  max-width: none;
  overflow: visible;
  transition: none;
}
nav.page-nav button:hover .label,
nav.page-nav button.active .label {
  max-width: none;
  opacity: 1;
}

/* active dot pulse (non-linear) — the gradient + glow stay from style.css */
nav.page-nav button.active .dot {
  animation: pagenav-dot-pulse 2s cubic-bezier(0.22, 1, 0.36, 1) infinite;
}
@keyframes pagenav-dot-pulse {
  0%, 100% { transform: scale(1); }
  50% { transform: scale(1.35); }
}

/* narrow (still-desktop) windows: the whole nav tucks in, text stays put */
@media (max-width: 820px) {
  nav.page-nav {
    right: 0.6rem;
    gap: 0.4rem;
  }
  nav.page-nav button {
    gap: 0.26rem;
    padding: clamp(0.16rem, 0.4vw + 0.06rem, 0.26rem)
             clamp(0.36rem, 0.9vw + 0.06rem, 0.5rem)
             clamp(0.16rem, 0.4vw + 0.06rem, 0.26rem)
             clamp(0.14rem, 0.4vw + 0.02rem, 0.28rem);
  }
}

/* reduced motion: static, always visible, no glide/pulse */
@media (prefers-reduced-motion: reduce) {
  nav.page-nav,
  nav.page-nav button,
  nav.page-nav .dot,
  nav.page-nav .label,
  .page-nav-indicator {
    transition: none;
    animation: none;
  }
  nav.page-nav button.active .dot {
    animation: none;
  }
}
</style>
