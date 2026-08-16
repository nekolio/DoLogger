/* composables/usePageNav.ts — PPT-style wheel/touch navigation.
 *
 * Rules (as specified):
 *   - any wheel movement within 500 ms of the last page switch counts as
 *     ONE action and is ignored — a cooldown, like a skill in a game;
 *   - after the cooldown, one wheel gesture in either direction moves
 *     exactly one page;
 *   - it can never get stuck: an inner scroller under the cursor (demo
 *     terminal, filter-popup results, an expanded page-3 card) scrolls
 *     natively first — the page switch fires only when that scroller
 *     hits its edge. Hard zones ([data-wheel-lock-hard]: the filter
 *     popup, the demo terminal, page 3 while a card is inspected)
 *     absorb the wheel and never trigger a page switch while the
 *     pointer is over them;
 *   - touch devices get the SAME one-page-per-gesture rule: a vertical
 *     swipe flips exactly one page regardless of amplitude or speed —
 *     only the direction matters. Swipes that start on an inner
 *     scroller (page 1 hero, page 3 cards) scroll it natively instead.
 *
 * Keyboard mirrors the wheel: ↓ / PageDown / Space = next, ↑ / PageUp /
 * Shift+Space = previous, Home / End jump to the edges.
 */

import { ref, onMounted, onBeforeUnmount } from 'vue'

const PAGES = ['page1', 'page2', 'page3']
const COOLDOWN_MS = 500
const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches

