<script setup lang="ts">
/* FilterPopup — the page-1 asset browser.
 *
 * Opens directly from the hero action button (no intermediate panel),
 * teleported to body so nothing clips or becomes its containing block.
 * Compact by design: the filter block is fully visible with no inner
 * scrollbar (the version selector is a dropdown, not a chip wall), and
 * the results area fits ~4-6 rows — surplus rows scroll internally.
 *
 * Filters: version dropdown (multi-select, lists EVERY release, default
 * latest two — results preload those until filters are touched), release
 * type, OS, arch, kind (CLI / LIB / 官方插件). Default scope = the
 * visitor's own platform (OS + arch), one "全部" chip per group to widen.
 *
 * The version dropdown drives the hero's download button: exactly one
 * version selected → that release, else the latest.
 *
 * Theme/language toggles in .top-controls deliberately do NOT close the
 * popup — it re-renders in place (the outside-mousedown handler excludes
 * the top bar). Wheel over the popup is hard-locked
 * (data-wheel-lock-hard): lists scroll natively, the page never flips.
 */
import { ref, computed, watch, nextTick, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { groupAssets, selectRelease, useSiteData, OS_ORDER, ARCH_ORDER,
         type Release, type ReleaseAsset, type Platform } from '../data'

const props = defineProps<{
  open: boolean
  releases: Release[]
  anchorEl: HTMLElement | null
}>()
const emit = defineEmits<{ (e: 'close'): void }>()
const { t } = useI18n()
const siteData = useSiteData()
const platform = computed<Platform>(() => siteData.value?.platform ?? { os: 'linux', arch: 'x86_64' })

const OS_KEYS: Record<string, string> = { windows: 'os-windows', macos: 'os-macos', linux: 'os-linux' }

type Kind = 'cli' | 'lib' | 'plugin'
type RelType = 'release' | 'prerelease'
const KIND_OPTS: Kind[] = ['cli', 'lib', 'plugin']
const TYPE_OPTS: RelType[] = ['release', 'prerelease']

const fVersions = ref<string[]>([])
const fOs = ref<Platform['os'][]>([])
const fArch = ref<string[]>([])
const fKind = ref<Kind[]>([...KIND_OPTS])
const fType = ref<RelType[]>([...TYPE_OPTS])

const defaultVersions = computed(() => props.releases.slice(0, 2).map(r => r.tag_name))

/* Default scope, applied once the data arrives (the popup can open
   before the fetch resolves): latest two versions + the visitor's own
   platform — most users only want their one or two assets. */
let init = false
watch(() => props.releases, (rels) => {
  if (init || !rels.length) return
  init = true
  fVersions.value = rels.slice(0, 2).map(r => r.tag_name)
  fOs.value = [platform.value.os]
  fArch.value = [platform.value.arch]
}, { immediate: true })

/* Exactly one version selected → the download button targets it. */
watch(fVersions, (v) => {
  const tags = props.releases.map(r => r.tag_name)
  selectRelease(v.length === 1 && tags.includes(v[0]) ? v[0] : (tags[0] ?? null))
})

/* Toggles a plain array (template refs auto-unwrap, so the click
   handlers pass values and assign the result back — passing a ref
   from the template would deliver the unwrapped array instead). */
function toggle<T>(list: T[], v: T): T[] {
  return list.includes(v) ? list.filter(x => x !== v) : [...list, v]
}
function resetFilters() {
  fVersions.value = defaultVersions.value
  fOs.value = [platform.value.os]
  fArch.value = [platform.value.arch]
  fKind.value = [...KIND_OPTS]
  fType.value = [...TYPE_OPTS]
}

/* ── version dropdown ─────────────────────────────────────────────── */
const vOpen = ref(false)
function toggleAllVersions(e: Event) {
  const on = (e.target as HTMLInputElement).checked
  fVersions.value = on ? props.releases.map(r => r.tag_name) : []
}
const versionSummary = computed(() => {
  const def = defaultVersions.value
  if (fVersions.value.length && def.length === fVersions.value.length
      && def.every(x => fVersions.value.includes(x))) return t('filter-versions-summary')
  if (!fVersions.value.length) return t('filter-versions-none')
  if (props.releases.length && fVersions.value.length >= props.releases.length) return t('filter-versions-all')
  const first = fVersions.value.slice(0, 2).join(', ')
  return fVersions.value.length > 2 ? `${first} +${fVersions.value.length - 2}` : first
})

/* ── live filtering ──────────────────────────────────────────────── */
interface ResItem { kind: Kind; asset: ReleaseAsset; tag: string; short: string }
interface ResArch { arch: string; items: ResItem[] }
interface ResOs { os: Platform['os']; archs: ResArch[] }

const filteredReleases = computed(() =>
  props.releases.filter(r =>
    fVersions.value.includes(r.tag_name) &&
    (r.prerelease ? fType.value.includes('prerelease') : fType.value.includes('release'))
  )
)

/* Results grouped 大分类 OS → 中分类 arch → 小分类 rows (CLI/LIB/插件),
   ordered by OS_ORDER / ARCH_ORDER with releases newest-first. */
const osGroups = computed<ResOs[]>(() => {
  const map = new Map<Platform['os'], Map<string, ResItem[]>>()
  for (const rel of filteredReleases.value) {
    for (const g of groupAssets(rel)) {
      if (!fArch.value.includes(g.arch)) continue
      for (const row of g.rows) {
        if (!fOs.value.includes(row.os)) continue
        let archMap = map.get(row.os)
        if (!archMap) { archMap = new Map(); map.set(row.os, archMap) }
        let items = archMap.get(g.arch)
        if (!items) { items = []; archMap.set(g.arch, items) }
        const push = (kind: Kind, asset: ReleaseAsset) =>
          items!.push({ kind, asset, tag: rel.tag_name, short: asset.name })
        if (row.cli && fKind.value.includes('cli')) push('cli', row.cli)
        if (row.lib && fKind.value.includes('lib')) push('lib', row.lib)
        if (row.plugins && fKind.value.includes('plugin')) row.plugins.forEach(p => push('plugin', p))
      }
    }
  }
  return OS_ORDER.filter(os => map.has(os)).map(os => ({
    os,
    archs: ARCH_ORDER.filter(a => map.get(os)!.has(a))
      .map(a => ({ arch: a, items: map.get(os)!.get(a)! }))
  }))
})

const matchCount = computed(() => osGroups.value.reduce(
  (n, g) => n + g.archs.reduce((m, a) => m + a.items.length, 0), 0))

/* footer links make sense only when exactly one version is targeted */
const singleRel = computed(() => filteredReleases.value.length === 1 ? filteredReleases.value[0] : null)
const checksumsUrl = computed(() =>
  singleRel.value?.assets?.find(a => a.name === 'checksums-sha256.txt')?.browser_download_url ?? '')
const benchUrl = computed(() =>
  singleRel.value?.assets?.find(a => a.name === 'benchmark-results.json')?.browser_download_url ?? '')
const multiVer = computed(() => filteredReleases.value.length > 1)

function kindLabel(k: Kind): string {
  return k === 'cli' ? 'CLI' : k === 'lib' ? 'LIB' : t('filter-plugins')
}

const summary = computed(() => `${matchCount.value} ${t('filter-assets')}`)
defineExpose({ summary })

/* ── positioning: centered on the anchor, flip up when tight ────────
   The panel is enlarged as a WHOLE (1.25× on top of the earlier size),
   so it reaches ~862 px wide; its horizontal center follows the trigger
   button's center (not the button's left edge), clamped to the viewport. */
const popupEl = ref<HTMLElement | null>(null)
const vselEl = ref<HTMLElement | null>(null)
const openDir = ref<'down' | 'up'>('down')
const POPUP_W = 690
const POPUP_SCALE = 1.25          // whole-panel re-scale: 690 → ~862 px wide
const FLIP_MIN = 450              // flip up when less space remains below the anchor

function position() {
  const pop = popupEl.value
  const anchor = props.anchorEl
  if (!pop || !anchor) return
  const r = anchor.getBoundingClientRect()
  const width = Math.min(POPUP_W * POPUP_SCALE, window.innerWidth - 24)
  const center = r.left + r.width / 2
  pop.style.left = Math.max(12, Math.min(center - width / 2, window.innerWidth - width - 12)) + 'px'
  pop.style.width = width + 'px'
  const spaceBelow = window.innerHeight - r.bottom - 12
  const spaceAbove = r.top - 12
  if (spaceBelow < FLIP_MIN && spaceAbove > spaceBelow) {
    openDir.value = 'up'
    pop.style.top = 'auto'
    pop.style.bottom = (window.innerHeight - r.top + 8) + 'px'
    pop.style.setProperty('--pop-max-h', spaceAbove + 'px')
  } else {
    openDir.value = 'down'
    pop.style.bottom = 'auto'
    pop.style.top = (r.bottom + 8) + 'px'
    pop.style.setProperty('--pop-max-h', spaceBelow + 'px')
  }
}

/* ── close: outside mousedown (popup, anchor AND the top-controls
      theme/lang bar are excluded — switching theme or language must
      not dismiss the dialog), Esc, page scroll, resize ────────────── */
function onDocDown(e: MouseEvent) {
  const pop = popupEl.value
  const anchor = props.anchorEl
  const tgt = e.target as Node | null
  if (!tgt) return
  if (tgt instanceof Element && tgt.closest('.top-controls')) return // theme/lang toggles keep the popup open
  if (pop && pop.contains(tgt)) {
    /* inside the popup: only the version dropdown itself toggles it */
    if (vOpen.value && vselEl.value && !vselEl.value.contains(tgt)) vOpen.value = false
    return
  }
  if (anchor && anchor.contains(tgt)) return
  emit('close')
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (vOpen.value) vOpen.value = false
    else emit('close')
  }
}
/* Scroll-close is SELECTIVE and keys off the DOCUMENT's own scroll
   position, not the anchor's viewport rect:
     - the listener is NON-capture on window, so it fires ONLY for the
       document scroller. Inner scrollers (popup results list, version
       dropdown, demo terminal, page-3 card bodies) never bubble and can
       never reach it — no capture-phase leakage;
     - the close signal is a real change in window.scrollY. Layout shifts
       (the hero badge appearing once the release fetch resolves, a web
       font swap, the typewriter re-layout) move the anchor WITHOUT
       changing scrollY, so they can never dismiss the dialog — on every
       refresh, deterministically.
   A short grace window still absorbs any immediate scroll (e.g. scroll
   restoration right after a refresh) so it cannot close the fresh panel. */
