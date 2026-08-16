<script setup lang="ts">
/* CyberCursor — native pointer + effects layer (5th revision).
 *
 * The SYSTEM pointer stays visible and usable; this layer only adds
 * polish on top of it:
 *   - a soft glow bloom;
 *   - a comet trail whose LENGTH follows cursor SPEED with a fast attack
 *     but a slow, relaxed release: moving stretches a ribbon, stopping
 *     lets it linger ~0.6-1s and then retract back into a single light
 *     dot — it never freezes behind a resting cursor;
 *   - one chasing ring;
 *   - a restrained press: the ring squeezes subtly and a soft, brief
 *     glow pulse blooms and fades (no expanding shockwave, no snap frame).
 *
 * The trail is fed by an exponentially low-passed position, so hand
 * jitter is smoothed away before it is ever recorded. Leaving the
 * window fades the whole layer out via a CSS transition (no frozen
 * ghost frame); re-entering fades it back in.
 *
 * The click path is cheap and idempotent: pointerdowns only set a flag
 * and add a class, and the pooled pulse is driven by the Web Animations
 * API from inside the rAF tick (throttled to once per ~150ms) — rapid
 * clicks never force a reflow or churn classes mid-frame.
 *
 * States are recolor-only (the native I-beam already handles text
 * zones): hover over clickable → magenta recolor, over text → cyan.
 * Disabled for touch devices and reduced-motion users; the toggle in
 * the top bar turns the effects on/off live (the pointer is always the
 * native one — html:not(.cyber-cursor) hides this layer).
 */
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { useCursorEnabled } from '../cursor'

const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const FINE_POINTER = window.matchMedia('(hover: hover) and (pointer: fine)').matches
const cursorEnabled = useCursorEnabled()

const layer = ref<HTMLElement | null>(null)
const glowEl = ref<HTMLDivElement | null>(null)
const cometTailEl = ref<SVGPolylineElement | null>(null)
const cometMidEl = ref<SVGPolylineElement | null>(null)
const cometCoreEl = ref<SVGPolylineElement | null>(null)
const cometHeadEl = ref<SVGPolylineElement | null>(null)
const ringEl = ref<HTMLDivElement | null>(null)
const dotEl = ref<HTMLDivElement | null>(null)
const pulseEl = ref<HTMLDivElement | null>(null)

let raf = 0
let x = -100, y = -100            // raw target
let sx = -100, sy = -100          // smoothed (low-passed) position
let visible = false
let textMode = false
let lastT = 0

/* Trail: min-distance sampled from the smoothed position. The number of
   points we draw follows a normalized trail length (0..1) that rises with
   speed quickly but releases slowly — fast movement stretches a long
   ribbon, and stopping retracts it over ~0.6-1s back to the head dot. */
const PTS = 36                    // max points drawn (≈ 36 × MIN_DIST px ribbon)
const MIN_DIST = 2.0              // px between sampled points
const SPEED_MAX = 1.1             // px/ms → normalized 1 (saturates sooner)
const TAIL_ATTACK = 45            // ms time constant while growing (snappy)
const TAIL_RELEASE = 260          // ms time constant while shrinking (relaxed)
const pts: { x: number; y: number }[] = []
let lastPush = { x: -100, y: -100 }
let speed = 0                     // EMA px/ms
let tailLen = 0                   // 0..1 — fast attack, slow release

/* click pulse: one pooled element + WAAPI — no reflow, coalesced per 150ms */
let pulseAnim: Animation | null = null
let pulseLockUntil = 0
let pulsePending = false

const CLICKABLE = 'a, button, summary, .file-item, .scroll-hint, .page-nav, [role="button"]'
const TEXT_ZONE = '.ide-code, .term-body, input, select, textarea, [contenteditable]'

function onMove(e: MouseEvent) {
  x = e.clientX
  y = e.clientY
  if (!visible) {
    visible = true
    sx = x; sy = y
    lastPush = { x, y }
    lastT = performance.now()
    layer.value?.classList.remove('leaving')
  }
  const el = e.target instanceof Element ? e.target : null
  const clickable = !!el && !!el.closest(CLICKABLE)
  const text = !!el && !!el.closest(TEXT_ZONE)
  textMode = text
  const hot = clickable && !text
  layer.value?.classList.toggle('hot', hot)
  layer.value?.classList.toggle('text', textMode)
}

/* leave: fade the whole layer out (CSS transition), stop the engine.
   The last painted frame dissolves instead of freezing on screen. */
function onLeave() {
  if (!visible) return
  visible = false
  layer.value?.classList.add('leaving')
  pts.length = 0
  tailLen = 0
  speed = 0
  pulsePending = false
  cancelAnimationFrame(raf)
  raf = 0
}
function onEnter() {
  if (!visible) {
    visible = true
    layer.value?.classList.remove('leaving')
    lastT = performance.now()
    raf = requestAnimationFrame(tick)
  }
}

function onDown() {
  if (textMode) return // text zones keep the native I-beam feel
  layer.value?.classList.add('pressed')
  pulsePending = true // coalesce: the rAF tick fires it, throttled to ~150ms
}
function onUp() {
  layer.value?.classList.remove('pressed')
}

function setPoints(el: SVGPolylineElement | null, n: number) {
  if (!el) return
  let s = ''
  for (let i = n - 1; i >= 0; i--) {
    if (s) s += ' '
    s += pts[i].x.toFixed(1) + ',' + pts[i].y.toFixed(1)
  }
  el.setAttribute('points', s)
}

/* fire the pooled pulse via the Web Animations API — no class churn, no
   forced reflow (offsetWidth), so rapid clicking stays smooth. */
