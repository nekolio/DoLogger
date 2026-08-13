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

/* "All platforms & checksums" list — real asset names from the release. */
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
        <a :href="RELEASES_URL" class="btn btn-outline">
          <svg class="icon"><use href="./assets/icons.svg#icon-ellipsis"></use></svg>
          {{ t('more-versions') }}
        </a>
        <a :href="docsUrl" class="btn btn-outline">
          <svg class="icon"><use href="./assets/icons.svg#icon-book"></use></svg>
          {{ t('docs') }}
        </a>
        <a :href="REPO_URL" class="btn btn-outline">
          <svg class="icon"><use href="./assets/icons.svg#icon-star"></use></svg>
          {{ t('star') }}
        </a>
      </div>

      <details class="other-platforms">
        <summary>
          <svg class="icon"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
          {{ t('other-platforms') }}
        </summary>
        <ul>
          <li v-for="a in assetList" :key="a.name">
            <span class="tag-kind">{{ a.kind }}</span><a :href="a.url">{{ a.name }}</a>
          </li>
        </ul>
      </details>
    </div>

    <div class="scroll-hint" @click="scrollToDemo">
      <svg class="icon"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
      <span class="text">{{ hintText }}</span>
      <span class="cursor"></span>
    </div>
  </section>
</template>