let openedAt = 0
let scrollYAtOpen = 0
const SCROLL_GRACE = 400  // ms after opening during which scrolls are ignored
const SCROLL_SLOP = 8     // px the DOCUMENT must scroll to count as a real page scroll

function onScroll(e: Event) {
  const t = e.target
  /* Defensive: the non-capture listener already excludes inner scrollers,
     but ignore any stray element target (never the document) just in case. */
  if (t instanceof HTMLElement && t !== document.documentElement && t !== document.body) return
  if (performance.now() - openedAt < SCROLL_GRACE) return
  if (Math.abs(window.scrollY - scrollYAtOpen) <= SCROLL_SLOP) return
  emit('close')
}
function onResize() { emit('close') }

watch(() => props.open, (v) => {
  if (v) {
    openedAt = performance.now()
    scrollYAtOpen = window.scrollY
    nextTick(() => position())
    document.addEventListener('mousedown', onDocDown)
    document.addEventListener('keydown', onKey)
    window.addEventListener('scroll', onScroll, { passive: true })
    window.addEventListener('resize', onResize)
  } else {
    vOpen.value = false
    document.removeEventListener('mousedown', onDocDown)
    document.removeEventListener('keydown', onKey)
    window.removeEventListener('scroll', onScroll)
    window.removeEventListener('resize', onResize)
  }
})
onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocDown)
  document.removeEventListener('keydown', onKey)
  window.removeEventListener('scroll', onScroll)
  window.removeEventListener('resize', onResize)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="fpop2">
      <div v-if="open" ref="popupEl" class="fpopup fpop2" role="dialog" :aria-label="t('panel-filter-title')"
           :class="openDir" data-wheel-lock data-wheel-lock-hard>
        <div class="fpop-filters">
          <div class="frow">
            <div class="vsel" ref="vselEl">
              <button type="button" class="vsel-btn" :aria-expanded="vOpen" @click="vOpen = !vOpen">
                <span class="fg-inline">{{ t('filter-versions') }}</span>
                <span class="vsel-summary">{{ versionSummary }}</span>
                <svg class="icon chev" :class="{ open: vOpen }"><use href="./assets/icons.svg#icon-chevron-down"></use></svg>
              </button>
              <div v-if="vOpen" class="vsel-menu">
                <label class="vsel-all">
                  <input type="checkbox" :checked="releases.length > 0 && fVersions.length === releases.length"
                         @change="toggleAllVersions" />
                  <span>{{ t('filter-versions-all') }}</span>
                </label>
                <label v-for="r in releases" :key="r.tag_name" class="vsel-item">
                  <input type="checkbox" :checked="fVersions.includes(r.tag_name)"
                         @change="fVersions = toggle(fVersions, r.tag_name)" />
                  <span class="vsel-tag">{{ r.tag_name }}</span>
                  <span v-if="r.prerelease" class="prerelease-badge">{{ t('rel-prerelease') }}</span>
                </label>
              </div>
            </div>
            <div class="fchips">
              <span class="fg-inline">{{ t('filter-release-type') }}</span>
              <button v-for="ft in TYPE_OPTS" :key="ft" type="button" class="chip"
                      :class="{ on: fType.includes(ft) }" :aria-pressed="fType.includes(ft)"
                      @click="fType = toggle(fType, ft)">
                {{ ft === 'release' ? t('filter-stable') : t('rel-prerelease') }}
              </button>
            </div>
          </div>

          <div class="frow">
            <div class="fchips">
              <span class="fg-inline">{{ t('filter-os') }}</span>
              <button type="button" class="chip" :class="{ on: fOs.length === OS_ORDER.length }"
                      :aria-pressed="fOs.length === OS_ORDER.length" @click="fOs = [...OS_ORDER]">
                {{ t('filter-all') }}
              </button>
              <button v-for="os in OS_ORDER" :key="os" type="button" class="chip"
                      :class="{ on: fOs.includes(os) }" :aria-pressed="fOs.includes(os)"
                      @click="fOs = toggle(fOs, os)">{{ t(OS_KEYS[os]) }}</button>
            </div>
          </div>

          <div class="frow">
            <div class="fchips">
              <span class="fg-inline">{{ t('filter-arch') }}</span>
              <button type="button" class="chip" :class="{ on: fArch.length === ARCH_ORDER.length }"
                      :aria-pressed="fArch.length === ARCH_ORDER.length" @click="fArch = [...ARCH_ORDER]">
                {{ t('filter-all') }}
              </button>
              <button v-for="arch in ARCH_ORDER" :key="arch" type="button" class="chip"
                      :class="{ on: fArch.includes(arch) }" :aria-pressed="fArch.includes(arch)"
                      @click="fArch = toggle(fArch, arch)">{{ arch }}</button>
            </div>
            <div class="fchips">
              <span class="fg-inline">{{ t('filter-kind') }}</span>
              <button v-for="kind in KIND_OPTS" :key="kind" type="button" class="chip"
                      :class="{ on: fKind.includes(kind) }" :aria-pressed="fKind.includes(kind)"
                      @click="fKind = toggle(fKind, kind)">{{ kindLabel(kind) }}</button>
            </div>
          </div>
        </div>

        <div class="fpop-results" data-wheel-lock>
          <!-- The whole result region (list OR empty state) fades/slides
               on filter changes that flip between them (e.g. toggling the
               release type to a filter with no matches). mode="out-in"
               plays the leave before the enter so nothing pops. -->
          <Transition name="resregion" mode="out-in">
            <template v-if="osGroups.length">
              <!-- OS groups and arch groups get their own transition groups
                   so switching ANY filter (os/arch/kind/type) animates the
                   appearing/disappearing groups AND the rows reflow — not
                   just the row-level kind filter. -->
              <TransitionGroup name="resgrp" tag="div" class="res-groups">
                <div v-for="g in osGroups" :key="g.os" class="res-os">
                  <h4 class="res-os-head">
                    <span class="res-os-name">{{ t(OS_KEYS[g.os]) }}</span>
                    <span class="res-os-count">{{ g.archs.reduce((n, a) => n + a.items.length, 0) }} {{ t('filter-assets') }}</span>
                  </h4>
                  <TransitionGroup name="resarch" tag="div" class="res-archs">
                    <div v-for="a in g.archs" :key="a.arch" class="res-arch">
                      <h5 class="res-arch-head">{{ a.arch }}</h5>
                      <TransitionGroup name="resrow" tag="div" class="res-arch-rows">
                        <a v-for="it in a.items" :key="it.asset.name" class="res-row"
                           :href="it.asset.browser_download_url" :title="it.asset.name">
                          <span class="tag-kind" :class="it.kind">{{ kindLabel(it.kind) }}</span>
                          <span class="res-name">{{ it.short }}</span>
                          <span v-if="multiVer" class="res-ver">{{ it.tag }}</span>
                        </a>
                      </TransitionGroup>
                    </div>
                  </TransitionGroup>
                </div>
              </TransitionGroup>
            </template>
            <div v-else class="fpop-empty">
              <p>{{ t('filter-empty') }}</p>
              <button type="button" class="chip" @click="resetFilters">{{ t('filter-reset') }}</button>
            </div>
          </Transition>
        </div>

        <div v-if="singleRel" class="fpop-foot">
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
    </Transition>
  </Teleport>
