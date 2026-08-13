<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import PageHero from './components/PageHero.vue'
import PageDemo from './components/PageDemo.vue'
import PageOverview from './components/PageOverview.vue'
import { loadSiteData } from './data'

const { locale, t } = useI18n()

/* ── theme ─────────────────────────────────────────────────────────── */
const theme = ref<'dark' | 'light'>('dark')
try {
  const saved = localStorage.getItem('dologger:theme')
  if (saved === 'light' || saved === 'dark') theme.value = saved
} catch { /* private mode */ }

function applyTheme(t: 'dark' | 'light') {
  document.documentElement.setAttribute('data-theme', t)
  // favicon switches with the theme: dark chip in dark mode, light chip
  // (and darker gradient) in light mode. Browsers cannot animate tab
  // icons, so the "shine" is baked into each static variant.
  const icon = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
  if (icon) icon.setAttribute('href', './assets/favicon' + (t === 'light' ? '-light' : '') + '.svg')
}
applyTheme(theme.value)
watch(theme, (t) => {
  applyTheme(t)
  try { localStorage.setItem('dologger:theme', t) } catch { /* private mode */ }
})

/* ── language ──────────────────────────────────────────────────────── */
let savedLang = (navigator.language || '').toLowerCase().startsWith('zh') ? 'zh' : 'en'
try {
  savedLang = localStorage.getItem('dologger:lang') || savedLang
} catch { /* private mode */ }
locale.value = savedLang === 'zh' ? 'zh' : 'en'
watch(locale, (l) => {
  document.documentElement.lang = l
  try { localStorage.setItem('dologger:lang', l) } catch { /* private mode */ }
})

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
      </div>
      <span class="divider"></span>
      <div class="group">
        <span class="group-label">{{ t('lang-label') }}</span>
        <button class="btn-small" :class="{ active: locale === 'zh' }" title="中文" @click="locale = 'zh'">中</button>
        <button class="btn-small" :class="{ active: locale === 'en' }" title="English" @click="locale = 'en'">En</button>
      </div>
    </div>

    <PageHero />
    <PageDemo />
    <PageOverview />
  </div>
</template>
