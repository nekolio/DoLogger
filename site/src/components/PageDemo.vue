<script setup lang="ts">
/* PageDemo — the "before → after" migration demo.
 *
 * Narrative: the code panel shows a real-looking service whose only
 * audit trail is stdout (println! / fmt.Printf / print / fprintf). The
 * engine scans the file, focuses the offending line, deletes it, types
 * the DoLogger replacement — and the terminal underneath switches from
 * the old plain-text stream (slow, ERROR/WARN, grey) to the DoLogger
 * stream (real `[ns] [LEVEL] [component]` format, colored, all-INFO,
 * ~6× faster).
 *
 * Memory safety: the terminal holds at most MAX_TERMINAL_LINES rows
 * (oldest spliced off), every timeout is tracked and cleared on unmount,
 * and the whole engine pauses while the tab is hidden or the section is
 * off-screen.
 */
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { snippets, demoFiles } from '../demo/snippets'
import type { Snippet } from '../demo/snippets'
import { tokenize } from '../demo/tokenizer'
import { BEFORE_LOGS, AFTER_LOGS, SPEED } from '../demo/logs'
import type { DemoLog } from '../demo/logs'

const { t } = useI18n()
const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches
const FINE_POINTER = window.matchMedia('(hover: hover) and (pointer: fine)').matches

/* ── timers, tracked for cleanup; deferred (never dropped) while paused ── */
const timers = new Set<number>()
function later(fn: () => void, ms: number): number {
  const id = window.setTimeout(() => {
    timers.delete(id)
    if (paused) { timers.add(later(fn, 100)); return } // hold, don't lose, the step
    fn()
  }, ms)
  timers.add(id)
  return id
}

/* ── visibility / off-screen pausing (memory + battery) ───────────── */
let paused = false
let sectionVisible = true
function setPaused(v: boolean) {
  if (paused === v) return
  paused = v
  if (v) {
    stopTerminalLoop() // the interval stops while hidden — zero work
  } else if (wantedSpeed > 0) {
    startTerminalLoop(wantedSpeed) // resume exactly where it left off
  }
}
function onVisibility() { setPaused(document.hidden || !sectionVisible) }
function onIntersect(entries: IntersectionObserverEntry[]) {
  sectionVisible = entries[0]?.isIntersecting ?? true
  onVisibility()
}
let observer: IntersectionObserver | null = null

/* ── per-language state ───────────────────────────────────────────── */
type Phase = 'idle' | 'scrolling' | 'overshoot' | 'focusing' | 'deleting' | 'typing' | 'done'
interface LangState {
  phase: Phase
  lineIndex: number
  scanStep: number
  replaced: boolean
  typedChars: number
  timer: number | null
  lineElements: HTMLDivElement[]
}
function newState(): LangState {
  return { phase: 'idle', lineIndex: 0, scanStep: 0, replaced: false, typedChars: 0, timer: null, lineElements: [] }
}

const currentLang = ref('rust')
const langState: Record<string, LangState> = {}
for (const lang in snippets) langState[lang] = newState()

/* ── DOM refs ─────────────────────────────────────────────────────── */
const codeDisplayEl = ref<HTMLDivElement | null>(null)
const codeScrollEl = ref<HTMLDivElement | null>(null)
const codeLinesEl = ref<HTMLDivElement | null>(null)
const cursorEl = ref<HTMLSpanElement | null>(null)
const terminalOutputEl = ref<HTMLDivElement | null>(null)
const demoSection = ref<HTMLElement | null>(null)

/* ── terminal ─────────────────────────────────────────────────────── */
interface TermRow { n: number; side: 'before' | 'after'; cls: string; text: string }
const MAX_TERMINAL_LINES = 120
const terminalLines = ref<TermRow[]>([])
const side = ref<'before' | 'after'>('before')
const speed = ref(SPEED.before)
let termN = 0
let termIdx = 0
let terminalTimer: number | null = null
let terminalOn = false
let wantedSpeed = SPEED.before

const termPool: Record<'before' | 'after', DemoLog[]> = { before: BEFORE_LOGS, after: AFTER_LOGS }

