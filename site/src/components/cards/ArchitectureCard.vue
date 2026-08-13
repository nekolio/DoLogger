<script setup lang="ts">
import { useI18n } from 'vue-i18n'

defineProps<{ expanded?: boolean }>()
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
        <span :style="{ '--i': i }">{{ stage }}</span>
      </template>
    </div>
    <a class="card-link" :href="WIKI_URL + '/en_US-ArchitectureReference'">{{ t('arch-link') }}</a>

    <div v-if="expanded" class="card-detail">
      <img class="arch-img" src="./assets/architecture.svg" :alt="t('arch-detail')" loading="lazy" />
      <div class="card-caption">{{ t('arch-detail') }}</div>
    </div>
  </div>
</template>
