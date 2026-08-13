<script setup lang="ts">
/* CyberCursor — native pointer + effects layer (3rd revision).
 *
 * The SYSTEM pointer stays visible and usable; this layer only adds
 * flash on top of it: a soft glow bloom, a brand-ramp comet trail
 * (fixed-size point history, no innerHTML churn — polylines updated
 * via setAttribute), twin chasing rings, a snap frame that lerps onto
 * the hovered click target, and a click ripple.
 *
 * States are recolor-only (shape never swaps — the native I-beam
 * already handles text zones):
 *   hover over clickable → magenta recolor (+ snap frame on the target)
 *   over text/terminal   → cyan recolor
 *   mousedown            → rings tighten + ripple
 *
 * Memory safety: one rAF loop stopped on unmount; the point history is
 * a fixed-size array. Disabled for touch devices and reduced-motion
 * users. The toggle only controls the effects — the pointer is always
 * the native one (html:not(.cyber-cursor) .cyber-cursor-layer hides).
 */
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { useCursorEnabled } from '../cursor'

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const FINE_POINTER = window.matchMedia('(hover: hover) and (pointer: fine)').matches
const cursorEnabled = useCursorEnabled()

const layer = ref<HTMLElement | null>(null)
const glowEl = ref<HTMLDivElement | null>(null)
const cometEl = ref<SVGPolylineElement | null>(null)
const cometHeadEl = ref<SVGPolylineElement | null>(null)
const ringOEl = ref<HTMLDivElement | null>(null)
const ringIEl = ref<HTMLDivElement | null>(null)
const snapEl = ref<HTMLDivElement | null>(null)
const rippleEl = ref<HTMLDivElement | null>(null)

let raf = 0
let x = -100
let y = -100
/* per-layer positions with different lerp speeds */
let gox = -100, goy = -100 // glow   (fast)
let ox = -100, oy = -100   // ring-o (slow)
let ixx = -100, iyy = -100 // ring-i (fast)
let visible = false
let textMode = false

const PTS = 24
const pts: { x: number; y: number }[] = []

/* snap frame: lerps toward the hovered click target's rect */
const snap = { x: 0, y: 0, w: 0, h: 0, active: 0 }
const snapCur = { x: 0, y: 0, w: 0, h: 0, a: 0 }

const CLICKABLE = 'a, button, summary, .file-item, .scroll-hint, .page-nav, [role="button"]'
const TEXT_ZONE = '.ide-code, .term-body, input, select, textarea, [contenteditable]'

function onMove(e: MouseEvent) {
  x = e.clientX
  y = e.clientY
  if (!visible) { visible = true; ox = ixx = gox = x; oy = iyy = goy = y }
  const el = e.target instanceof Element ? e.target : null
  const clickable = !!el && !!el.closest(CLICKABLE)
  const text = !!el && !!el.closest(TEXT_ZONE)
  textMode = text
  const hot = clickable && !text
  layer.value?.classList.toggle('hot', hot)
  layer.value?.classList.toggle('text', textMode)
  if (clickable && el) {
    const r = el.getBoundingClientRect()
    snap.x = r.left - 4
    snap.y = r.top - 4
    snap.w = r.width + 8
    snap.h = r.height + 8
    snap.active = 1
  } else {
    snap.active = 0
  }
}
function onDown() {
  if (textMode) return // text zones keep the native I-beam feel
  layer.value?.classList.add('pressed')
  const r = rippleEl.value
  if (r) {
    r.style.transform = `translate(${x - 24}px, ${y - 24}px)`
    r.classList.remove('go')
    void r.offsetWidth // reflow restarts the pooled animation
    r.classList.add('go')
  }
}
function onUp() {
  layer.value?.classList.remove('pressed')
}
function onLeave() {
  visible = false
  pts.length = 0
}

function setPoints(el: SVGPolylineElement | null, len: number) {
  if (!el) return
  let s = ''
  const n = Math.min(pts.length, len)
  for (let i = 0; i < n; i++) {
    if (i) s += ' '
    s += pts[i].x.toFixed(1) + ',' + pts[i].y.toFixed(1)
  }
  el.setAttribute('points', s)
}

function tick() {
  raf = requestAnimationFrame(tick)
  if (!visible) return
  const lerpO = 0.12
  const lerpI = 0.3
  const lerpG = 0.35
  ox += (x - ox) * lerpO
  oy += (y - oy) * lerpO
  ixx += (x - ixx) * lerpI
  iyy += (y - iyy) * lerpI
  gox += (x - gox) * lerpG
  goy += (y - goy) * lerpG
  pts.unshift({ x, y })
  if (pts.length > PTS) pts.pop() // fixed-size history
  if (glowEl.value) glowEl.value.style.transform = `translate(${gox.toFixed(1)}px, ${goy.toFixed(1)}px)`
  if (ringOEl.value) ringOEl.value.style.transform = `translate(${ox.toFixed(1)}px, ${oy.toFixed(1)}px)`
  if (ringIEl.value) ringIEl.value.style.transform = `translate(${ixx.toFixed(1)}px, ${iyy.toFixed(1)}px)`
  setPoints(cometEl.value, PTS)
  setPoints(cometHeadEl.value, 6)
  /* snap frame — lerp the rect toward the target, fade with its activity */
  snapCur.x += (snap.x - snapCur.x) * 0.3
  snapCur.y += (snap.y - snapCur.y) * 0.3
  snapCur.w += (snap.w - snapCur.w) * 0.3
  snapCur.h += (snap.h - snapCur.h) * 0.3
  snapCur.a += (snap.active - snapCur.a) * 0.2
  if (snapEl.value) {
    snapEl.value.style.transform = `translate(${snapCur.x.toFixed(1)}px, ${snapCur.y.toFixed(1)}px)`
    snapEl.value.style.width = snapCur.w.toFixed(1) + 'px'
    snapEl.value.style.height = snapCur.h.toFixed(1) + 'px'
    snapEl.value.style.opacity = (snapCur.a * 0.9).toFixed(2)
  }
}

/* The engine only runs while the user keeps the cyber cursor ON — the
 * top-bar toggle activates/deactivates it live (native pointer stays). */
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
    <div class="cursor-glow" ref="glowEl"></div>
    <svg class="cursor-comet" aria-hidden="true">
      <defs>
        <linearGradient id="cursor-grad" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#7FD5FF" />
          <stop offset="0.5" stop-color="#C792EA" />
          <stop offset="1" stop-color="#F472D0" />
        </linearGradient>
      </defs>
      <polyline ref="cometEl"></polyline>
      <polyline ref="cometHeadEl" class="head"></polyline>
    </svg>
    <div class="cursor-ring-o" ref="ringOEl"></div>
    <div class="cursor-ring-i" ref="ringIEl"></div>
    <div class="cursor-snap" ref="snapEl"></div>
    <div class="cursor-ripple" ref="rippleEl"><i></i></div>
  </div>
</template>
