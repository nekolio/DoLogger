<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'

/* Live demo — real adapter APIs (adapters/rust, adapters/go,
 * adapters/python, core C ABI) with the real internal-log format
 * `[ns] [LEVEL] [component]` and the engine's actual boot sequence. */
const { t } = useI18n()
const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches

interface HighlightRule { regex: RegExp; class: string }
interface Snippet {
  lang: string
  targetLine: number
  lines: string[]
  replaceLine: string
  highlightRules: HighlightRule[]
}
interface LogEntry { level: string; comp: string; msg: string }
type Phase = 'idle' | 'scrolling' | 'focusing' | 'deleting' | 'typing' | 'done'
interface LangState {
  phase: Phase
  progress: number
  lineIndex: number
  targetLine: number
  replaced: boolean
  typedChars: number
  timer: number | null
  paused: boolean
  scrollSteps: number
  maxScrollSteps: number
  lineElements: HTMLDivElement[]
}

const files: { lang: string; file: string }[] = [
  { lang: 'rust', file: 'main.rs' },
  { lang: 'go', file: 'main.go' },
  { lang: 'python', file: 'main.py' },
  { lang: 'c', file: 'main.c' }
]

const snippets: Record<string, Snippet> = {
  rust: {
    lang: 'Rust',
    targetLine: 6,
    lines: [
      'use dologger_sdk::Logger;',
      '',
      'fn main() {',
      '    let mut logger = Logger::init(None).expect("init"); // default config',
      '    logger.info("Application started");',
      '    logger.warn("Disk usage at 85%");',
      '    println!("user 42 deleted record #7");',
      '    logger.shutdown();',
      '}'
    ],
    replaceLine: '    logger.audit("user 42 deleted record #7"); // Ed25519-signed + WORM',
    highlightRules: [
      { regex: /\b(fn|let|mut|use|Result|Ok)\b/g, class: 'kw' },
      { regex: /"([^"]*)"/g, class: 'str' },
      { regex: /\/\/.*$/g, class: 'com' },
      { regex: /\bprintln!/g, class: 'stdout' },
      { regex: /\b(Logger|dologger_sdk)\b/g, class: 'dologger' },
      { regex: /\b(main|init|expect|None|shutdown)\b/g, class: 'fn' },
      { regex: /\b(Info|Warn|Error|Debug|Trace|Fatal|Audit)\b/g, class: 'type' }
    ]
  },
  go: {
    lang: 'Go',
    targetLine: 17,
    lines: [
      'package main',
      '',
      'import (',
      '    "fmt"',
      '',
      '    "github.com/dologger/adapters/go/dologger"',
      ')',
      '',
      'func main() {',
      '    log, err := dologger.NewLogger("")',
      '    if err != nil {',
      '        panic(err)',
      '    }',
      '    defer log.Shutdown()',
      '',
      '    log.Info("Hello from Go")',
      '    log.Warn("Disk usage at 85%")',
      '    fmt.Println("user 42 deleted record #7")',
      '}'
    ],
    replaceLine: '    log.Audit("user 42 deleted record #7")',
    highlightRules: [
      { regex: /\b(package|import|func|if|err|nil|panic|defer|var)\b/g, class: 'kw' },
      { regex: /"([^"]*)"/g, class: 'str' },
      { regex: /\/\/.*$/g, class: 'com' },
      { regex: /\bfmt\.Println\b/g, class: 'stdout' },
      { regex: /\b(dologger|NewLogger)\b/g, class: 'dologger' },
      { regex: /\bmain\b/g, class: 'fn' },
      { regex: /\b(Info|Warn|Error|Debug|Trace|Fatal|Audit|Shutdown)\b/g, class: 'type' }
    ]
  },
  python: {
    lang: 'Python',
    targetLine: 5,
    lines: [
      'from dologger import DoLogger',
      '',
      'log = DoLogger()  # auto-discovers config',
      'log.info("Hello from Python")',
      'log.warn("Disk usage at 85%")',
      'print("user 42 deleted record #7")',
      'log.shutdown()'
    ],
    replaceLine: 'log.audit("user 42 deleted record #7")',
    highlightRules: [
      { regex: /\b(from|import|def|if|return|for|in|range)\b/g, class: 'kw' },
      { regex: /"([^"]*)"/g, class: 'str' },
      { regex: /#.*$/g, class: 'com' },
      { regex: /\bprint\b/g, class: 'stdout' },
      { regex: /\b(DoLogger|dologger)\b/g, class: 'dologger' },
      { regex: /\b(trace|debug|info|warn|error|fatal|audit|shutdown)\b/g, class: 'type' }
    ]
  },
  c: {
    lang: 'C',
    targetLine: 10,
    lines: [
      '#include <stdio.h>',
      '#include "dologger_core.h"',
      '',
      'int main(void) {',
      '    dologger_error_t err;',
      '    dologger_handle_t *h = dologger_init(NULL, &err);',
      '    if (!h) return 1;',
      '',
      '    dologger_record_params_t p = {0};',
      '    p.level = DO_LOG_AUDIT;',
      '    p.message = "Hello from C";',
      '    dologger_log(h, &p);',
      '',
      '    dologger_shutdown(h);',
      '    return 0;',
      '}'
    ],
    replaceLine: '    p.message = "user 42 deleted record #7";',
    highlightRules: [
      { regex: /\b(include|int|void|if|return|NULL)\b/g, class: 'kw' },
      { regex: /"([^"]*)"/g, class: 'str' },
      { regex: /\/\/.*$/g, class: 'com' },
      { regex: /\bdologger_[a-z_]+\b/g, class: 'dologger' },
      { regex: /\b(main|dologger_init|dologger_log|dologger_shutdown)\b/g, class: 'fn' },
      { regex: /\b(DO_LOG_[A-Z_]+|dologger_error_t|dologger_handle_t|dologger_record_params_t)\b/g, class: 'type' }
    ]
  }
}

