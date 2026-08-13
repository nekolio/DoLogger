<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import PageHero from './components/PageHero.vue'
import PageDemo from './components/PageDemo.vue'
import PageOverview from './components/PageOverview.vue'
import PageNav from './components/PageNav.vue'
import CyberCursor from './components/CyberCursor.vue'
import { loadSiteData } from './data'
import { usePageNav } from './composables/usePageNav'
import { useCursorEnabled, setCursorEnabled } from './cursor'

const { active: pageIndex, count: pageCount, goTo } = usePageNav()

const { locale, t } = useI18n()

/* ── theme: user choice (dark/light), else follow the OS live ─────── */
type Theme = 'dark' | 'light' | 'system'
const theme = ref<Theme>('system') // first visit: match the system
try {
  const saved = localStorage.getItem('dologger:theme')
  if (saved === 'dark' || saved === 'light' || saved === 'system') theme.value = saved
} catch { /* private mode */ }

const systemDark = window.matchMedia('(prefers-color-scheme: dark)')
function resolveTheme(): 'dark' | 'light' {
  return theme.value === 'system' ? (systemDark.matches ? 'dark' : 'light') : theme.value
}
function applyTheme() {
  const t = resolveTheme()
  document.documentElement.setAttribute('data-theme', t)
  // favicon switches with the theme: dark chip in dark mode, light chip
  // (and darker gradient) in light mode. Browsers cannot animate tab
  // icons, so the "shine" is baked into each static variant.
  const icon = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
  if (icon) icon.setAttribute('href', './assets/favicon' + (t === 'light' ? '-light' : '') + '.svg')
}
applyTheme()
watch(theme, (v) => {
  applyTheme()
  try { localStorage.setItem('dologger:theme', v) } catch { /* private mode */ }
})
// in "system" mode the page follows OS light/dark changes live
const onSysTheme = () => { if (theme.value === 'system') applyTheme() }
systemDark.addEventListener('change', onSysTheme)
onBeforeUnmount(() => systemDark.removeEventListener('change', onSysTheme))

/* ── language: system detection on first visit, manual override after */
function detectLang(): 'zh' | 'en' {
  const signals = [
    ...((navigator as Navigator & { languages?: readonly string[] }).languages || []),
    navigator.language
  ]
  let localeName = ''
  try { localeName = Intl.DateTimeFormat().resolvedOptions().locale } catch { /* old browsers */ }
  signals.push(localeName)
  for (const s of signals) {
    if (s && s.toLowerCase().startsWith('zh')) return 'zh'
  }
  // timezone fallback — China-adjacent regions without zh locales
  try {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone || ''
    if (/^(Asia\/(Shanghai|Taipei|Hong_Kong|Macau|Urumqi)|Etc\/GMT[+-]8)/i.test(tz)) return 'zh'
  } catch { /* old browsers */ }
  return 'en'
}
let savedLang: string = detectLang()
try {
  savedLang = localStorage.getItem('dologger:lang') || savedLang
} catch { /* private mode */ }
locale.value = savedLang === 'zh' ? 'zh' : 'en'
document.documentElement.lang = locale.value // the watch below only fires on change
watch(locale, (l) => {
  document.documentElement.lang = l
  try { localStorage.setItem('dologger:lang', l) } catch { /* private mode */ }
})

/* ── cursor style toggle (cyber cursor ↔ native pointer) ──────────── */
const cursorOn = useCursorEnabled()
function toggleCursor() { setCursorEnabled(!cursorOn.value) }

onMounted(() => {
  loadSiteData()
})
</script>

<template>
  <div id="app">
    <div class="top-controls">
      <div class="group">
        <span class="group-label">{{ t('theme-label') }}</span>
        <button class="btn-small" :class="{ active: theme === 'dark' }" title="Dark" @click="theme = 'dark'">
          <svg class="icon"><use href="./assets/icons.svg#icon-moon"></use></svg>
        </button>
        <button class="btn-small" :class="{ active: theme === 'light' }" title="Light" @click="theme = 'light'">
          <svg class="icon"><use href="./assets/icons.svg#icon-sun"></use></svg>
        </button>
        <button class="btn-small" :class="{ active: theme === 'system' }" :title="t('theme-system')" @click="theme = 'system'">
          <svg class="icon"><use href="./assets/icons.svg#icon-monitor"></use></svg>
        </button>
      </div>
      <span class="divider"></span>
      <div class="group">
        <span class="group-label">{{ t('lang-label') }}</span>
        <button class="btn-small" :class="{ active: locale === 'zh' }" title="中文" @click="locale = 'zh'">中</button>
        <button class="btn-small" :class="{ active: locale === 'en' }" title="English" @click="locale = 'en'">En</button>
      </div>
      <span class="divider"></span>
      <button class="btn-small cursor-toggle" :class="{ active: cursorOn }"
              :title="cursorOn ? t('cursor-cyber') : t('cursor-native')"
              :aria-pressed="cursorOn ? 'true' : 'false'" @click="toggleCursor">
        <svg class="icon"><use href="./assets/icons.svg#icon-mouse"></use></svg>
      </button>
    </div>

    <PageHero />
    <PageDemo />
    <PageOverview />

    <PageNav :count="pageCount" :active="pageIndex" @go="goTo" />
    <CyberCursor />
  </div>
</template>
