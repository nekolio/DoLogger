<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSiteData } from '../../data'

const { t } = useI18n()
const siteData = useSiteData()

const RELEASES_URL = 'https://github.com/Nekolio/DoLogger/releases'

const releases = computed(() => siteData.value?.releases ?? [])
function fmtDate(iso: string): string {
  const d = new Date(iso)
  return isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10)
}
</script>

<template>
  <div>
    <ul v-if="releases.length">
      <li v-for="r in releases.slice(0, 5)" :key="r.tag_name" class="release-row">
        <a :href="r.html_url || RELEASES_URL">{{ r.tag_name || r.name || '?' }}</a>
        <span v-if="r.prerelease" class="prerelease-badge">{{ t('rel-prerelease') }}</span>
        <span class="date">{{ fmtDate(r.published_at) }}</span>
      </li>
    </ul>
    <div v-else>{{ t('rel-empty') }}</div>
    <a class="card-link" :href="RELEASES_URL">{{ t('rel-all') }}</a>
  </div>
</template>