function appendTermRow(row: TermRow) {
  terminalLines.value.push(row)
  const overflow = terminalLines.value.length - MAX_TERMINAL_LINES
  if (overflow > 0) terminalLines.value.splice(0, overflow) // node cap — no unbounded DOM
  const out = terminalOutputEl.value
  if (out) out.scrollTop = out.scrollHeight
}

function startTerminalLoop(ms: number) {
  stopTerminalLoop()
  wantedSpeed = ms
  side.value = ms === SPEED.after ? 'after' : 'before'
  speed.value = ms
  terminalOn = true
  terminalTimer = window.setInterval(() => {
    const pool = termPool[side.value]
    appendTermRow({ n: termN++, side: side.value, cls: pool[termIdx].cls, text: pool[termIdx].text })
    termIdx = (termIdx + 1) % pool.length
  }, ms)
}
function stopTerminalLoop() {
  if (terminalTimer !== null) { clearInterval(terminalTimer); terminalTimer = null }
  terminalOn = false
}
function clearTerminal() {
  terminalLines.value = []
  termIdx = 0
}

/* ── rendering ────────────────────────────────────────────────────── */
function renderCode(lang: string, state: LangState) {
  const s = snippets[lang]
  const html: string[] = []
  s.lines.forEach((line, idx) => {
    let display = line
    if (state.replaced && idx === s.targetLine) display = s.replaceLine
    const cls = 'line' + (idx === s.targetLine ? ' target' : '')
    if (idx === s.targetLine) {
      html.push(`<div class="${cls}" data-line="${idx}"><span class="line-text">${tokenize(display, s.lexer)}</span></div>`)
    } else {
      html.push(`<div class="${cls}" data-line="${idx}">${tokenize(display, s.lexer)}</div>`)
    }
  })
  if (codeLinesEl.value) codeLinesEl.value.innerHTML = html.join('')
  state.lineElements = Array.from(codeLinesEl.value?.querySelectorAll<HTMLDivElement>('.line') || [])
  if (codeScrollEl.value) codeScrollEl.value.style.transform = 'translateY(0px)'
}

/* The cursor lives INSIDE the target line, after its text span — it
 * always sits exactly at the end of the characters being typed/deleted
 * (the old version parked it at the panel corner instead). */
function moveCursorTo(state: LangState) {
  const el = state.lineElements[state.lineIndex] ?? state.lineElements[snippets[currentLang.value].targetLine]
  const c = cursorEl.value
  if (!el || !c) return
  c.remove()
  el.appendChild(c)
  c.style.display = 'inline-block'
}
function hideCursor() {
  const c = cursorEl.value
  if (c) c.style.display = 'none'
}

function setTargetText(state: LangState, text: string) {
  const s = snippets[currentLang.value]
  const el = state.lineElements[s.targetLine]
  const textEl = el?.querySelector('.line-text')
  if (textEl) textEl.textContent = text
}

function applyStyles(state: LangState) {
  const s = snippets[currentLang.value]
  const els = state.lineElements
  const t = s.targetLine
  els.forEach(el => el.classList.remove('highlight', 'focused', 'dimmed', 'blurred'))
  if (state.phase === 'idle' || state.phase === 'done') {
    if (state.phase === 'done' && els[t]) els[t].classList.add('highlight', 'focused')
    return
  }
  if (state.phase === 'scrolling' || state.phase === 'overshoot') {
    const idx = Math.min(state.lineIndex, els.length - 1)
    if (els[idx]) {
      els[idx].classList.add('highlight', 'focused')
      els.forEach((el, i) => { if (i !== idx) el.classList.add('dimmed') })
    }
  } else {
    if (els[t]) {
      els[t].classList.add('highlight', 'focused')
      els.forEach((el, i) => { if (i !== t) el.classList.add('blurred') })
    }
  }
}

/* ── geometry helpers (measured, not guessed) ─────────────────────── */
function panelMetrics() {
  const panel = codeDisplayEl.value
  const h = panel ? panel.clientHeight : 400
  const first = panel?.querySelector('.line')
  const lh = first ? (first as HTMLElement).offsetHeight : 22
  return { h, lh }
}
function offsetForLine(state: LangState, idx: number): number {
  const el = state.lineElements[idx]
  if (!el) return 0
  const { h } = panelMetrics()
  return -(el.offsetTop - h / 2 + el.offsetHeight / 2)
}

