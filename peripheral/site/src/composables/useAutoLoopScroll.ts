/* composables/useAutoLoopScroll.ts — ping-pong auto-scroll for page-3 PC
 * card content ("来回循环").
 *
 * Motion model:
 *   - scrollTop rises at a constant speed (px/s) until the bottom, dwells
 *     briefly, then falls back to the top and dwells — an endless
 *     back-and-forth loop (乒乓). NOT a wrap-to-top ticker.
 *   - the speed is the SAME for every card regardless of content length
 *     (a longer card simply takes longer to traverse).
 *
 * Pauses ONLY while the tab is hidden, during a short post-interaction
 * cooldown, or while a fine pointer hovers the card body (the user is
 * reading it — wheel takes over natively). No IntersectionObserver gate,
 * no hover grace period: extra logic is what made the loop appear
 * "not implemented". Reduced-motion does not disable it (functional
 * content display, not decoration).
 *
 * IMPORTANT: attach() is called with the PC grid's .card-body only —
 * mobile .mcard bodies are NOT looped (the selector in PageOverview
 * targets '#page3 .grid .card-body').
 */

interface LoopState {
  raf: number
  dir: 1 | -1          // ping-pong direction
  vel: number
  lastT: number
  holdUntil: number    // pause after user interaction
  dwellUntil: number   // brief pause at each end of the ping-pong
  onWheel: () => void
  onTouchStart: () => void
  onTouchEnd: () => void
}

const instances = new Map<HTMLElement, LoopState>()

const Y_SPEED = 22          // px/s — steady, clearly-visible vertical ping-pong
const DWELL_MS = 1400       // ms paused at each end
const PAUSE_AFTER_INTERACTION = 2500 // ms
const ACCEL_TAU = 350       // ms — speed smoothing time constant

const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')

export function useAutoLoopScroll() {
  function attach(el: HTMLElement, dir: 'y' | 'x'): void {
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
      if (dir !== 'y') return
      const maxScroll = el.scrollHeight - el.clientHeight
      if (maxScroll <= 2) { st.vel = 0; return } // content fits — nothing to loop

      if (now < st.dwellUntil) return // hard hold at a ping-pong end

      /* pause only on tab-hidden, post-interaction cooldown, or hover */
      const hovering = finePointer.matches && el.matches(':hover')
      const idle = document.hidden || now < st.holdUntil || hovering
      const target = idle ? 0 : Y_SPEED
      st.vel += (target - st.vel) * Math.min(1, dt / ACCEL_TAU)
      if (st.vel <= 0.05) return

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
