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
const FORK_URL = REPO_URL + '/fork'
const STAR_API_URL = 'https://api.github.com/user/starred/Nekolio/DoLogger'
const OS_KEYS: Record<string, string> = { windows: 'os-windows', macos: 'os-macos', linux: 'os-linux' }
const REDUCED_MOTION = window.matchMedia('(prefers-reduced-motion: reduce)').matches

/* Everything derives from the SELECTED release (default: latest), so
   picking a single version in the filter popup flips the download
   button and the os label at once. */
const releases = computed(() => siteData.value?.releases ?? [])
const repo = computed(() => siteData.value?.repo ?? null)
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
/* Live star/fork counts (data.ts refreshes them with the 15-min cache
   TTL; — while unavailable). Compact notation: 12.3k. */
const COUNT_FMT = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 })
function fmtCount(n: number | null | undefined): string {
  return n == null ? '—' : COUNT_FMT.format(n)
}

/* ── real star / fork actions ─────────────────────────────────────── */
/* Star first tries the GitHub API PUT (it 401s without auth); on success
   the heart fills and a transient toast confirms, on any failure the repo
   page opens so the user can star manually. Counts stay live from
   data.ts — no fake increment. Fork is a plain link to GitHub's flow. */
const starred = ref(false)
const starBusy = ref(false)
const starMsg = ref('')
let starResetTimer: number | null = null

function showStarSuccess() {
  starred.value = true
  starMsg.value = t('star-done')
  if (starResetTimer !== null) clearTimeout(starResetTimer)
  starResetTimer = window.setTimeout(() => {
    starred.value = false
    starMsg.value = ''
  }, 2600)
}

async function onStar() {
  if (starBusy.value) return
  starBusy.value = true
  try {
    const res = await fetch(STAR_API_URL, {
      method: 'PUT',
      signal: AbortSignal.timeout(8000)
    })
    if (res.ok) showStarSuccess()
    else window.open(REPO_URL, '_blank', 'noopener')
  } catch {
    /* 401 / network failure / CORS → degrade to manual starring */
    window.open(REPO_URL, '_blank', 'noopener')
  } finally {
    starBusy.value = false
  }
}

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
onBeforeUnmount(() => {
  stopHintTyping()
  if (starResetTimer !== null) clearTimeout(starResetTimer)
})
</script>

<template>
  <section class="page" id="page1">
    <div class="hero-content">
      <div class="hero-frame">
        <img class="hero" src="./assets/hero.svg" draggable="false"
             alt="DoLogger boot sequence — 4 sandboxed plugins (trust BLUE), Ed25519 chain armed, 7-stage pipeline online, plus the current release version" />
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

        <!-- live repo stats — the GitHub icon links to the repo; the star
             button really stars (API PUT → repo page fallback); the fork
             link opens GitHub's fork flow. Counts stay live from data.ts. -->
        <div class="repo-stats-cta">
          <a class="rs-gh" :href="REPO_URL" target="_blank" rel="noopener"
             :aria-label="t('repo-github')" :title="t('repo-github')">
            <svg class="icon"><use href="./assets/icons.svg#icon-github"></use></svg>
          </a>
          <button type="button" class="rs-item rs-star" :class="{ starred }"
                  :aria-label="t('repo-stars-aria')" :aria-pressed="starred"
                  :title="t('star')" @click="onStar">
            <b class="rs-num">{{ fmtCount(repo?.stargazers_count) }}</b>
            <span class="rs-heart" aria-hidden="true">
              <svg class="icon heart-outline"><use href="./assets/icons.svg#icon-heart"></use></svg>
              <svg class="icon heart-fill"><use href="./assets/icons.svg#icon-heart-fill"></use></svg>
            </span>
          </button>
          <span class="rs-sep" aria-hidden="true">|</span>
          <a class="rs-item rs-fork-item" :href="FORK_URL" target="_blank" rel="noopener"
             :aria-label="t('repo-forks-aria')" :title="t('repo-forks-aria')">
            <b class="rs-num">{{ fmtCount(repo?.forks_count) }}</b>
            <span class="rs-fork" aria-hidden="true">
              <svg class="icon"><use href="./assets/icons.svg#icon-branch"></use></svg>
            </span>
          </a>
        </div>

        <a :href="docsUrl" class="btn btn-outline">
          <svg class="icon"><use href="./assets/icons.svg#icon-book"></use></svg>
          {{ t('docs') }}
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

    <!-- transient confirmation for a successful star -->
    <Transition name="toast">
      <div v-if="starMsg" class="star-toast" role="status">{{ starMsg }}</div>
    </Transition>
  </section>
