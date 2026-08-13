<script setup lang="ts">
/* FilterPopup — the page-1 asset browser.
 *
 * Lives in a Teleport so it overlays cleanly: inside the panel, the
 * panel's backdrop-filter would become its containing block and its
 * overflow would clip it. It is positioned fixed against the anchor
 * button's rect at open time, opens downward (upward when the viewport
 * leaves no room), and caps its height to the available space minus a
 * 12px margin — it can never touch the page bottom or stretch the panel.
 *
 * Filters are multi-select chips (version, arch, OS, CLI/LIB, release
 * vs pre-release) and the results list below updates live. The version
 * chips default to the latest two releases ("show everything, but not
 * really everything") and drive the hero's download button: exactly one
 * version selected → that release, else the latest.
 *
 * Wheel over the popup is hard-locked (data-wheel-lock-hard) — the
 * list scrolls natively and the page never flips away mid-interaction.
 */
import { ref, computed, watch, nextTick, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { groupAssets, selectRelease, OS_ORDER, ARCH_ORDER,
         type Release, type ReleaseAsset, type Platform } from '../data'

const props = defineProps<{
  open: boolean
  releases: Release[]
  anchorEl: HTMLElement | null
}>()
const emit = defineEmits<{ (e: 'close'): void }>()
const { t } = useI18n()

const OS_KEYS: Record<string, string> = { windows: 'os-windows', macos: 'os-macos', linux: 'os-linux' }

type Kind = 'cli' | 'lib'
type RelType = 'release' | 'prerelease'
const KIND_OPTS: Kind[] = ['cli', 'lib']
const TYPE_OPTS: RelType[] = ['release', 'prerelease']

const fVersions = ref<string[]>([])
const fOs = ref<Platform['os'][]>([...OS_ORDER])
const fArch = ref<string[]>([...ARCH_ORDER])
const fKind = ref<Kind[]>([...KIND_OPTS])
const fType = ref<RelType[]>([...TYPE_OPTS])

/* Default version filter: the latest two releases, applied once the
   data arrives (the popup can open before the fetch resolves). */
let versionsInit = false
watch(() => props.releases, (rels) => {
  if (!versionsInit && rels.length) {
    versionsInit = true
    fVersions.value = rels.slice(0, 2).map(r => r.tag_name)
  }
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
  fVersions.value = props.releases.slice(0, 2).map(r => r.tag_name)
  fOs.value = [...OS_ORDER]
  fArch.value = [...ARCH_ORDER]
  fKind.value = [...KIND_OPTS]
  fType.value = [...TYPE_OPTS]
}

/* ── live filtering ──────────────────────────────────────────────── */
interface PopRow { os: Platform['os']; cli?: ReleaseAsset; lib?: ReleaseAsset }
interface PopGroup { arch: string; rows: PopRow[] }
interface PopVer { release: Release; groups: PopGroup[] }

const filteredReleases = computed(() =>
  props.releases.filter(r =>
    fVersions.value.includes(r.tag_name) &&
    (r.prerelease ? fType.value.includes('prerelease') : fType.value.includes('release'))
  )
)

const results = computed<PopVer[]>(() => {
  const out: PopVer[] = []
  for (const rel of filteredReleases.value) {
    const ver: PopVer = { release: rel, groups: [] }
    for (const g of groupAssets(rel)) {
      if (!fArch.value.includes(g.arch)) continue
      const rows = g.rows
        .filter(row => fOs.value.includes(row.os))
        .map(row => {
          const r: PopRow = { os: row.os }
          if (row.cli && fKind.value.includes('cli')) r.cli = row.cli
          if (row.lib && fKind.value.includes('lib')) r.lib = row.lib
          return r
        })
        .filter(row => row.cli || row.lib)
      if (rows.length) ver.groups.push({ arch: g.arch, rows })
    }
    if (ver.groups.length) out.push(ver)
  }
  return out
})

const matchCount = computed(() =>
  results.value.reduce((n, v) => n + v.groups.reduce(
    (m, g) => m + g.rows.reduce((k, r) => k + (r.cli ? 1 : 0) + (r.lib ? 1 : 0), 0), 0), 0)
)

/* footer links make sense only when exactly one version is targeted */
const singleRel = computed(() => filteredReleases.value.length === 1 ? filteredReleases.value[0] : null)
const checksumsUrl = computed(() =>
  singleRel.value?.assets?.find(a => a.name === 'checksums-sha256.txt')?.browser_download_url ?? '')
const benchUrl = computed(() =>
  singleRel.value?.assets?.find(a => a.name === 'benchmark-results.json')?.browser_download_url ?? '')

function fmtDate(iso: string): string {
  const d = new Date(iso)
  return isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10)
}

const summary = computed(() => `${matchCount.value} ${t('filter-assets')}`)
defineExpose({ summary })

/* ── positioning: fixed against the anchor, flip up when tight ───── */
const popupEl = ref<HTMLElement | null>(null)
const openDir = ref<'down' | 'up'>('down')

function position() {
  const pop = popupEl.value
  const anchor = props.anchorEl
  if (!pop || !anchor) return
  const r = anchor.getBoundingClientRect()
  const width = Math.min(r.width, window.innerWidth - 24)
  pop.style.left = Math.max(12, Math.min(r.left, window.innerWidth - width - 12)) + 'px'
  pop.style.width = width + 'px'
  const spaceBelow = window.innerHeight - r.bottom - 12
  const spaceAbove = r.top - 12
  if (spaceBelow < 260 && spaceAbove > spaceBelow) {
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

/* ── close: outside mousedown (popup AND anchor excluded), Esc,
      page scroll (popup-internal scroll keeps it open), resize ───── */
function onDocDown(e: MouseEvent) {
  const pop = popupEl.value
  const anchor = props.anchorEl
  const t = e.target as Node | null
  if (!t) return
  if (pop && pop.contains(t)) return
  if (anchor && anchor.contains(t)) return
  emit('close')
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
function onScroll(e: Event) {
  const t = e.target
  const pop = popupEl.value
  /* scroll targets are Elements (scrollers) — guard for Node so a
     synthetic event targeted at window can't crash the handler */
  if (pop && t instanceof Node && pop.contains(t)) return // popup-internal scroll — keep open
  emit('close')
}
function onResize() { emit('close') }

watch(() => props.open, (v) => {
  if (v) {
    nextTick(() => position())
    document.addEventListener('mousedown', onDocDown)
    document.addEventListener('keydown', onKey)
    window.addEventListener('scroll', onScroll, { capture: true, passive: true })
    window.addEventListener('resize', onResize)
  } else {
    document.removeEventListener('mousedown', onDocDown)
    document.removeEventListener('keydown', onKey)
    window.removeEventListener('scroll', onScroll, { capture: true })
    window.removeEventListener('resize', onResize)
  }
})
onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocDown)
  document.removeEventListener('keydown', onKey)
  window.removeEventListener('scroll', onScroll, { capture: true })
  window.removeEventListener('resize', onResize)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="fpop">
      <div v-if="open" ref="popupEl" class="fpopup" role="dialog" :aria-label="t('panel-filter-title')"
           :class="openDir" data-wheel-lock data-wheel-lock-hard>
        <div class="fpop-filters">
          <div class="fgroup">
            <span class="fg-label">{{ t('filter-release-type') }}</span>
            <button v-for="ft in TYPE_OPTS" :key="ft" type="button" class="chip"
                    :class="{ on: fType.includes(ft) }" :aria-pressed="fType.includes(ft)"
                    @click="fType = toggle(fType, ft)">
              {{ ft === 'release' ? t('filter-stable') : t('rel-prerelease') }}
            </button>
          </div>
          <div class="fgroup">
            <span class="fg-label">{{ t('filter-versions') }}</span>
            <span class="chip-hint">{{ t('filter-versions-hint') }}</span>
            <button v-for="r in releases" :key="r.tag_name" type="button" class="chip"
                    :class="{ on: fVersions.includes(r.tag_name) }" :aria-pressed="fVersions.includes(r.tag_name)"
                    @click="fVersions = toggle(fVersions, r.tag_name)">
              {{ r.tag_name }}
              <span v-if="r.prerelease" class="prerelease-badge">{{ t('rel-prerelease') }}</span>
            </button>
          </div>
          <div class="fgroup">
            <span class="fg-label">{{ t('filter-os') }}</span>
            <button v-for="os in OS_ORDER" :key="os" type="button" class="chip"
                    :class="{ on: fOs.includes(os) }" :aria-pressed="fOs.includes(os)"
                    @click="fOs = toggle(fOs, os)">{{ t(OS_KEYS[os]) }}</button>
          </div>
          <div class="fgroup">
            <span class="fg-label">{{ t('filter-arch') }}</span>
            <button v-for="arch in ARCH_ORDER" :key="arch" type="button" class="chip"
                    :class="{ on: fArch.includes(arch) }" :aria-pressed="fArch.includes(arch)"
                    @click="fArch = toggle(fArch, arch)">{{ arch }}</button>
          </div>
          <div class="fgroup">
            <span class="fg-label">{{ t('filter-kind') }}</span>
            <button v-for="kind in KIND_OPTS" :key="kind" type="button" class="chip"
                    :class="{ on: fKind.includes(kind) }" :aria-pressed="fKind.includes(kind)"
                    @click="fKind = toggle(fKind, kind)">{{ kind === 'cli' ? 'CLI' : 'LIB' }}</button>
          </div>
        </div>

        <div class="fpop-results">
          <template v-if="results.length">
            <div v-for="ver in results" :key="ver.release.tag_name" class="ver-group">
              <div v-if="results.length > 1" class="ver-head">
                <b>{{ ver.release.tag_name }}</b>
                <span v-if="ver.release.prerelease" class="prerelease-badge">{{ t('rel-prerelease') }}</span>
                <span class="vs-date">{{ fmtDate(ver.release.published_at) }}</span>
              </div>
              <div v-for="g in ver.groups" :key="g.arch" class="arch-group">
                <h4 class="arch-head">{{ g.arch }}</h4>
                <div v-for="row in g.rows" :key="row.os" class="os-row">
                  <span class="os-name">{{ t(OS_KEYS[row.os]) }}</span>
                  <span v-if="row.cli" class="tag-kind">CLI</span>
                  <a v-if="row.cli" :href="row.cli.browser_download_url" :title="row.cli.name">{{ row.cli.name }}</a>
                  <span v-if="row.lib" class="tag-kind lib">LIB</span>
                  <a v-if="row.lib" :href="row.lib.browser_download_url" :title="row.lib.name">{{ row.lib.name }}</a>
                </div>
              </div>
            </div>
          </template>
          <div v-else class="fpop-empty">
            <p>{{ t('filter-empty') }}</p>
            <button type="button" class="chip" @click="resetFilters">{{ t('filter-reset') }}</button>
          </div>
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