const logMessages: LogEntry[] = [
  { level: 'info',  comp: 'core',     msg: "Internal diagnostic log started at './dologger_internal.log'" },
  { level: 'info',  comp: 'core',     msg: 'Hello DoLogger' },
  { level: 'info',  comp: 'plugin',   msg: '4 sandboxed plugins loaded · trust BLUE' },
  { level: 'audit', comp: 'audit',    msg: 'ed25519 chain armed' },
  { level: 'info',  comp: 'pipeline', msg: '7-stage pipeline online' },
  { level: 'info',  comp: 'plugin',   msg: 'fmt_text mounted (phase: formatting)' },
  { level: 'info',  comp: 'pipeline', msg: 'Sink fan-out: console, file, worm' },
  { level: 'audit', comp: 'audit',    msg: 'user 42 deleted record #7' },
  { level: 'info',  comp: 'pipeline', msg: 'Shutdown: flushed 2 remaining records' },
  { level: 'info',  comp: 'core',     msg: 'DoLogger engine shutdown complete' }
]

/* ── DOM refs (the engine mutates these imperatively) ─────────────── */
const codeLinesEl = ref<HTMLDivElement | null>(null)
const codeScrollEl = ref<HTMLDivElement | null>(null)
const codeDisplayEl = ref<HTMLDivElement | null>(null)
const cursorBlockEl = ref<HTMLSpanElement | null>(null)
const terminalOutputEl = ref<HTMLDivElement | null>(null)

const currentLang = ref('rust')
const langState: Record<string, LangState> = {}
for (const lang in snippets) {
  langState[lang] = {
    phase: 'idle', progress: 0, lineIndex: 0,
    targetLine: snippets[lang].targetLine, replaced: false,
    typedChars: 0, timer: null, paused: false,
    scrollSteps: 0, maxScrollSteps: 22, lineElements: []
  }
}

/* ── timers (tracked for cleanup) ─────────────────────────────────── */
const timers = new Set<number>()
function later(fn: () => void, ms: number): number {
  const id = window.setTimeout(() => { timers.delete(id); fn() }, ms)
  timers.add(id)
  return id
}