/* ── the state machine ────────────────────────────────────────────── */
let loopTimer: number | null = null

function scheduleLoop(lang: string) {
  if (loopTimer !== null) clearTimeout(loopTimer)
  loopTimer = later(() => {
    const state = langState[lang]
    if (state && state.phase === 'done') restartCycle(lang)
  }, 4500)
}

function restartCycle(lang: string) {
  const state = langState[lang]
  if (!state) return
  if (state.timer !== null) { clearTimeout(state.timer); state.timer = null }
  state.phase = 'idle'
  state.lineIndex = 0
  state.scanStep = 0
  state.replaced = false
  state.typedChars = 0
  renderCode(lang, state)
  hideCursor()
  // back to the "before" world: terminal resets to the old slow stream
  clearTerminal()
  startTerminalLoop(SPEED.before)
  runPhase(lang)
}

function startAnimation(lang: string) {
  const state = langState[lang]
  if (!state) return
  clearTerminal()
  startTerminalLoop(SPEED.before)
  runPhase(lang)
}

function runPhase(lang: string) {
  const state = langState[lang]
  if (!state) return
  const s = snippets[lang]
  const total = s.lines.length
  const target = s.targetLine
  const overshoot = Math.min(3, total - 1 - target)

  switch (state.phase) {
    case 'idle': {
      state.phase = 'scrolling'
      state.lineIndex = 0
      state.timer = later(() => runPhase(lang), 300)
      break
    }

    /* smooth ease-in-out scan from the top toward target+overshoot.
       scanStep drives the curve (lineIndex is the computed position, so
       it cannot double as the loop counter — the scan would stall at 0). */
    case 'scrolling': {
      const steps = 38
      const p = state.scanStep / steps
      const eased = p < 0.5 ? 2 * p * p : 1 - Math.pow(-2 * p + 2, 2) / 2
      state.lineIndex = Math.round(eased * (target + overshoot))
      const off = offsetForLine(state, state.lineIndex)
      if (codeScrollEl.value) codeScrollEl.value.style.transform = `translateY(${off}px)`
      applyStyles(state)
      state.scanStep++
      if (state.scanStep >= steps) {
        state.phase = 'overshoot'
        state.scanStep = 0
        state.timer = later(() => runPhase(lang), 60)
      } else {
        state.timer = later(() => runPhase(lang), 45)
      }
      break
    }

    /* settle back onto the target line */
    case 'overshoot': {
      state.lineIndex = Math.max(state.lineIndex - 1, target)
      const off = offsetForLine(state, state.lineIndex)
      if (codeScrollEl.value) codeScrollEl.value.style.transform = `translateY(${off}px)`
      applyStyles(state)
      if (state.lineIndex === target) {
        state.phase = 'focusing'
        state.timer = later(() => runPhase(lang), 450)
      } else {
        state.timer = later(() => runPhase(lang), 70)
      }
      break
    }

    case 'focusing': {
      applyStyles(state)
      // cursor appears at the end of the line about to be rewritten
      state.lineIndex = target
      moveCursorTo(state)
      setTargetText(state, s.lines[target])
      state.phase = 'deleting'
      state.timer = later(() => runPhase(lang), 350)
      break
    }

    case 'deleting': {
      const textEl = state.lineElements[target]?.querySelector('.line-text')
      if (textEl && textEl.textContent && textEl.textContent.length > 0) {
        textEl.textContent = textEl.textContent.slice(0, -1)
        state.timer = later(() => runPhase(lang), 16)
      } else {
        state.phase = 'typing'
        state.typedChars = 0
        state.timer = later(() => runPhase(lang), 120)
      }
      break
    }

    case 'typing': {
      const repl = s.replaceLine
      if (state.typedChars < repl.length) {
        const next = repl.slice(0, state.typedChars + 1)
        setTargetText(state, next)
        state.typedChars++
        state.timer = later(() => runPhase(lang), 24 + (Math.random() * 14 | 0))
      } else {
        finishEdit(lang)
      }
      break
    }

    case 'done': break
  }
}

