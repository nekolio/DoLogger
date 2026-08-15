<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSiteData, pickRelease, assetFor, type Platform } from '../data'
import FilterPopup from './FilterPopup.vue'

const { t, locale } = useI18n()
const siteData = useSiteData()

const REPO_URL = 'https://github.com/Nekolio/DoLogger'
const WIKI_URL = REPO_URL + '/wiki'
const RELEASES_URL = REPO_URL + '/releases'
const OS_KEYS: Record<string, string> = { windows: 'os-windows', macos: 'os-macos', linux: 'os-linux' }
const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches

/* Everything derives from the SELECTED release (default: latest), so
   picking a single version in the filter popup flips the download
   button and the os label at once. */
const releases = computed(() => siteData.value?.releases ?? [])
const selectedRelease = computed(() => pickRelease(releases.value.length ? releases.value : null))
const platform = computed<Platform>(() => siteData.value?.platform ?? { os: 'linux', arch: 'x86_64' })
/* The hero badge shows the latest NON-prerelease release; while every
   release is still a prerelease (as now) it falls back to the latest
   and marks it. Independent of the popup's version filter. */
const badgeRelease = computed(() => releases.value.find(r => !r.prerelease) ?? releases.value[0] ?? null)
const downloadUrl = computed(() => {
  const hit = assetFor(selectedRelease.value, platform.value.os, platform.value.arch)
  return (hit && hit.browser_download_url) || selectedRelease.value.html_url || RELEASES_URL
})
const osLabel = computed(() => {
  const osKey = OS_KEYS[platform.value.os] || 'os-linux'
  return t(osKey) + ' (' + platform.value.arch + ')'
})
const docsUrl = computed(() =>
  locale.value === 'zh' ? WIKI_URL + '/Chinese-Home' : WIKI_URL + '/Home'
)

/* ── filter popup — opened directly from the actions row (no panel) ─ */
const fOpen = ref(false)
const filterAnchorEl = ref<HTMLElement | null>(null)
const popupRef = ref<InstanceType<typeof FilterPopup> | null>(null)
const filterSummary = computed(() => popupRef.value?.summary ?? '')

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
             alt="DoLogger boot sequence — Hello DoLogger, formatter_json + filter_level plugins, Ed25519 chain armed, 7-stage pipeline online" />
      </div>

      <div class="badge" v-if="badgeRelease">
        <svg class="icon"><use href="./assets/icons.svg#icon-rocket"></use></svg>
        <span>{{ badgeRelease.tag_name }}</span>
        <span v-if="badgeRelease.prerelease" class="badge-pre">{{ t('rel-prerelease') }}</span>
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
        <button ref="filterAnchorEl" type="button" class="btn btn-outline" :class="{ active: fOpen }"
                :aria-expanded="fOpen" aria-haspopup="dialog" @click="fOpen = !fOpen">
          <svg class="icon"><use href="./assets/icons.svg#icon-tag"></use></svg>
          {{ t('panel-filter-title') }}
          <b class="vs-current">{{ filterSummary }}</b>
          <svg class="icon chev" :class="{ open: fOpen }"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
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
    </div>

    <FilterPopup :open="fOpen" :releases="releases" :anchor-el="filterAnchorEl"
                 ref="popupRef" @close="fOpen = false" />

    <div class="scroll-hint" @click="scrollToDemo">
      <svg class="icon"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
      <span class="text">{{ hintText }}</span>
      <span class="cursor"></span>
    </div>
  </section>
</template>
