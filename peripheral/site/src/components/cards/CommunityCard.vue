<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSiteData } from '../../data'

const { t } = useI18n()
const siteData = useSiteData()

const REPO_URL = 'https://github.com/Nekolio/DoLogger'
const contributors = computed(() => siteData.value?.contributors ?? [])
const repo = computed(() => siteData.value?.repo)
/* the top contributors — surplus rows loop-scroll inside the card */
const visible = computed(() => contributors.value.slice(0, 6))
</script>

<template>
  <div>
    <ul v-if="contributors.length">
      <li v-for="(c, i) in visible" :key="c.login" class="contrib-row" :style="{ '--i': i }">
        <img v-if="c.avatar_url" class="avatar" :src="c.avatar_url" :alt="c.login" loading="lazy" />
        <span v-else class="icon"><svg class="icon"><use href="./assets/icons.svg#icon-github"></use></svg></span>
        <a :href="c.html_url || REPO_URL">@{{ c.login || '?' }}</a>
        <span v-if="c.contributions" class="cnt">{{ c.contributions }} {{ t('comm-commit') }}</span>
      </li>
    </ul>
    <div v-else>{{ t('comm-empty') }}</div>
    <div class="repo-stats">
      <span class="stat">{{ t('comm-stars') }}: <b>{{ repo?.stargazers_count != null ? repo.stargazers_count : '—' }}</b></span>
      <span class="stat">{{ t('comm-forks') }}: <b>{{ repo?.forks_count != null ? repo.forks_count : '—' }}</b></span>
      <span class="stat">{{ t('comm-license') }}: <b>Apache-2.0 OR MIT</b></span>
      <span class="stat">{{ t('comm-ci') }}: <a :href="REPO_URL + '/actions'">GitHub Actions</a></span>
    </div>

    <div class="card-caption">{{ t('comm-detail-title') }}</div>
  </div>
</template>
