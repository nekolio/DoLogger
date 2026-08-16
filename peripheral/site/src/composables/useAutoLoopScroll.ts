/* composables/useAutoLoopScroll.ts — seamless auto-scroll for page-3 card
 * content (card bodies scroll vertically).
 *
 * Motion model:
 *   - 'y' (card bodies): a slow ping-pong — scroll down, dwell briefly at
 *     the bottom, scroll back up, dwell at the top, repeat ("来回循环").
 *     Speed is a fixed px/s, so a longer card simply takes longer to
 *     traverse; it never scrolls faster because it has more content.
 *
 * Behavior:
 *   - pauses while the tab is hidden, during a short post-interaction
 *     cooldown, and while a fine pointer hovers the card (the user is
 *     reading it — wheel takes over natively);
 *   - does NOT depend on IntersectionObserver: the card bodies live inside
 *     the fixed page-3 viewport, and a missed observer callback used to
 *     freeze the loop forever ("no scroll animation" bug). The loop runs
 *     whenever the page is visible; off-screen it just scrolls harmlessly.
 *   - one rAF loop per container, every loop cancelled on detachAll().
 *     Redundant attach() calls are no-ops.
 */

interface LoopState {
  raf: number
  dir: 1 | -1          // ping-pong direction for vertical loops
  vel: number
  lastT: number
  holdUntil: number    // pause after user interaction
  dwellUntil: number   // brief pause at each end of the ping-pong
  onWheel: () => void
  onTouchStart: () => void
  onTouchEnd: () => void
}

const instances = new Map<HTMLElement, LoopState>()

const Y_SPEED = 20          // px/s — comfortable, clearly-visible vertical loop
const DWELL_MS = 1400       // ms paused at each end of the ping-pong
const PAUSE_AFTER_INTERACTION = 2500 // ms
const ACCEL_TAU = 350       // ms — speed smoothing time constant

/* Live MediaQueryList so the loop reacts to a mid-session pointer
 * change (a fine pointer appears) without a reload. */
const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')

export function useAutoLoopScroll() {
  function attach(el: HTMLElement, dir: 'y' | 'x'): void {
    /* NOTE: reduced-motion does NOT disable the loop — the loop is a
       functional content display (overflowing card content must remain
       reachable), not a decorative animation. Decorative entrances
       (fly-in) are gated separately by the components. */
    if (instances.has(el)) return
    const st: LoopState = {
      raf: 0,
      dir: 1,
      vel: 0,
      lastT: performance.now(),
      holdUntil: 0,
      dwellUntil: 0,
      onWheel: () => { st.holdUntil = performance.now() + PAUSE_AFTER_INTERACTION },
      onTouchStart: () => { st.holdUntil = Infinity },
      onTouchEnd: () => { st.holdUntil = performance.now() + 1500 },
    }
    instances.set(el, st)

    const step = (now: number) => {
      st.raf = requestAnimationFrame(step)
      const dt = Math.min(64, now - st.lastT)
      st.lastT = now
      const maxScroll = dir === 'y' ? el.scrollHeight - el.clientHeight : el.scrollWidth - el.clientWidth
      if (maxScroll <= 2) { st.vel = 0; return } // content fits — nothing to loop

      if (now < st.dwellUntil) return // hard hold at a ping-pong end

      /* target speed: 0 only while the tab is hidden, during the
         post-interaction cooldown, or while a fine pointer hovers the
         card BODY (not the whole card — hovering the title or edge keeps
         the loop visible). No IntersectionObserver gate — a missed
         callback must never freeze the loop. */
      const hovering = finePointer.matches && el.matches(':hover')
      const idle = document.hidden || now < st.holdUntil || hovering
      const target = idle ? 0 : Y_SPEED
      st.vel += (target - st.vel) * Math.min(1, dt / ACCEL_TAU)

      if (st.vel <= 0.05) return

      if (dir === 'y') {
        el.scrollTop += st.dir * st.vel * (dt / 1000)
        if (st.dir === 1 && el.scrollTop >= maxScroll - 0.5) {
          el.scrollTop = maxScroll
          st.dir = -1
          st.vel = 0
          st.dwellUntil = now + DWELL_MS
        } else if (st.dir === -1 && el.scrollTop <= 0.5) {
          el.scrollTop = 0
          st.dir = 1
          st.vel = 0
          st.dwellUntil = now + DWELL_MS
        }
      } else {
        /* horizontal axis is unused now (the arch chain flex-wraps), but
           kept for symmetry — a seamless one-way wrap over a doubled track. */
        const wrapAt = el.scrollWidth / 2
        if (wrapAt > el.clientWidth + 2) {
          el.scrollLeft += st.vel * (dt / 1000)
          if (el.scrollLeft >= wrapAt - 1) el.scrollLeft = 0
        }
      }
    }

    st.raf = requestAnimationFrame(step)
    el.addEventListener('wheel', st.onWheel, { passive: true })
    el.addEventListener('touchstart', st.onTouchStart, { passive: true })
    el.addEventListener('touchend', st.onTouchEnd, { passive: true })
  }

  function detach(el: HTMLElement): void {
    const st = instances.get(el)
    if (!st) return
    cancelAnimationFrame(st.raf)
    el.removeEventListener('wheel', st.onWheel)
    el.removeEventListener('touchstart', st.onTouchStart)
    el.removeEventListener('touchend', st.onTouchEnd)
    instances.delete(el)
  }

  function detachAll(): void {
    for (const [el, st] of instances) {
      cancelAnimationFrame(st.raf)
      el.removeEventListener('wheel', st.onWheel)
      el.removeEventListener('touchstart', st.onTouchStart)
      el.removeEventListener('touchend', st.onTouchEnd)
    }
    instances.clear()
  }

  /** attach to every matching container currently in the DOM. */
  function attachAll(selY: string, selX: string): void {
    if (selY) document.querySelectorAll<HTMLElement>(selY).forEach(el => attach(el, 'y'))
    if (selX) document.querySelectorAll<HTMLElement>(selX).forEach(el => attach(el, 'x'))
  }

  return { attach, detach, detachAll, attachAll }
}