</template>

<style>
/* ── FilterPopup component styles (teleported to <body>, non-scoped so
   they apply to the detached root). Scoped under `.fpop2` to beat the
   global style.css rules by specificity. The panel was re-enlarged as a
   WHOLE (width is set inline in position()) and the typography was pulled
   back to comfortable proportions; enter/leave is a mirror pair, chips
   got a springy select/deselect, and the panel speaks JetBrains Mono. ── */

/* typography — one monospace voice for the whole panel */
.fpop2,
.fpop2 .chip,
.fpop2 .vsel-btn,
.fpop2 .vsel-btn .vsel-summary,
.fpop2 .vsel-menu label,
.fpop2 .vsel-menu .vsel-tag,
.fpop2 .res-os-head,
.fpop2 .res-os-count,
.fpop2 .res-arch-head,
.fpop2 .res-row,
.fpop2 .res-row .tag-kind,
.fpop2 .res-row .res-ver,
.fpop2 .fpop-foot a,
.fpop2 .fpop-empty {
  font-family: 'JetBrains Mono', ui-monospace, Consolas, 'Courier New', monospace;
}

/* font sizes pulled back from the over-enlarged typography. The chip
   label still crowded its pill, so chips drop a further notch and gain
   proportionally more padding (see the chip rule below); the version
   selector and the inline group labels follow so the hierarchy stays
   consistent and comfortable. */
