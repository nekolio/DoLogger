<script setup lang="ts">
/* PageNav — right-edge navigation. Small dots that grow into labeled
 * pills on hover/active, so the current section is always readable.
 * The whole nav idles out (non-linear fade + slide) after 1.8s without
 * interaction — any mousemove / wheel / key / page change pokes it
 * awake again; hovering it keeps it awake. Skipped under reduced motion
 * (always visible). */
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps<{ count: number; active: number }>()
const emit = defineEmits<{ (e: 'go', i: number): void }>()
const { t } = useI18n()
const LABELS = ['nav-hero', 'nav-demo', 'nav-overview']
const NUMS = ['01 ·', '02 ·', '03 ·']

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const idle = ref(false)
const IDLE_MS = 1800
let idleTimer = 0
let pokeRaf = 0

function poke() {
  if (REDUCED_MOTION) return
  idle.value = false
  clearTimeout(idleTimer)
  idleTimer = window.setTimeout(() => { idle.value = true }, IDLE_MS)
}
/* rAF-throttled mousemove poke */
function onMove() {
  if (pokeRaf) return
  pokeRaf = requestAnimationFrame(() => { pokeRaf = 0; poke() })
}
function onWheel() { poke() }
function onKey() { poke() }
function navEnter() {
  if (REDUCED_MOTION) return
  idle.value = false
  clearTimeout(idleTimer)
}

watch(() => props.active, poke)
onMounted(() => {
  window.addEventListener('mousemove', onMove)
  window.addEventListener('wheel', onWheel, { passive: true })
  window.addEventListener('keydown', onKey)
  poke()
})
onBeforeUnmount(() => {
  clearTimeout(idleTimer)
  cancelAnimationFrame(pokeRaf)
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('wheel', onWheel)
  window.removeEventListener('keydown', onKey)
})
</script>

<template>
  <nav class="page-nav" :class="{ idle }" aria-label="sections" @mouseenter="navEnter" @mouseleave="poke">
    <button v-for="i in count" :key="i"
            type="button"
            :class="{ active: active === i - 1 }"
            :aria-label="t(LABELS[i - 1])"
            :aria-current="active === i - 1 ? 'true' : undefined"
            @click="emit('go', i - 1)">
      <span class="dot"></span>
      <span class="label"><b>{{ NUMS[i - 1] }}</b>{{ t(LABELS[i - 1]) }}</span>
    </button>
  </nav>
</template>
