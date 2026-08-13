/* composables/usePageNav.ts — PPT-style wheel navigation.
 *
 * Rules (as specified):
 *   - any wheel movement within 500 ms of the last page switch counts as
 *     ONE action and is ignored — a cooldown, like a skill in a game;
 *   - after the cooldown, one wheel gesture in either direction moves
 *     exactly one page;
 *   - it can never get stuck: at the first/last page the boundary absorbs
 *     the gesture, and any wheel-locked zone under the cursor
 *     ([data-wheel-lock]: terminal, code panel, expanded page-3 grid)
 *     scrolls natively first — the page switch only fires when that zone
 *     hits its edge. Hard zones ([data-wheel-lock-hard]: the filter
 *     popup) never trigger a page switch while the pointer is over them;
 *   - touch devices and narrow screens never hijack the wheel — they get
 *     plain native scrolling (no snap) instead.
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

  const finePointer = window.matchMedia('(hover: hover) and (pointer: fine)')
  const narrow = window.matchMedia('(max-width: 700px)')

  function enabled(): boolean {
    return finePointer.matches && !narrow.matches
  }

  function goTo(i: number) {
    if (i < 0 || i >= count) return
    const el = document.getElementById(PAGES[i])
    if (!el) return
    el.scrollIntoView({ behavior: REDUCED_MOTION ? 'auto' : 'smooth' })
    active.value = i
  }

  let cooldownUntil = 0
  let scrollRaf = 0

  function onWheel(e: WheelEvent) {
    if (!enabled()) return
    const now = performance.now()
    if (now < cooldownUntil) { e.preventDefault(); return } // inside the cooldown — one action per gesture
    const dir = e.deltaY > 0 ? 1 : e.deltaY < 0 ? -1 : 0
    if (dir === 0) return

    /* A wheel-locked zone under the cursor scrolls first — the page
       switch fires only once that zone hits its edge (can't get stuck).
       Hard zones (the filter popup) never flip the page at all. */
    const target = e.target instanceof Element ? e.target : null
    const lockEl = target?.closest<HTMLElement>('[data-wheel-lock]')
    if (lockEl) {
      const canDown = lockEl.scrollTop + lockEl.clientHeight < lockEl.scrollHeight - 1
      const canUp = lockEl.scrollTop > 1
      if ((dir > 0 && canDown) || (dir < 0 && canUp)) return // native scroll, no switch
      if (lockEl.hasAttribute('data-wheel-lock-hard')) return // hard stop: never navigate away
    }

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
  })
  onBeforeUnmount(() => {
    window.removeEventListener('wheel', onWheel)
    window.removeEventListener('keydown', onKey)
    window.removeEventListener('scroll', onScroll)
    if (scrollRaf) cancelAnimationFrame(scrollRaf)
  })

  return { active, count, goTo }
}
