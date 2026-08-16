<script setup lang="ts">
/* CyberCursor — native pointer + effects layer (6th revision).
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
 * The trail GEOMETRY is sampled from the RAW pointer path, sub-sampled
 * to a minimum spacing so a curved motion always yields a continuously
 * curving ribbon. The low-passed position drives only the glow and the
 * chasing ring — never the ribbon — so smoothing can no longer lag the
 * path into straight chords or corners. Speed is measured from the true
 * per-frame pointer movement, so it is exactly zero at rest and the
 * ribbon always retracts fully to the dot.
 *
 * Leaving the window fades the whole layer out via a CSS transition (no
 * frozen ghost frame); re-entering fades it back in and snaps the
 * geometry to the pointer's new position instead of reconnecting to the
 * old one with a straight line.
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
let x = -100, y = -100            // raw pointer target — trail geometry source
let sx = -100, sy = -100          // low-passed position — glow / ring only
let prevX = -100, prevY = -100    // raw target at the previous rAF tick (speed)
let visible = false
let textMode = false
let lastT = 0
let snapNextMove = false          // snap geometry on the next mousemove (re-entry)

/* Trail geometry: min-distance samples taken from the RAW pointer path.
   The number of points drawn follows a normalized trail length (0..1) that
   rises with speed quickly but releases slowly — fast movement stretches a
   long ribbon, and stopping retracts it over ~0.6-1s back to the head dot.
   Raw sampling + per-jump sub-sampling keeps a curved path curved:
   consecutive points are never farther than MIN_DIST apart. */
const PTS = 36                    // max points drawn (≈ 36 × MIN_DIST px ribbon)
const MIN_DIST = 2.0              // px between sampled points
const SPEED_MAX = 1.1             // px/ms → normalized 1 (saturates sooner)
const TAIL_ATTACK = 45            // ms time constant while growing (snappy)
const TAIL_RELEASE = 260          // ms time constant while shrinking (relaxed)
const pts: { x: number; y: number }[] = []
let lastPush = { x: -100, y: -100 } // last geometry point pushed (raw)
let speed = 0                     // EMA px/ms (true per-frame pointer speed)
let tailLen = 0                   // 0..1 — fast attack, slow release

/* click pulse: one pooled element + WAAPI — no reflow, coalesced per 150ms */
let pulseAnim: Animation | null = null
let pulseLockUntil = 0
let pulsePending = false

const CLICKABLE = 'a, button, summary, .file-item, .scroll-hint, .page-nav, [role="button"]'
const TEXT_ZONE = '.ide-code, .term-body, input, select, textarea, [contenteditable]'

/* snap every position-derived piece of state to a known pointer location.
   Used on activation and window re-entry so the trail can never reconnect
   to a stale position with a spurious straight line. */
function resetTrail(px: number, py: number) {
  pts.length = 0
  x = px; y = py
  sx = px; sy = py
  prevX = px; prevY = py
  lastPush = { x: px, y: py }
  speed = 0
  tailLen = 0
}

/* push raw geometry toward (tx, ty), splitting any jump longer than
   MIN_DIST into evenly-spaced samples so the polyline never cuts a curve
   into a straight chord (e.g. after a fast flick between events). */
function sampleTo(tx: number, ty: number) {
  let px = lastPush.x
  let py = lastPush.y
  let moved = Math.hypot(tx - px, ty - py)
  let guard = 0
  while (moved >= MIN_DIST && guard < 128) {
    const f = MIN_DIST / moved
    px += (tx - px) * f
    py += (ty - py) * f
    pts.unshift({ x: px, y: py })
    if (pts.length > PTS) pts.pop()
    moved = Math.hypot(tx - px, ty - py)
    guard++
  }
  lastPush = { x: px, y: py }
}