function highlightLine(line: string, rules: HighlightRule[]): string {
  if (!line) return ''
  let html = line
  rules.forEach(rule => {
    html = html.replace(rule.regex, match => `<span class="${rule.class}">${match}</span>`)
  })
  return html
}

function renderCode(lang: string, state: LangState) {
  const snippet = snippets[lang]
  const targetIdx = snippet.targetLine
  let html = ''
  snippet.lines.forEach((line, idx) => {
    let displayLine = line
    if (state.replaced && idx === targetIdx) displayLine = snippet.replaceLine
    const lineHtml = highlightLine(displayLine, snippet.highlightRules)
    const classes = 'line' + (idx === targetIdx ? ' target' : '')
    html += `<div class="${classes}" data-line="${idx}">${lineHtml}</div>`
  })
  if (codeLinesEl.value) codeLinesEl.value.innerHTML = html
  state.lineElements = Array.from(codeLinesEl.value?.querySelectorAll<HTMLDivElement>('.line') || [])
  if (codeScrollEl.value) codeScrollEl.value.style.transform = 'translateY(0px)'
  applyStyles(state)
}

function applyStyles(state: LangState) {
  const elements = state.lineElements
  const targetIdx = state.targetLine
  elements.forEach(el => el.classList.remove('highlight', 'focused', 'dimmed', 'blurred'))
  if (state.phase === 'idle' || state.phase === 'done') {
    if (state.phase === 'done' && elements[targetIdx]) elements[targetIdx].classList.add('highlight', 'focused')
    return
  }
  if (state.phase === 'scrolling') {
    const idx = Math.min(state.lineIndex, elements.length - 1)
    if (elements[idx]) {
      elements[idx].classList.add('highlight', 'focused')
      elements.forEach((el, i) => { if (i !== idx) el.classList.add('dimmed') })
    }
  } else {
    if (elements[targetIdx]) {
      elements[targetIdx].classList.add('highlight', 'focused')
      elements.forEach((el, i) => { if (i !== targetIdx) el.classList.add('blurred') })
    }
  }
}

/* ── terminal ─────────────────────────────────────────────────────── */
interface TerminalLine { n: number; level: string; comp: string; msg: string }
const terminalLines = ref<TerminalLine[]>([])
let terminalLogIndex = 0
let terminalInterval: number | null = null
let terminalSpeed = 200

function appendLogLine(entry: LogEntry) {
  terminalLines.value.push({
    n: terminalLogIndex++, level: entry.level, comp: entry.comp, msg: entry.msg
  })
}

function startTerminalLogs(speed: number) {
  if (terminalInterval !== null) clearInterval(terminalInterval)
  terminalSpeed = speed || 200
  if (terminalLines.value.length === 0) terminalLines.value = []
  terminalInterval = window.setInterval(() => {
    if (terminalLogIndex >= logMessages.length) terminalLogIndex = 0
    appendLogLine(logMessages[terminalLogIndex])
    terminalLogIndex++
    const out = terminalOutputEl.value
    if (out) out.scrollTop = out.scrollHeight
  }, terminalSpeed)
}

function scrollTerminalToBottom() {
  const out = terminalOutputEl.value
  if (out) out.scrollTop = out.scrollHeight
}

function staticRender() {
  const snippet = snippets[currentLang.value]
  let html = ''
  snippet.lines.forEach((line, idx) => {
    if (idx === snippet.targetLine) line = snippet.replaceLine
    html += `<div class="line highlight focused">${highlightLine(line, snippet.highlightRules)}</div>`
  })
  if (codeLinesEl.value) codeLinesEl.value.innerHTML = html
  if (cursorBlockEl.value) cursorBlockEl.value.style.display = 'none'
  terminalLines.value = []
  logMessages.forEach(entry => appendLogLine(entry))
}