function finishEdit(lang: string) {
  const state = langState[lang]
  const s = snippets[lang]
  if (!state) return
  state.replaced = true
  state.phase = 'done'
  state.typedChars = 0
  // tokenize the finished replacement so the line lights up properly
  const el = state.lineElements[s.targetLine]
  const textEl = el?.querySelector('.line-text')
  if (textEl) textEl.innerHTML = tokenize(s.replaceLine, s.lexer)
  hideCursor()
  applyStyles(state)
  // the migration is done: terminal switches to the DoLogger stream,
  // format changes and the scroll speed jumps (240 → 40 ms/line)
  clearTerminal()
  startTerminalLoop(SPEED.after)
  scheduleLoop(lang)
}

/* ── language switching ───────────────────────────────────────────── */
function switchLanguage(lang: string) {
  if (lang === currentLang.value) return
  const old = langState[currentLang.value]
  if (old) {
    if (old.timer !== null) { clearTimeout(old.timer); old.timer = null }
    old.phase = 'done' // freeze in place; a fresh cycle starts on switch
  }
  if (loopTimer !== null) { clearTimeout(loopTimer); loopTimer = null }
  currentLang.value = lang
  const state = langState[lang]
  renderCode(lang, state)
  state.phase = 'idle'
  state.replaced = false
  state.lineIndex = 0
  restartCycle(lang)
}

/* ── reduced motion: static final state, no animation loop ────────── */
function staticRender() {
  const s = snippets[currentLang.value]
  const state = langState[currentLang.value]
  state.replaced = true
  renderCode(currentLang.value, state)
  state.phase = 'done'
  applyStyles(state)
  hideCursor()
  clearTerminal()
  AFTER_LOGS.forEach(log => {
    appendTermRow({ n: termN++, side: 'after', cls: log.cls, text: log.text })
  })
}

/* ── lifecycle ────────────────────────────────────────────────────── */
function cleanup() {
  stopTerminalLoop()
  if (loopTimer !== null) clearTimeout(loopTimer)
  for (const lang in langState) {
    if (langState[lang].timer !== null) clearTimeout(langState[lang].timer)
  }
  timers.forEach(id => clearTimeout(id))
  timers.clear()
  observer?.disconnect()
  document.removeEventListener('visibilitychange', onVisibility)
}

function init() {
  renderCode('rust', langState['rust'])
  if (REDUCED_MOTION || !FINE_POINTER) {
    // touch users see the static end state; the wheel nav takes over
    staticRender()
    return
  }
  startAnimation('rust')
  observer = new IntersectionObserver(onIntersect, { threshold: 0.05 })
  if (demoSection.value) observer.observe(demoSection.value)
  document.addEventListener('visibilitychange', onVisibility)
}

onMounted(init)
onBeforeUnmount(cleanup)
</script>

<template>
  <section class="page" id="page2" ref="demoSection">
    <div class="ide-wrapper">
      <div class="ide-main">
        <div class="ide-sidebar">
          <div v-for="f in demoFiles" :key="f.lang" class="file-item"
               :class="{ active: f.lang === currentLang }"
               @click="switchLanguage(f.lang)">
            <svg class="icon"><use href="./assets/icons.svg#icon-file-code"></use></svg>
            {{ f.file }}
          </div>
        </div>
        <div class="ide-code" ref="codeDisplayEl">
          <div class="code-scroll" ref="codeScrollEl">
            <div ref="codeLinesEl"></div>
          </div>
          <span class="cursor-block" ref="cursorEl" style="display:none;"></span>
        </div>
      </div>

      <div class="ide-terminal">
        <div class="term-header">
          <span class="term-dots"><i></i><i></i><i></i></span>
          <span class="term-title">user-service — stdout</span>
          <span class="term-pill" :class="side">{{ side === 'before' ? snippets[currentLang].before : snippets[currentLang].after }}</span>
          <span class="term-speed">{{ t('demo-speed') }} ≈ {{ speed }} {{ t('demo-ms') }}</span>
        </div>
        <div class="term-body" ref="terminalOutputEl">
          <div v-for="l in terminalLines" :key="l.n" class="log-line" :class="'side-' + l.side + ' ' + l.cls">{{ l.text }}</div>
        </div>
      </div>
    </div>
  </section>
</template>
