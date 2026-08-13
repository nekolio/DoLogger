<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSiteData, useSelectedTag, selectRelease, pickRelease, assetFor, groupAssets } from '../data'

const { t, locale } = useI18n()
const siteData = useSiteData()
const selectedTag = useSelectedTag()

const REPO_URL = 'https://github.com/Nekolio/DoLogger'
const WIKI_URL = REPO_URL + '/wiki'
const RELEASES_URL = REPO_URL + '/releases'
const OS_KEYS: Record<string, string> = { windows: 'os-windows', macos: 'os-macos', linux: 'os-linux' }
const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches

/* Everything derives from the SELECTED release (default: latest), so
   picking v0.2.0 in the dropdown flips the download button, the os
   label, and the grouped asset panel all at once. */
const releases = computed(() => siteData.value?.releases ?? [])
const selectedRelease = computed(() => pickRelease(releases.value.length ? releases.value : null))
const platform = computed(() => siteData.value?.platform ?? { os: 'linux', arch: 'x86_64' })
const version = computed(() => selectedRelease.value.tag_name || 'v0.1.0')
const downloadUrl = computed(() => {
  const hit = assetFor(selectedRelease.value, platform.value.os, platform.value.arch)
  return (hit && hit.browser_download_url) || selectedRelease.value.html_url || RELEASES_URL
})
const osLabel = computed(() => {
  const osKey = OS_KEYS[platform.value.os] || 'os-linux'
  return t(osKey) + ' (' + platform.value.arch + ')'
})
const assetGroups = computed(() => groupAssets(selectedRelease.value))
const checksumsUrl = computed(() => selectedRelease.value.assets?.find(a => a.name === 'checksums-sha256.txt')?.browser_download_url ?? '')
const benchUrl = computed(() => selectedRelease.value.assets?.find(a => a.name === 'benchmark-results.json')?.browser_download_url ?? '')
const docsUrl = computed(() =>
  locale.value === 'zh' ? WIKI_URL + '/Chinese-Home' : WIKI_URL + '/Home'
)

function fmtDate(iso: string): string {
  const d = new Date(iso)
  return isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10)
}

/* ── version dropdown (custom, keyboard-accessible) ───────────────── */
const open = ref(REDUCED_MOTION) // reduced-motion users get the panel pre-opened
const vOpen = ref(false)
const vIndex = ref(0)
const versionSelEl = ref<HTMLElement | null>(null)

/* ── panel height cap, measured at runtime ─────────────────────────
 * The open panel must never push the hero past the fold, but the
 * content above it (hero art, title, tags, actions) is not a fixed
 * size — locale, zoom, and width all change it. So measure the real
 * distance from the actions row to the viewport bottom and expose it
 * as --panel-avail; the CSS cap then always leaves the panel inside
 * the page, whatever the layout ends up being. */
const actionsEl = ref<HTMLElement | null>(null)
const PAGE_PAD = 24     // .page padding (1.5rem)
const PANEL_GAP = 14    // .panel margin-top (0.9rem)
const PANEL_SLACK = 24  // breathing room so nothing sits flush at the fold
let measureRaf = 0
function measurePanelAvail() {
  cancelAnimationFrame(measureRaf)
  measureRaf = requestAnimationFrame(() => {
    const el = actionsEl.value
    if (!el) return
    const above = el.offsetTop + el.offsetHeight // offsetParent = .hero-content
    const avail = Math.max(0, window.innerHeight - PAGE_PAD * 2 - above - PANEL_GAP - PANEL_SLACK)
    document.documentElement.style.setProperty('--panel-avail', avail + 'px')
  })
}

