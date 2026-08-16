/* composables/useAutoLoopScroll.ts — seamless auto-scroll for page-3 card
 * content (card bodies scroll vertically, the architecture pipeline
 * scrolls horizontally).
 *
 * Two motion models, chosen per axis:
 *   - 'y' (card bodies): a slow ping-pong — scroll down, dwell briefly at
 *     the bottom, scroll back up, dwell at the top, repeat ("来回循环").
 *     Speed is a fixed px/s, so a longer card simply takes longer to
 *     traverse; it never scrolls faster because it has more content.
 *   - 'x' (pipe marquee): a continuous wrap over a doubled content track —
 *     scrolling wraps at half the track width, which is pixel-identical to
 *     the head, so the loop is seamless and the pipeline always flows one
 *     way (assembly never runs backwards).
 *
 * Behavior:
 *   - pauses on hover (fine pointers), on wheel/touch interaction, while
 *     the tab is hidden, and while the element is scrolled out of view;
 *   - respects prefers-reduced-motion (no loop at all — static content);
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
  inView: boolean      // IntersectionObserver: element within the viewport
  onWheel: () => void
  onTouchStart: () => void
  onTouchEnd: () => void
  io: IntersectionObserver | null
}

const instances = new Map<HTMLElement, LoopState>()

const Y_SPEED = 14          // px/s — gentle vertical loop (12–18)
const X_SPEED = 24          // px/s — marquee (20–30)
const DWELL_MS = 1400       // ms paused at each end of the ping-pong
const PAUSE_AFTER_INTERACTION = 2500 // ms
const ACCEL_TAU = 350       // ms — speed smoothing time constant

/* Live MediaQueryLists so the loop reacts to a mid-session change (a
 * pointer appears, reduced motion toggles) without a reload. */
const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)')

export function useAutoLoopScroll() {
  function hovered(el: HTMLElement): boolean {
    /* Sticky :hover on touch would freeze the loop forever — only pause
       on genuine hover-capable pointers. Pausing when the whole card is
       hovered (not just the body) matches "hover pauses the card". */
    if (!finePointer.matches) return false
    const card = el.closest('.card, .mcard')
    return card ? card.matches(':hover') : el.matches(':hover')
  }

  function attach(el: HTMLElement, dir: 'y' | 'x'): void {
    if (instances.has(el) || reducedMotion.matches) return
    const st: LoopState = {
      raf: 0,
      dir: 1,
      vel: 0,
      lastT: performance.now(),
      holdUntil: 0,
      dwellUntil: 0,
      inView: true,
      onWheel: () => { st.holdUntil = performance.now() + PAUSE_AFTER_INTERACTION },
      onTouchStart: () => { st.holdUntil = Infinity },
      onTouchEnd: () => { st.holdUntil = performance.now() + 1500 },
      io: null
    }
    instances.set(el, st)

    const step = (now: number) => {
      st.raf = requestAnimationFrame(step)
      const dt = Math.min(64, now - st.lastT)
      st.lastT = now
      const maxScroll = dir === 'y' ? el.scrollHeight - el.clientHeight : el.scrollWidth - el.clientWidth
      if (maxScroll <= 2) { st.vel = 0; return } // content fits — nothing to loop

      if (now < st.dwellUntil) return // hard hold at a ping-pong end

      /* target speed: 0 while hidden / off-screen / interacted / hovered */
      const idle = document.hidden || !st.inView || now < st.holdUntil || hovered(el)
      const target = idle ? 0 : (dir === 'y' ? Y_SPEED : X_SPEED)
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
        /* the marquee track is the content DOUBLED — wrapping at half the
           scroll width shows the second copy's head, which is pixel-identical
           to the first copy's head: a seamless one-way loop. */
        const wrapAt = el.scrollWidth / 2
        if (wrapAt > el.clientWidth + 2) {
          el.scrollLeft += st.vel * (dt / 1000)
          if (el.scrollLeft >= wrapAt - 1) el.scrollLeft = 0
        }
      }
    }

    const io = new IntersectionObserver((entries) => {
      for (const e of entries) st.inView = e.isIntersecting
    }, { root: null, threshold: 0 })
    io.observe(el)
    st.io = io

    st.raf = requestAnimationFrame(step)
    el.addEventListener('wheel', st.onWheel, { passive: true })
    el.addEventListener('touchstart', st.onTouchStart, { passive: true })
    el.addEventListener('touchend', st.onTouchEnd, { passive: true })
  }

  function detach(el: HTMLElement): void {
    const st = instances.get(el)
    if (!st) return
    cancelAnimationFrame(st.raf)
    st.io?.disconnect()
    el.removeEventListener('wheel', st.onWheel)
    el.removeEventListener('touchstart', st.onTouchStart)
    el.removeEventListener('touchend', st.onTouchEnd)
    instances.delete(el)
  }

  function detachAll(): void {
    for (const [el, st] of instances) {
      cancelAnimationFrame(st.raf)
      st.io?.disconnect()
      el.removeEventListener('wheel', st.onWheel)
      el.removeEventListener('touchstart', st.onTouchStart)
      el.removeEventListener('touchend', st.onTouchEnd)
    }
    instances.clear()
  }

  /** attach to every matching container currently in the DOM. */
  function attachAll(selY: string, selX: string): void {
    document.querySelectorAll<HTMLElement>(selY).forEach(el => attach(el, 'y'))
    document.querySelectorAll<HTMLElement>(selX).forEach(el => attach(el, 'x'))
  }

  return { attach, detach, detachAll, attachAll }
}
