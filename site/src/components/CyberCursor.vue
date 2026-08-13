<script setup lang="ts">
/* CyberCursor — custom cyber-style pointer for fine-pointer devices:
 * an exact dot at the cursor, a lerped diamond ring that lags behind,
 * and a fading particle trail. The native cursor is hidden only while
 * this is active (`html.cyber-cursor`). Disabled entirely for touch
 * devices and prefers-reduced-motion users.
 *
 * Memory safety: one rAF loop, stopped on unmount; the trail is a
 * fixed-size history, never appended unbounded.
 */
import { ref, onMounted, onBeforeUnmount } from 'vue'

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const FINE_POINTER = window.matchMedia('(hover: hover) and (pointer: fine)').matches

const dot = ref<HTMLDivElement | null>(null)
const ring = ref<HTMLDivElement | null>(null)
const trailEl = ref<HTMLDivElement | null>(null)

let raf = 0
let x = -100
let y = -100
let rx = -100
let ry = -100
let visible = false
const TRAIL_LEN = 10
const trail: { x: number; y: number }[] = []

function onMove(e: MouseEvent) {
  x = e.clientX
  y = e.clientY
  if (!visible) { visible = true; rx = x; ry = y }
  const hot = !!(e.target instanceof Element && e.target.closest('a, button, summary, .file-item, .card, .scroll-hint, .page-nav'))
  ring.value?.classList.toggle('hot', hot)
  dot.value?.classList.toggle('hot', hot)
}
function onLeave() { visible = false }

function tick() {
  raf = requestAnimationFrame(tick)
  if (!visible) return
  rx += (x - rx) * 0.22
  ry += (y - ry) * 0.22
  trail.unshift({ x, y })
  if (trail.length > TRAIL_LEN) trail.pop() // fixed-size history
  if (dot.value) dot.value.style.transform = `translate(${x}px, ${y}px)`
  if (ring.value) ring.value.style.transform = `translate(${rx}px, ${ry}px)`
  if (trailEl.value) {
    let html = ''
    for (let i = trail.length - 1; i >= 0; i--) {
      const a = 0.45 * (1 - i / TRAIL_LEN)
      const s = 7 - i * 0.55
      html += `<i style="opacity:${a.toFixed(2)};width:${s.toFixed(1)}px;height:${s.toFixed(1)}px;transform:translate(${(trail[i].x - s / 2).toFixed(1)}px,${(trail[i].y - s / 2).toFixed(1)}px)"></i>`
    }
    trailEl.value.innerHTML = html
  }
}

onMounted(() => {
  if (REDUCED_MOTION || !FINE_POINTER) return
  document.documentElement.classList.add('cyber-cursor')
  window.addEventListener('mousemove', onMove, { passive: true })
  document.documentElement.addEventListener('mouseleave', onLeave)
  raf = requestAnimationFrame(tick)
})
onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  window.removeEventListener('mousemove', onMove)
  document.documentElement.removeEventListener('mouseleave', onLeave)
  document.documentElement.classList.remove('cyber-cursor')
})
</script>

<template>
  <div class="cyber-cursor-layer" aria-hidden="true">
    <div class="cursor-trail" ref="trailEl"></div>
    <div class="cursor-ring" ref="ring"></div>
    <div class="cursor-dot" ref="dot"></div>
  </div>
</template>