.fpop2 .fg-inline { font-size: 0.78rem; }
.fpop2 .chip { font-size: 0.85rem; }
.fpop2 .vsel-btn { font-size: 0.92rem; }
.fpop2 .vsel-menu label { font-size: 0.92rem; }
.fpop2 .vsel-all { font-size: 0.84rem !important; }
.fpop2 .prerelease-badge { font-size: 0.8rem; }
.fpop2 .res-os-head { font-size: 1.05rem; }
.fpop2 .res-os-count { font-size: 0.85rem; }
.fpop2 .res-arch-head { font-size: 0.92rem; }
.fpop2 .res-row { font-size: 0.98rem; }
.fpop2 .res-row .tag-kind { font-size: 0.82rem; }
.fpop2 .res-row .res-ver { font-size: 0.84rem; }
.fpop2 .fpop-foot a { font-size: 1rem; }
.fpop2 .fpop-empty { font-size: 1.05rem; }

/* chips — springy scale pop on select, soft settle/shrink on deselect,
   plus the color/border/glow transitions carried over from style.css */
.fpop2 .chip {
  padding: 0.4rem 1.05rem;
  transition: transform 0.28s cubic-bezier(0.22, 1, 0.36, 1),
              border-color 0.2s ease, color 0.2s ease,
              background 0.22s ease, box-shadow 0.22s ease;
}
.fpop2 .chip.on {
  animation: fpop-chip-in 0.32s cubic-bezier(0.34, 1.56, 0.64, 1);
}
.fpop2 .chip:not(.on) {
  animation: fpop-chip-out 0.26s cubic-bezier(0.22, 1, 0.36, 1);
}
@keyframes fpop-chip-in {
  0% { transform: scale(1); }
  45% { transform: scale(1.06); }
  100% { transform: scale(1); }
}
@keyframes fpop-chip-out {
  0% { transform: scale(1); }
  50% { transform: scale(0.955); }
  100% { transform: scale(1); }
}

