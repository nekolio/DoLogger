<script setup lang="ts">
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const WIKI_URL = 'https://github.com/Nekolio/DoLogger/wiki'
const stages = ['pre-filter', 'filter', 'field provider', 'assembly', 'process', 'format', 'sink fan-out']
</script>

<template>
  <div>
    <div class="arch-hot">{{ t('arch-hot') }}</div>
    <div class="arch-flow">
      <template v-for="(stage, i) in stages" :key="stage">
        <span v-if="i > 0" class="sep">→</span>
        <span>{{ stage }}</span>
      </template>
    </div>
    <a class="card-link" :href="WIKI_URL + '/en_US-ArchitectureReference'">{{ t('arch-link') }}</a>
  </div>
</template>

<style scoped>
.arch-hot {
  font-size: 0.8rem;
  line-height: 1.6;
  margin-bottom: 0.5rem;
}
</style>