function firePulse() {
  const p = pulseEl.value
  if (!p) return
  const i = p.firstElementChild as HTMLElement | null
  if (!i) return
  p.style.transform = `translate(${sx.toFixed(1)}px, ${sy.toFixed(1)}px)`
  pulseAnim?.cancel()
  /* easing mirrors the site's --ease-out: cubic-bezier(0.22, 1, 0.36, 1) */
  pulseAnim = i.animate(
    [
      { opacity: 0, transform: 'scale(0.6)' },
      { opacity: 0.3, transform: 'scale(0.92)', offset: 0.45 },
      { opacity: 0, transform: 'scale(1.05)' }
    ],
    { duration: 320, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', fill: 'forwards' }
  )
}

function tick() {
  raf = requestAnimationFrame(tick)
  if (!visible) return
  const now = performance.now()
  const dt = Math.max(1, now - lastT)
  lastT = now

  /* smooth the raw target — exponential low-pass kills hand jitter */
  sx += (x - sx) * 0.42
  sy += (y - sy) * 0.42

  /* instantaneous speed → EMA; then a normalized trail length that rises
     fast but releases slowly (the single source of truth for the ribbon) */
  const dist = Math.hypot(sx - lastPush.x, sy - lastPush.y)
  const inst = Math.min(dist / dt, 8)
  speed += (inst - speed) * 0.22
  if (dist >= MIN_DIST) {
    pts.unshift({ x: sx, y: sy })
    if (pts.length > PTS) pts.pop()
    lastPush = { x: sx, y: sy }
  }

  const speedNorm = Math.min(1, speed / SPEED_MAX)
  const tau = speedNorm > tailLen ? TAIL_ATTACK : TAIL_RELEASE
  tailLen += (speedNorm - tailLen) * (1 - Math.exp(-dt / tau))
  if (tailLen < 0.015) tailLen = 0 // guaranteed full collapse to the dot

  const nTail = Math.round(tailLen * PTS)
  const nMid = Math.min(nTail, Math.round(tailLen * PTS * 0.55))
  const nHead = Math.min(nTail, 3)

  if (glowEl.value) glowEl.value.style.transform = `translate(${sx.toFixed(1)}px, ${sy.toFixed(1)}px)`
  if (ringEl.value) ringEl.value.style.transform = `translate(${sx.toFixed(1)}px, ${sy.toFixed(1)}px)`
  if (dotEl.value) dotEl.value.style.transform = `translate(${sx.toFixed(1)}px, ${sy.toFixed(1)}px)`
  setPoints(cometTailEl.value, nTail)
  setPoints(cometMidEl.value, nMid)
  setPoints(cometCoreEl.value, nHead)
  setPoints(cometHeadEl.value, nHead)
  /* the ribbon's opacity follows the trail length too — a stopped cursor
     leaves only the dot (its own opacity is static) */
  const ribbonOpacity = (tailLen * 0.85).toFixed(2)
  const svg = cometTailEl.value?.ownerSVGElement
  if (svg) svg.style.opacity = ribbonOpacity

  /* coalesced, throttled click pulse — fires at most once per ~150ms */
  if (pulsePending && now >= pulseLockUntil) {
    pulsePending = false
    pulseLockUntil = now + 150
    firePulse()
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
  document.documentElement.addEventListener('mouseenter', onEnter)
  raf = requestAnimationFrame(tick)
}
function deactivate() {
  if (!active) return
  active = false
  cancelAnimationFrame(raf)
  raf = 0
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('mousedown', onDown)
  window.removeEventListener('mouseup', onUp)
  document.documentElement.removeEventListener('mouseleave', onLeave)
  document.documentElement.removeEventListener('mouseenter', onEnter)
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
      <polyline ref="cometTailEl" class="tail"></polyline>
      <polyline ref="cometMidEl" class="mid"></polyline>
      <polyline ref="cometCoreEl" class="core"></polyline>
      <polyline ref="cometHeadEl" class="head"></polyline>
    </svg>
    <div class="cursor-ring" ref="ringEl"></div>
    <div class="cursor-dot" ref="dotEl"></div>
    <div class="cursor-pulse" ref="pulseEl"><i></i></div>
  </div>
</template>

<style>
/* CyberCursor effect overrides — calmer, softer replacements for the
   global cursor-effect rules in src/style.css ("Cyber cursor effects
   layer" section). `#app` keeps specificity above the global selectors
   so these win regardless of bundle order (component <style> blocks are
   emitted before the global stylesheet). */
#app .cyber-cursor-layer .cursor-comet {
  /* opacity is now driven per-frame by the relaxed trail length in JS —
     the CSS transition would only double-smooth (and delay) it */
  transition: none;
}
#app .cyber-cursor-layer .cursor-pulse {
  position: fixed;
  left: 0;
  top: 0;
  width: 32px;
  height: 32px;
  margin: -16px 0 0 -16px;
  pointer-events: none;
}
#app .cyber-cursor-layer .cursor-pulse i {
  position: absolute;
  inset: 0;
  border: none;
  border-radius: 50%;
  background: radial-gradient(circle, rgba(244, 114, 208, 0.3) 0%, rgba(199, 146, 234, 0.1) 55%, transparent 72%);
  opacity: 0;
}
/* the old expanding-shockwave keyframes are dead: WAAPI drives this glow */
#app .cyber-cursor-layer .cursor-pulse.go i { animation: none; }
/* restrained press: a gentle squeeze instead of a hard pinch */
#app .cyber-cursor-layer.pressed .cursor-ring {
  scale: 0.82;
  border-color: #F472D0;
  box-shadow: 0 0 12px rgba(244, 114, 208, 0.28);
}
#app .cyber-cursor-layer.pressed .cursor-dot {
  scale: 1.25;
  background: #F472D0;
}
</style>