function onMove(e: MouseEvent) {
  const nx = e.clientX
  const ny = e.clientY
  if (!visible || snapNextMove) {
    /* activation OR re-entry after a window leave: snap to the real
       pointer position — never interpolate from a stale/offscreen one */
    snapNextMove = false
    visible = true
    resetTrail(nx, ny)
    lastT = performance.now()
    layer.value?.classList.remove('leaving')
    startEngine()
  }
  x = nx
  y = ny
  sampleTo(nx, ny)

  const el = e.target instanceof Element ? e.target : null
  const clickable = !!el && !!el.closest(CLICKABLE)
  const text = !!el && !!el.closest(TEXT_ZONE)
  textMode = text
  const hot = clickable && !text
  layer.value?.classList.toggle('hot', hot)
  layer.value?.classList.toggle('text', textMode)
}

/* leave: fade the whole layer out (CSS transition), stop the engine and
   drop every trail point so nothing lingers or reconnects on return. */
function onLeave() {
  if (!visible) return
  visible = false
  snapNextMove = true
  layer.value?.classList.add('leaving')
  stopEngine()
  pts.length = 0
  tailLen = 0
  speed = 0
  pulsePending = false
}
function onEnter() {
  if (visible) return
  visible = true
  layer.value?.classList.remove('leaving')
  lastT = performance.now()
  startEngine()
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
  p.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px)`
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

function startEngine() {
  if (raf) return
  raf = requestAnimationFrame(tick)
}
function stopEngine() {
  if (raf) cancelAnimationFrame(raf)
  raf = 0
}

function tick() {
  /* the engine only runs while the pointer is over the window */
  if (!visible) {
    raf = 0
    return
  }
  raf = requestAnimationFrame(tick)
  const now = performance.now()
  const dt = Math.max(1, now - lastT)
  lastT = now

  /* low-pass the raw target for the glow + chasing ring ONLY — the ribbon
     geometry below uses raw samples, so smoothing can't lag the path */
  sx += (x - sx) * 0.42
  sy += (y - sy) * 0.42

  /* true per-frame pointer speed → EMA. Measured from actual movement, not
     the distance to the last sample, so it is exactly 0 at rest and the
     ribbon always retracts fully (no residual "phantom speed"). */
  const dx = x - prevX
  const dy = y - prevY
  prevX = x
  prevY = y
  const inst = Math.min(Math.hypot(dx, dy) / dt, 8)
  speed += (inst - speed) * 0.22

  /* normalized trail length: fast attack, slow release — the single source
     of truth for how much of the ribbon is drawn */
  const speedNorm = Math.min(1, speed / SPEED_MAX)
  const tau = speedNorm > tailLen ? TAIL_ATTACK : TAIL_RELEASE
  tailLen += (speedNorm - tailLen) * (1 - Math.exp(-dt / tau))
  if (tailLen < 0.015) tailLen = 0 // guaranteed full collapse to the dot

  /* never draw more points than have been sampled (robustness: the ribbon
     can only shrink toward the head while geometry fills in) */
  const available = pts.length
  const nTail = Math.min(Math.round(tailLen * PTS), available)
  const nMid = Math.min(nTail, Math.round(tailLen * PTS * 0.55))
  const nHead = Math.min(nTail, 3)

  if (glowEl.value) glowEl.value.style.transform = `translate(${sx.toFixed(1)}px, ${sy.toFixed(1)}px)`
  if (ringEl.value) ringEl.value.style.transform = `translate(${sx.toFixed(1)}px, ${sy.toFixed(1)}px)`
  if (dotEl.value) dotEl.value.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px)`
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

/* The engine only runs while the cursor is ON and the pointer is over the
 * window — the top-bar toggle activates/deactivates it live (native pointer
 * stays). */
let active = false

function activate() {
  if (active || REDUCED_MOTION || !FINE_POINTER) return
  active = true
  resetTrail(-100, -100) // clear any stale trail and park offscreen
  visible = false        // wait for the pointer to report its position
  snapNextMove = false
  document.documentElement.classList.add('cyber-cursor')
  window.addEventListener('mousemove', onMove, { passive: true })
  window.addEventListener('mousedown', onDown, { passive: true })
  window.addEventListener('mouseup', onUp, { passive: true })
  document.documentElement.addEventListener('mouseleave', onLeave)
  document.documentElement.addEventListener('mouseenter', onEnter)
  startEngine()
}
function deactivate() {
  if (!active) return
  active = false
  stopEngine()
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