function toggleVersionMenu() { vOpen.value = !vOpen.value }
function chooseVersion(tag: string) {
  selectRelease(tag)
  vOpen.value = false
}
function onVersionKey(e: KeyboardEvent) {
  const rels = releases.value
  if (!rels.length) return
  if ((e.key === 'Enter' || e.key === ' ') && !vOpen.value) {
    vOpen.value = true
    vIndex.value = Math.max(0, rels.findIndex(r => r.tag_name === selectedRelease.value.tag_name))
    e.preventDefault()
  } else if (e.key === 'Escape') {
    vOpen.value = false
  } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault()
    if (!vOpen.value) {
      vOpen.value = true
      vIndex.value = Math.max(0, rels.findIndex(r => r.tag_name === selectedRelease.value.tag_name))
      return
    }
    vIndex.value = (vIndex.value + (e.key === 'ArrowDown' ? 1 : -1) + rels.length) % rels.length
  } else if (e.key === 'Enter' && vOpen.value) {
    chooseVersion(rels[vIndex.value].tag_name)
  }
}
function onClickOutside(e: MouseEvent) {
  const el = versionSelEl.value
  if (el && !(e.target instanceof Node && el.contains(e.target))) vOpen.value = false
}
onMounted(() => document.addEventListener('mousedown', onClickOutside))
onBeforeUnmount(() => document.removeEventListener('mousedown', onClickOutside))

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

/* re-measure whenever the layout above the panel could change */
onMounted(() => {
  measurePanelAvail()
  window.addEventListener('resize', measurePanelAvail)
  if (document.fonts) document.fonts.ready.then(measurePanelAvail).catch(() => {})
})
watch(locale, () => nextTick(measurePanelAvail))
onBeforeUnmount(() => {
  window.removeEventListener('resize', measurePanelAvail)
  cancelAnimationFrame(measureRaf)
})
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

      <div class="actions" ref="actionsEl">
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
          <!-- version picker: selecting a tag re-targets every link below -->
          <div class="version-select" ref="versionSelEl">
            <button type="button" class="vs-button" :aria-expanded="vOpen" aria-haspopup="listbox"
                    @click="toggleVersionMenu" @keydown="onVersionKey">
              <svg class="icon"><use href="./assets/icons.svg#icon-tag"></use></svg>
              <span class="vs-label">{{ t('panel-select-version') }}</span>
              <b class="vs-current">{{ version }}</b>
              <svg class="icon chev" :class="{ open: vOpen }"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
            </button>
            <ul v-if="vOpen" class="vs-menu" role="listbox" :aria-label="t('panel-select-version')">
              <li v-for="(r, i) in releases" :key="r.tag_name" role="option"
                  :aria-selected="r.tag_name === selectedRelease.tag_name"
                  :class="{ selected: r.tag_name === selectedRelease.tag_name, focused: i === vIndex }"
                  @click="chooseVersion(r.tag_name)" @mouseenter="vIndex = i">
                <span class="vs-tag">{{ r.tag_name }}</span>
                <span v-if="r.prerelease" class="prerelease-badge">{{ t('rel-prerelease') }}</span>
                <span class="vs-date">{{ fmtDate(r.published_at) }}</span>
              </li>
            </ul>
          </div>

          <!-- assets grouped by architecture → OS (both naming schemes) -->
          <div class="asset-groups">
            <div v-for="g in assetGroups" :key="g.arch" class="arch-group">
              <h4 class="arch-head">{{ g.arch }}</h4>
              <div v-for="row in g.rows" :key="row.os" class="os-row">
                <span class="os-name">{{ t(OS_KEYS[row.os]) }}</span>
                <span v-if="row.cli" class="tag-kind">CLI</span>
                <a v-if="row.cli" :href="row.cli.browser_download_url" :title="row.cli.name">{{ row.cli.name }}</a>
                <span v-if="row.lib" class="tag-kind lib">LIB</span>
                <a v-if="row.lib" :href="row.lib.browser_download_url" :title="row.lib.name">{{ row.lib.name }}</a>
              </div>
            </div>
            <div class="panel-extra">
              <a v-if="checksumsUrl" :href="checksumsUrl">
                <svg class="icon"><use href="./assets/icons.svg#icon-shield"></use></svg>
                {{ t('panel-checksums') }}
              </a>
              <a v-if="benchUrl" :href="benchUrl">
                <svg class="icon"><use href="./assets/icons.svg#icon-gauge"></use></svg>
                {{ t('panel-bench') }}
              </a>
            </div>
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