/* ── the animation state machine ──────────────────────────────────── */
let loopTimer: number | null = null
function scheduleLoop(lang: string) {
  if (loopTimer !== null) clearTimeout(loopTimer)
  loopTimer = later(() => {
    const state = langState[lang]
    if (state && (state.phase === 'done' || state.phase === 'idle')) {
      resetAnimation(lang)
      startAnimation(lang)
    }
  }, 3000)
}

function startAnimation(lang: string) {
  const state = langState[lang]
  if (!state) return
  if (state.timer !== null) clearTimeout(state.timer)
  state.paused = false
  if (state.phase === 'done' || state.phase === 'idle') {
    state.phase = 'idle'
    state.progress = 0
    state.replaced = false
    state.typedChars = 0
    renderCode(lang, state)
    if (cursorBlockEl.value) cursorBlockEl.value.style.display = 'none'
    if (codeScrollEl.value) codeScrollEl.value.style.transform = 'translateY(0px)'
    terminalSpeed = 200
    if (terminalInterval === null) startTerminalLogs(200)
  }
  runPhase(lang)
}

function resetAnimation(lang: string) {
  const state = langState[lang]
  if (!state) return
  if (state.timer !== null) clearTimeout(state.timer)
  state.phase = 'idle'
  state.progress = 0
  state.replaced = false
  state.typedChars = 0
  state.paused = false
  renderCode(lang, state)
  if (cursorBlockEl.value) cursorBlockEl.value.style.display = 'none'
  if (codeScrollEl.value) codeScrollEl.value.style.transform = 'translateY(0px)'
  terminalSpeed = 200
  if (terminalInterval !== null) {
    clearInterval(terminalInterval)
    terminalInterval = null
    startTerminalLogs(200)
  }
}

function runPhase(lang: string) {
  const state = langState[lang]
  if (!state || state.paused) return
  const snippet = snippets[lang]
  const totalLines = snippet.lines.length
  const targetIdx = snippet.targetLine

  switch (state.phase) {
    case 'idle':
      state.phase = 'scrolling'
      state.progress = 0
      state.lineIndex = 0
      state.scrollSteps = 0
      state.maxScrollSteps = 22
      runPhase(lang)
      break

    case 'scrolling': {
      state.scrollSteps++
      const lineHeight = 1.6 * 16
      const containerHeight = codeDisplayEl.value?.clientHeight || 400
      if (state.scrollSteps >= state.maxScrollSteps) {
        state.lineIndex = targetIdx
        const offset = -(targetIdx * lineHeight - containerHeight / 2 + lineHeight / 2)
        if (codeScrollEl.value) codeScrollEl.value.style.transform = `translateY(${offset}px)`
        state.phase = 'focusing'
        runPhase(lang)
      } else {
        const progress = state.scrollSteps / state.maxScrollSteps
        let idx: number
        const center = targetIdx
        if (progress < 0.5) {
          const p = progress / 0.5
          idx = Math.round(center - 15 + p * 25)
        } else if (progress < 0.75) {
          const p = (progress - 0.5) / 0.25
          idx = Math.round(center + 10 - p * 18)
        } else {
          const p = (progress - 0.75) / 0.25
          idx = Math.round(center - 8 + p * 11)
        }
        state.lineIndex = Math.min(Math.max(idx, 0), totalLines - 1)
        const offset = -(state.lineIndex * lineHeight - containerHeight / 2 + lineHeight / 2)
        if (codeScrollEl.value) codeScrollEl.value.style.transform = `translateY(${offset}px)`
        applyStyles(state)
        state.timer = later(() => runPhase(lang), 80)
      }
      break
    }

    case 'focusing':
      applyStyles(state)
      state.timer = later(() => {
        state.phase = 'deleting'
        state.typedChars = 0
        if (cursorBlockEl.value) cursorBlockEl.value.style.display = 'inline-block'
        runPhase(lang)
      }, 400)
      break

    case 'deleting': {
      const targetEl = state.lineElements[targetIdx]
      if (targetEl) {
        const text = targetEl.textContent || ''
        if (text.length > 0) {
          targetEl.textContent = text.slice(0, -1)
          state.timer = later(() => runPhase(lang), 15)
        } else {
          state.phase = 'typing'
          state.typedChars = 0
          runPhase(lang)
        }
      } else {
        state.phase = 'typing'
        runPhase(lang)
      }
      break
    }

    case 'typing': {
      const targetEl = state.lineElements[targetIdx]
      if (targetEl) {
        const replaceLine = snippet.replaceLine
        if (state.typedChars < replaceLine.length) {
          targetEl.textContent += replaceLine.charAt(state.typedChars)
          state.typedChars++
          state.timer = later(() => runPhase(lang), 20 + Math.random() * 15)
        } else {
          state.replaced = true
          state.phase = 'done'
          targetEl.innerHTML = highlightLine(snippet.replaceLine, snippet.highlightRules)
          if (cursorBlockEl.value) cursorBlockEl.value.style.display = 'none'
          const elements = state.lineElements
          elements.forEach(el => el.classList.remove('blurred', 'dimmed'))
          if (elements[targetIdx]) elements[targetIdx].classList.add('highlight', 'focused')
          terminalSpeed = 30
          if (terminalInterval !== null) {
            clearInterval(terminalInterval)
            terminalInterval = null
            startTerminalLogs(30)
          }
          state.timer = null
          scheduleLoop(lang)
        }
      } else {
        state.phase = 'done'
        state.timer = null
        scheduleLoop(lang)
      }
      break
    }
    case 'done': break
  }
}

