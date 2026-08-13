<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSiteData, type ReleaseAsset } from '../data'

const { t, locale } = useI18n()
const siteData = useSiteData()

const REPO_URL = 'https://github.com/Nekolio/DoLogger'
const WIKI_URL = REPO_URL + '/wiki'
const RELEASES_URL = REPO_URL + '/releases'
const OS_KEYS: Record<string, string> = { windows: 'os-windows', macos: 'os-macos', linux: 'os-linux' }
const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches

const version = computed(() => siteData.value?.latest?.tag_name || 'v0.1.0')
const downloadUrl = computed(() => siteData.value?.downloadUrl || RELEASES_URL)
const osLabel = computed(() => {
  const d = siteData.value
  const osKey = (d && OS_KEYS[d.platform.os]) || 'os-linux'
  return t(osKey) + ' (' + ((d && d.platform.arch) || 'x86_64') + ')'
})
const docsUrl = computed(() =>
  locale.value === 'zh' ? WIKI_URL + '/Chinese-Home' : WIKI_URL + '/Home'
)

/* ── merged "all platforms · checksums · versions" panel ──────────── */
const open = ref(REDUCED_MOTION) // reduced-motion users get the list pre-opened

/* Real asset names from the release, tagged by kind. */
function classifyAsset(name: string): string {
  if (name.indexOf('dologctl') === 0) return 'CLI'
  if (name.indexOf('libdologger_core') === 0 || name.indexOf('dologger_core-') === 0) return 'LIB'
  if (name.indexOf('benchmark-results') === 0) return 'BENCH'
  return 'CHK'
}
const assetList = computed(() => {
  const assets: ReleaseAsset[] = siteData.value?.latest?.assets?.length
    ? siteData.value.latest.assets
    : []
  return assets.map(a => ({ name: a.name, url: a.browser_download_url, kind: classifyAsset(a.name) }))
})

/* Version list for the same panel (from the API/baked data). */
const releases = computed(() => siteData.value?.releases ?? [])
function fmtDate(iso: string): string {
  const d = new Date(iso)
  return isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10)
}

/* ── scroll-hint typewriter ───────────────────────────────────────── */
const hintText = ref('')
let typeTimer: number | null = null
const hintState = { char: 0, deleting: false }

function stopHintTyping() {
  if (typeTimer !== null) { clearTimeout(typeTimer); typeTimer = null }
}
function restartHintTyping() {
  stopHintTyping()
  hintText.value = ''
  hintState.char = 0
  hintState.deleting = false
  if (REDUCED_MOTION) {
    hintText.value = t('hint-1')
    return
  }
  typeLoop()
}
function typeLoop() {
  const current = t('hint-1')
  if (!hintState.deleting) {
    hintText.value = current.substring(0, hintState.char + 1)
    hintState.char++
    if (hintState.char === current.length) {
      hintState.deleting = true
      typeTimer = window.setTimeout(typeLoop, 2000)
      return
    }
    typeTimer = window.setTimeout(typeLoop, 80)
  } else {
    hintText.value = current.substring(0, hintState.char - 1)
    hintState.char--
    if (hintState.char === 0) {
      hintState.deleting = false
      typeTimer = window.setTimeout(typeLoop, 400)
      return
    }
    typeTimer = window.setTimeout(typeLoop, 40)
  }
}

function scrollToDemo() {
  document.getElementById('page2')?.scrollIntoView({ behavior: 'smooth' })
}

watch(locale, () => restartHintTyping())
onMounted(restartHintTyping)
onBeforeUnmount(stopHintTyping)
</script>

<template>
  <section class="page" id="page1">
    <div class="hero-content">
      <div class="hero-frame">
        <img class="hero" src="./assets/hero.svg"
             alt="DoLogger boot sequence — Hello DoLogger, 4 sandboxed plugins, Ed25519 chain armed, 7-stage pipeline online" />
      </div>

      <div class="badge">
        <svg class="icon"><use href="./assets/icons.svg#icon-rocket"></use></svg>
        <span>{{ version }}</span>
      </div>

      <div class="tags">
        <span><svg class="icon"><use href="./assets/icons.svg#icon-zap"></use></svg> {{ t('tag-zero-copy') }}</span>
        <span><svg class="icon"><use href="./assets/icons.svg#icon-shield"></use></svg> {{ t('tag-audit') }}</span>
        <span><svg class="icon"><use href="./assets/icons.svg#icon-layers"></use></svg> {{ t('tag-plugin') }}</span>
        <span><svg class="icon"><use href="./assets/icons.svg#icon-cloud"></use></svg> {{ t('tag-sinks') }}</span>
      </div>

      <div class="actions">
        <a :href="downloadUrl" class="btn btn-primary">
          <svg class="icon"><use href="./assets/icons.svg#icon-download"></use></svg>
          {{ t('download') }} <span>{{ osLabel }}</span>
        </a>
        <button class="btn btn-outline" type="button" @click="open = !open">
          <svg class="icon"><use href="./assets/icons.svg#icon-layers"></use></svg>
          {{ t('panel-title') }}
          <svg class="icon chev" :class="{ open }"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
        </button>
        <a :href="docsUrl" class="btn btn-outline">
          <svg class="icon"><use href="./assets/icons.svg#icon-book"></use></svg>
          {{ t('docs') }}
        </a>
        <a :href="REPO_URL" class="btn btn-outline">
          <svg class="icon"><use href="./assets/icons.svg#icon-star"></use></svg>
          {{ t('star') }}
        </a>
      </div>

      <div class="panel" :class="{ open }" :aria-hidden="!open">
        <!-- the curved energy line that draws itself when the panel opens -->
        <svg class="panel-curve" viewBox="0 0 400 26" preserveAspectRatio="none" aria-hidden="true">
          <defs>
            <linearGradient id="panel-grad" x1="0" y1="0" x2="1" y2="0">
              <stop offset="0" stop-color="#7FD5FF" />
              <stop offset="0.5" stop-color="#C792EA" />
              <stop offset="1" stop-color="#F472D0" />
            </linearGradient>
          </defs>
          <path class="curve-base" d="M 0 26 C 110 2, 260 24, 400 8" pathLength="1" />
          <path class="curve-flow" d="M 0 26 C 110 2, 260 24, 400 8" pathLength="1" />
        </svg>

        <div class="panel-inner">
          <div class="panel-col">
            <h4>{{ t('panel-assets') }}</h4>
            <ul class="asset-list">
              <li v-for="a in assetList" :key="a.name">
                <span class="tag-kind">{{ a.kind }}</span><a :href="a.url">{{ a.name }}</a>
              </li>
            </ul>
          </div>
          <div class="panel-col">
            <h4>{{ t('panel-versions') }}</h4>
            <ul class="version-list">
              <li v-for="r in releases.slice(0, 5)" :key="r.tag_name">
                <a :href="r.html_url || RELEASES_URL">{{ r.tag_name }}</a>
                <span v-if="r.prerelease" class="prerelease-badge">{{ t('rel-prerelease') }}</span>
                <span class="date">{{ fmtDate(r.published_at) }}</span>
              </li>
              <li class="view-all"><a :href="RELEASES_URL">{{ t('view-all-releases') }}</a></li>
            </ul>
          </div>
        </div>
      </div>
    </div>

    <div class="scroll-hint" @click="scrollToDemo">
      <svg class="icon"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
      <span class="text">{{ hintText }}</span>
      <span class="cursor"></span>
    </div>
  </section>
</template>