</template>

<style>
/* ────────────────────────────────────────────────────────────────
   PageHero.vue component styles. style.css owns the site theme; these
   overrides win because they are anchored to the #page1 id (style.css
   uses bare class selectors), so they apply on page 1 only.
   ──────────────────────────────────────────────────────────────── */

/* ── star/fork hover is scoped to the ICON, not the whole cluster ──
   style.css fires the heart spring and the fork bounce on
   .repo-stats-cta:hover; reset that here, then re-trigger only while
   the pointer is actually over the star item / branch icon. */
#page1 .repo-stats-cta:hover .rs-heart .heart-outline { transform: scale(1); opacity: 1; }
#page1 .repo-stats-cta:hover .rs-heart .heart-fill { transform: scale(0); opacity: 0; }
#page1 .repo-stats-cta:hover .rs-fork .icon { animation: none; filter: none; }

/* star: hovering the star item (count + heart icon) springs the fill in,
   reusing the non-linear transition style.css put on .heart-fill */
#page1 .repo-stats-cta .rs-star:hover .rs-heart .heart-outline { transform: scale(0.2); opacity: 0; }
#page1 .repo-stats-cta .rs-star:hover .rs-heart .heart-fill { transform: scale(1); opacity: 1; }

/* fork: hovering the branch icon plays the "commit lands" bounce */
#page1 .repo-stats-cta .rs-fork-item:hover .rs-fork .icon {
  animation: fork-land 0.9s cubic-bezier(0.22, 1, 0.36, 1) both;
  filter: drop-shadow(0 0 5px rgba(244, 114, 208, 0.7));
}

/* transient "starred" confirmation — the heart stays filled for a beat */
#page1 .repo-stats-cta .rs-star.starred .rs-heart .heart-outline { transform: scale(0.2); opacity: 0; }
#page1 .repo-stats-cta .rs-star.starred .rs-heart .heart-fill { transform: scale(1); opacity: 1; }

/* the star item is now a real <button>: drop UA chrome so it matches the
   rest of the cluster (style.css styled these as spans inside one <a>) */
#page1 .repo-stats-cta .rs-star {
  background: none;
  border: 0;
  padding: 0;
  margin: 0;
  font: inherit;
  color: inherit;
  line-height: inherit;
  cursor: pointer;
  -webkit-appearance: none;
  appearance: none;
}
#page1 .repo-stats-cta a {
  color: inherit;
  text-decoration: none;
}
#page1 .repo-stats-cta .rs-gh {
  display: inline-flex;
  align-items: center;
}

/* ── hero version badge: pin it above siblings and give it a themed
   background + glow so it can never be covered or visually lost ── */
#page1 .badge {
  position: relative;
  z-index: 3;
  background: var(--bg-card);
  backdrop-filter: blur(6px);
  box-shadow: 0 0 14px var(--accent-glow);
}

/* ── transient star toast (fixed to the viewport, bottom-center) ── */
.star-toast {
  position: fixed;
  left: 50%;
  bottom: 5.5rem;
  transform: translateX(-50%);
  padding: 0.5rem 1.1rem;
  border-radius: 24px;
  background: var(--bg-card);
  backdrop-filter: blur(8px);
  border: 1px solid var(--accent);
  color: var(--accent);
  font-size: 0.9rem;
  white-space: nowrap;
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45), 0 0 18px var(--accent-glow);
  z-index: 300;
  pointer-events: none;
}
.toast-enter-active, .toast-leave-active { transition: opacity 0.25s ease, transform 0.25s ease; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateX(-50%) translateY(8px); }

/* ── reduced motion: static final states, no spring/bounce ── */
@media (prefers-reduced-motion: reduce) {
  #page1 .repo-stats-cta .rs-heart .heart-outline,
  #page1 .repo-stats-cta .rs-heart .heart-fill { transition: none; }
  #page1 .repo-stats-cta .rs-fork-item:hover .rs-fork .icon { animation: none; }
  .star-toast, .toast-enter-active, .toast-leave-active { transition: none; }
}
</style>
