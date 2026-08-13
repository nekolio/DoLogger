<script setup lang="ts">
/* CyberCursor — custom cyber-style pointer for fine-pointer devices,
 * with Cyberpunk-2077-style *shape* language (not the IP itself):
 * a diamond dot at the cursor, a notched-diamond ring that lags behind
 * with corner-bracket ticks, and a fading particle trail.
 *
 * Interaction states, far more obvious than a recolor:
 *   hover over clickable  → ring opens up, corner brackets slide out,
 *                           a magenta arrowhead appears inside
 *   mousedown             → ring collapses with a flash + particle burst
 *   over text/terminal    → the dot/ring swap to an I-beam
 *
 * Memory safety: one rAF loop stopped on unmount; the trail and the
 * burst are fixed-size pools, never appended unbounded. Disabled for
 * touch devices and prefers-reduced-motion users.
 */
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { useCursorEnabled } from '../cursor'

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const FINE_POINTER = window.matchMedia('(hover: hover) and (pointer: fine)').matches
const cursorEnabled = useCursorEnabled()

const layer = ref<HTMLDivElement | null>(null)
const dot = ref<HTMLDivElement | null>(null)
const ring = ref<HTMLDivElement | null>(null)
const ibeamEl = ref<HTMLDivElement | null>(null)
const trailEl = ref<HTMLDivElement | null>(null)
const burstEl = ref<HTMLDivElement | null>(null)

let raf = 0
let x = -100
let y = -100
let rx = -100
let ry = -100
let visible = false
let textMode = false
const TRAIL_LEN = 10
const trail: { x: number; y: number }[] = []
const CLICKABLE = 'a, button, summary, .file-item, .card, .scroll-hint, .page-nav, [role="button"]'
const TEXT_ZONE = '.ide-code, .term-body, input, select, textarea, [contenteditable]'

function targetOf(e: Event): Element | null {
  return e.target instanceof Element ? e.target : null
}

function onMove(e: MouseEvent) {
  x = e.clientX
  y = e.clientY
  if (!visible) { visible = true; rx = x; ry = y }
  const el = targetOf(e)
  const hot = !!el && !!el.closest(CLICKABLE)
  const text = !!el && !!el.closest(TEXT_ZONE)
  textMode = text
  layer.value?.classList.toggle('ibeam', text)
  ring.value?.classList.toggle('hot', hot && !text)
  dot.value?.classList.toggle('hot', hot && !text)
}
function onDown() {
  if (textMode) return // I-beam: no press-collapse over text
  ring.value?.classList.add('pressed')
  burst(x, y)
}
function onUp() {
  ring.value?.classList.remove('pressed')
}
function onLeave() { visible = false }

/* fixed-size particle burst at the press point (8 shards, one shot) */
function burst(bx: number, by: number) {
  const host = burstEl.value
  if (!host) return
  host.style.transform = `translate(${bx}px, ${by}px)`
  let html = ''
  for (let i = 0; i < 8; i++) {
    const a = (i / 8) * Math.PI * 2 + (Math.random() - 0.5) * 0.6
    const d = 26 + Math.random() * 18
    html += `<i style="--bx:${(Math.cos(a) * d).toFixed(1)}px;--by:${(Math.sin(a) * d).toFixed(1)}px;--bd:${(Math.random() * 0.25).toFixed(2)}s"></i>`
  }
  host.innerHTML = html
}

function tick() {
  raf = requestAnimationFrame(tick)
  if (!visible) return
  rx += (x - rx) * 0.22
  ry += (y - ry) * 0.22
  trail.unshift({ x, y })
  if (trail.length > TRAIL_LEN) trail.pop() // fixed-size history
  if (dot.value) dot.value.style.transform = `translate(${x}px, ${y}px)`
  if (ring.value) ring.value.style.transform = `translate(${rx}px, ${ry}px)`
  if (ibeamEl.value) ibeamEl.value.style.transform = `translate(${x}px, ${y}px)`
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

/* The engine only runs while the user keeps the cyber cursor ON — the
 * top-bar toggle activates/deactivates it live (native cursor returns,
 * layer hidden via html:not(.cyber-cursor) .cyber-cursor-layer). */
let active = false

function activate() {
  if (active || REDUCED_MOTION || !FINE_POINTER) return
  active = true
  document.documentElement.classList.add('cyber-cursor')
  window.addEventListener('mousemove', onMove, { passive: true })
  window.addEventListener('mousedown', onDown, { passive: true })
  window.addEventListener('mouseup', onUp, { passive: true })
  document.documentElement.addEventListener('mouseleave', onLeave)
  raf = requestAnimationFrame(tick)
}
function deactivate() {
  if (!active) return
  active = false
  cancelAnimationFrame(raf)
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('mousedown', onDown)
  window.removeEventListener('mouseup', onUp)
  document.documentElement.removeEventListener('mouseleave', onLeave)
  document.documentElement.classList.remove('cyber-cursor')
}

onMounted(() => {
  watch(cursorEnabled, (on) => (on ? activate() : deactivate()), { immediate: true })
})
onBeforeUnmount(deactivate)
</script>

<template>
  <div class="cyber-cursor-layer" ref="layer" aria-hidden="true">
    <div class="cursor-trail" ref="trailEl"></div>
    <div class="cursor-ibeam" ref="ibeamEl"><i class="cap top"></i><i class="cap bottom"></i></div>
    <div class="cursor-ring" ref="ring">
      <span class="dia"></span>
      <i class="br tl"></i><i class="br tr"></i><i class="br bl"></i><i class="br br"></i>
      <svg class="cursor-arrow" viewBox="0 0 20 20"><path d="M3 2 L17 10 L3 18 L6.5 10 Z" /></svg>
    </div>
    <div class="cursor-dot" ref="dot"></div>
    <div class="cursor-burst" ref="burstEl"></div>
  </div>
</template>