/* asset-row list transitions — rows are keyed by asset name so inserts,
   removes and reorders animate. Enter slides down + fades in; leave fades
   out while sliding down out-of-flow (absolute) so the rows beneath it
   FLIP-glide up into the gap via .resrow-move. Non-linear easing mirrors
   the panel/chip motion language. */
.fpop2 .res-arch-rows { position: relative; }
.fpop2 .resrow-enter-active,
.fpop2 .resrow-leave-active {
  transition: opacity 0.2s ease, transform 0.3s cubic-bezier(0.22, 1, 0.36, 1);
}
.fpop2 .resrow-enter-from { opacity: 0; transform: translateY(-8px); }
.fpop2 .resrow-leave-to { opacity: 0; transform: translateY(6px); }
.fpop2 .resrow-leave-active { position: absolute; left: 0; right: 0; }
.fpop2 .resrow-move { transition: transform 0.32s cubic-bezier(0.22, 1, 0.36, 1); }

/* OS-group and arch-group transitions — same motion language, one level
   up: switching the OS or arch filter animates whole groups in/out and
   the remaining groups glide to fill (FLIP via .resgrp-move/.resarch-move). */
.fpop2 .res-groups { position: relative; }
.fpop2 .res-archs { position: relative; }
/* whole-region (list ↔ empty) transition — same motion language */
.fpop2 .resregion-enter-active,
.fpop2 .resregion-leave-active {
  transition: opacity 0.22s ease, transform 0.28s cubic-bezier(0.22, 1, 0.36, 1);
}
.fpop2 .resregion-enter-from { opacity: 0; transform: translateY(-8px); }
.fpop2 .resregion-leave-to { opacity: 0; transform: translateY(6px); }
.fpop2 .resgrp-enter-active,
.fpop2 .resgrp-leave-active,
.fpop2 .resarch-enter-active,
.fpop2 .resarch-leave-active {
  transition: opacity 0.22s ease, transform 0.3s cubic-bezier(0.22, 1, 0.36, 1);
}
.fpop2 .resgrp-enter-from,
.fpop2 .resarch-enter-from { opacity: 0; transform: translateY(-10px); }
.fpop2 .resgrp-leave-to,
.fpop2 .resarch-leave-to { opacity: 0; transform: translateY(6px); }
.fpop2 .resgrp-leave-active,
.fpop2 .resarch-leave-active { position: absolute; left: 0; right: 0; }
.fpop2 .resgrp-move,
.fpop2 .resarch-move { transition: transform 0.34s cubic-bezier(0.22, 1, 0.36, 1); }