function switchLanguage(lang: string) {
  if (lang === currentLang.value) {
    const state = langState[lang]
    if (state && state.paused) {
      state.paused = false
      runPhase(lang)
    }
    return
  }
  const oldState = langState[currentLang.value]
  if (oldState) {
    oldState.paused = true
    if (oldState.timer !== null) { clearTimeout(oldState.timer); oldState.timer = null }
  }
  currentLang.value = lang
  const newState = langState[lang]
  if (!newState) return
  if (loopTimer !== null) { clearTimeout(loopTimer); loopTimer = null }
  if (newState.phase === 'done' || newState.phase === 'idle') {
    resetAnimation(lang)
    startAnimation(lang)
  } else {
    newState.paused = false
    renderCode(lang, newState)
    applyStyles(newState)
    runPhase(lang)
  }
}

function cleanup() {
  if (terminalInterval !== null) { clearInterval(terminalInterval); terminalInterval = null }
  if (loopTimer !== null) clearTimeout(loopTimer)
  for (const lang in langState) {
    if (langState[lang].timer !== null) clearTimeout(langState[lang].timer)
  }
  timers.forEach(id => clearTimeout(id))
  timers.clear()
}

function init() {
  if (REDUCED_MOTION) {
    staticRender()
    scrollTerminalToBottom()
    return
  }
  const state = langState['rust']
  renderCode('rust', state)
  startAnimation('rust')
  startTerminalLogs(200)
}

onMounted(init)
onBeforeUnmount(cleanup)
</script>

<template>
  <section class="page" id="page2">
    <div class="ide-wrapper">
      <div class="ide-main">
        <div class="ide-sidebar">
          <div v-for="f in files" :key="f.lang" class="file-item"
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
          <span class="cursor-block" ref="cursorBlockEl" style="display:none;"></span>
        </div>
      </div>
      <div class="ide-terminal" ref="terminalOutputEl">
        <div v-if="terminalLines.length === 0" style="opacity:0.3; padding:0.3rem;">{{ t('demo-waiting') }}</div>
        <div v-for="l in terminalLines" :key="l.n" class="log-line">
          <span class="log-time">[{{ l.n }}]</span>
          <span class="log-level" :class="'lv-' + l.level">[{{ l.level.toUpperCase() }}]</span>
          <span class="log-comp">[{{ l.comp }}]</span>{{ l.msg }}
        </div>
      </div>
    </div>
  </section>
</template>
