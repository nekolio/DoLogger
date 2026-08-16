/* composables/useAutoLoopScroll.ts — seamless auto-scroll for page-3 card
 * content ("滚动播放卡片里的内容").
 *
 * Motion model (deliberately SIMPLE — this is the classic content ticker):
 *   - scrollTop increases at a constant speed (px/s), so content scrolls
 *     from bottom to top;
 *   - at the bottom it wraps back to the top and continues — a seamless
 *     loop (content is expected to be long enough to look continuous; a
 *     short card simply loops sooner);
 *   - the speed is the same for every card regardless of content length.
 *
 * Pauses ONLY while the tab is hidden or a fine pointer hovers the card
 * body (the user is reading it — wheel takes over natively). There is NO
 * IntersectionObserver gating and NO hover grace period: the earlier
 * versions' extra logic is exactly what made the loop appear "not
 * implemented". Reduced-motion does not disable it (it is a functional
 * content display, not decoration).
 */

interface LoopState {
  raf: number
  lastT: number
  holdUntil: number    // pause after user interaction
  onWheel: () => void
  onTouchStart: () => void
  onTouchEnd: () => void
}

const instances = new Map<HTMLElement, LoopState>()

const Y_SPEED = 26          // px/s — steady, clearly-visible content ticker
const PAUSE_AFTER_INTERACTION = 2500 // ms

const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')

export function useAutoLoopScroll() {
  function attach(el: HTMLElement, dir: 'y' | 'x'): void {
    if (instances.has(el)) return
    const st: LoopState = {
      raf: 0,
      lastT: performance.now(),
      holdUntil: 0,
      onWheel: () => { st.holdUntil = performance.now() + PAUSE_AFTER_INTERACTION },
      onTouchStart: () => { st.holdUntil = Infinity },
      onTouchEnd: () => { st.holdUntil = performance.now() + 1500 },
    }
    instances.set(el, st)

    const step = (now: number) => {
      st.raf = requestAnimationFrame(step)
      const dt = Math.min(64, now - st.lastT)
      st.lastT = now
      if (dir !== 'y') return // only the vertical ticker is used now
      const maxScroll = el.scrollHeight - el.clientHeight
      if (maxScroll <= 2) return // content fits — nothing to loop

      /* pause only on tab-hidden, post-interaction cooldown, or hover */
      const hovering = finePointer.matches && el.matches(':hover')
      if (document.hidden || now < st.holdUntil || hovering) return

      el.scrollTop += Y_SPEED * (dt / 1000)
      if (el.scrollTop >= maxScroll) el.scrollTop = 0 // wrap: seamless loop
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