export function usePageNav() {
  const active = ref(0)
  const count = PAGES.length
  /** direction of the last page transition (+1 next / -1 prev / 0) —
   *  consumed by page 3 to aim its card entry animation */
  const lastDir = ref<1 | -1 | 0>(0)

  const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')
  const narrow = window.matchMedia('(max-width: 700px)')

  function enabled(): boolean {
    return finePointer.matches && !narrow.matches
  }

  function goTo(i: number) {
    const prev = active.value
    if (i < 0 || i >= count) return
    const el = document.getElementById(PAGES[i])
    if (!el) return
    el.scrollIntoView({ behavior: REDUCED_MOTION ? 'auto' : 'smooth' })
    active.value = i
    lastDir.value = i > prev ? 1 : i < prev ? -1 : 0
    /* Mobile Chromium can drop the final paint after a smooth
       programmatic scroll (blank / half-visible page until a manual
       interaction). Verify a beat later and snap the page back into
       place if the scroll did not land — unless the user has already
       moved on to another page. */
    window.setTimeout(() => {
      if (active.value !== i) return
      const top = el.getBoundingClientRect().top
      if (Math.abs(top) > 2) {
        window.scrollTo({ top: window.scrollY + top, behavior: 'auto' })
      }
    }, 900)
  }

  let cooldownUntil = 0
  let scrollRaf = 0

  /* Generalized scroller-walk: from the wheel target up to <html>,
     find (a) the first ancestor that can scroll in `dir` (computed
     overflow + room), and (b) whether any [data-wheel-lock-hard] zone
     sits in the chain. Body/html are the page scroller itself and never
     count as inner scrollers.
       - scrollable in dir       → native scroll, no page switch;
       - scrollable at its edge  → hard zone in chain ? absorb : flip;
       - nothing scrollable      → hard zone in chain ? absorb : flip.
     Every sub-window now behaves: the terminal or the filter popup
     absorb wheel input; an expanded page-3 card scrolls natively and
     never flips while the pointer is over it; wheel at a boundary
     cleanly switches modes. */
  function walkScroller(el: Element, dir: number): { scroller: HTMLElement | null; hard: boolean } {
    /* SVG interior elements (e.g. the performance gauge's <path>) are
       SVGElement, not HTMLElement — walk from their containing <svg> /
       nearest HTMLElement ancestor so a data-wheel-lock-hard zone on the
       card still absorbs the wheel. */
    let cur: HTMLElement | null = el instanceof HTMLElement
      ? el
      : (el.closest('svg') as HTMLElement | null)?.parentElement ?? null
    let scroller: HTMLElement | null = null
    let hard = false
    while (cur && cur !== document.body && cur !== document.documentElement) {
      if (cur.hasAttribute('data-wheel-lock-hard')) hard = true
      if (!scroller) {
        const oy = getComputedStyle(cur).overflowY
        if (oy === 'auto' || oy === 'scroll' || oy === 'overlay') {
          const room = dir > 0
            ? cur.scrollTop + cur.clientHeight < cur.scrollHeight - 1
            : cur.scrollTop > 1
          if (room) scroller = cur
        }
      }
      cur = cur.parentElement
    }
    return { scroller, hard }
  }

  function onWheel(e: WheelEvent) {
    if (!enabled()) return
    const now = performance.now()
    if (now < cooldownUntil) { e.preventDefault(); return } // inside the cooldown — one action per gesture
    const dir = e.deltaY > 0 ? 1 : e.deltaY < 0 ? -1 : 0
    if (dir === 0) return

    const target = e.target instanceof Element ? e.target : document.body
    const { scroller, hard } = walkScroller(target, dir)
    if (scroller) return // an inner scroller has room — native scroll, no switch
    if (hard) { e.preventDefault(); return } // hard zone (popup / terminal): never navigate away

    const next = active.value + dir
    if (next < 0 || next >= count) { e.preventDefault(); return } // boundary absorbs it
    e.preventDefault()
    cooldownUntil = now + COOLDOWN_MS
    goTo(next)
  }

  function onKey(e: KeyboardEvent) {
    if (!enabled()) return
    const target = e.target as Element | null
    const tag = target?.tagName
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || (target instanceof HTMLElement && target.isContentEditable)) return
    const now = performance.now()
    let next = -1
    if (e.key === 'ArrowDown' || e.key === 'PageDown' || e.key === ' ') next = active.value + 1
    else if (e.key === 'ArrowUp' || e.key === 'PageUp' || (e.key === ' ' && e.shiftKey)) next = active.value - 1
    else if (e.key === 'Home') next = 0
    else if (e.key === 'End') next = count - 1
    if (next < 0 || next >= count) return
    e.preventDefault()
    cooldownUntil = now + COOLDOWN_MS
    goTo(next)
  }

  /* ── touch: one swipe = one page, direction only (same rule as the
     wheel). Swipes starting on an inner scroller scroll it natively. ── */
  let touchStart: { x: number; y: number; el: EventTarget | null; t: number } | null = null

  function onTouchStart(e: TouchEvent) {
    if (enabled()) return // fine-pointer devices keep the wheel nav
    const t = e.touches[0]
    touchStart = { x: t.clientX, y: t.clientY, el: e.target, t: performance.now() }
  }

  function onTouchEnd(e: TouchEvent) {
    if (enabled()) return
    if (!touchStart) return
    const start = touchStart
    touchStart = null
    const t = e.changedTouches[0]
    const dy = t.clientY - start.y
    const dx = t.clientX - start.x
    if (Math.abs(dy) < 24 || Math.abs(dy) < Math.abs(dx)) return // not a clear vertical swipe
    const dir = dy < 0 ? 1 : -1
    const target = start.el instanceof Element ? start.el : document.body
    const { scroller, hard } = walkScroller(target, dir)
    if (scroller || hard) return // inner scroller handles it natively

    const now = performance.now()
    if (now < cooldownUntil) return
    const next = active.value + dir
    if (next < 0 || next >= count) return
    cooldownUntil = now + COOLDOWN_MS
    goTo(next)
  }

  /* keep `active` truthful when the user scrolls by other means */
  function onScroll() {
    if (scrollRaf) return
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0
      let best = 0
      let bestDist = Infinity
      for (let i = 0; i < count; i++) {
        const el = document.getElementById(PAGES[i])
        if (!el) continue
        const dist = Math.abs(el.getBoundingClientRect().top)
        if (dist < bestDist) { bestDist = dist; best = i }
      }
      if (best !== active.value) active.value = best
    })
  }

  onMounted(() => {
    window.addEventListener('wheel', onWheel, { passive: false })
    window.addEventListener('keydown', onKey)
    window.addEventListener('scroll', onScroll, { passive: true })
    window.addEventListener('touchstart', onTouchStart, { passive: true })
    window.addEventListener('touchend', onTouchEnd, { passive: true })
  })
  onBeforeUnmount(() => {
    window.removeEventListener('wheel', onWheel)
    window.removeEventListener('keydown', onKey)
    window.removeEventListener('scroll', onScroll)
    window.removeEventListener('touchstart', onTouchStart)
    window.removeEventListener('touchend', onTouchEnd)
    if (scrollRaf) cancelAnimationFrame(scrollRaf)
  })

  return { active, count, goTo, lastDir }
}