/* enter/leave — MIRROR IMAGES: open scales up, fades in and drifts
   slightly upward (toward the anchor); close reverses exactly. The
   flip-up (.up) panel mirrors the drift vertically. */
.fpop2-enter-active,
.fpop2-leave-active {
  transition: transform 0.3s cubic-bezier(0.22, 1, 0.36, 1),
              opacity 0.3s cubic-bezier(0.22, 1, 0.36, 1);
}
.fpop2-enter-from { opacity: 0; transform: translateY(12px) scale(0.96); }
.fpop2-enter-to { opacity: 1; transform: translateY(0) scale(1); }
.fpop2-leave-from { opacity: 1; transform: translateY(0) scale(1); }
.fpop2-leave-to { opacity: 0; transform: translateY(12px) scale(0.96); }
.fpop2.up.fpop2-enter-from { transform: translateY(-12px) scale(0.96); }
.fpop2.up.fpop2-leave-to { transform: translateY(-12px) scale(0.96); }

@media (prefers-reduced-motion: reduce) {
  .fpop2-enter-active,
  .fpop2-leave-active { transition: none; }
  .fpop2-enter-from, .fpop2-enter-to,
  .fpop2-leave-from, .fpop2-leave-to { transform: none; opacity: 1; }
  .fpop2 .chip, .fpop2 .chip.on, .fpop2 .chip:not(.on) { animation: none; }
  .fpop2 .resrow-enter-active,
  .fpop2 .resrow-leave-active,
  .fpop2 .resrow-move { transition: none; }
  .fpop2 .resrow-enter-from,
  .fpop2 .resrow-leave-to { transform: none; opacity: 1; }
  .fpop2 .resrow-leave-active { position: static; }
  .fpop2 .resgrp-enter-active,
  .fpop2 .resgrp-leave-active,
  .fpop2 .resgrp-move,
  .fpop2 .resarch-enter-active,
  .fpop2 .resarch-leave-active,
  .fpop2 .resarch-move { transition: none; }
  .fpop2 .resgrp-enter-from,
  .fpop2 .resgrp-leave-to,
  .fpop2 .resarch-enter-from,
  .fpop2 .resarch-leave-to { transform: none; opacity: 1; }
  .fpop2 .resgrp-leave-active,
  .fpop2 .resarch-leave-active { position: static; }
}
</style>
