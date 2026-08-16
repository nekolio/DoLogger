<script setup lang="ts">
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const sinks = [
  'Console', 'File', 'Callback', 'Kafka', 'Syslog', 'Webhook',
  'SQLite', 'WORM', 'Security', 'Shared Memory', 'OTel'
]
</script>

<template>
  <div>
    <div class="sink-tags">
      <span v-for="(name, i) in sinks" :key="name" :style="{ '--i': i }">{{ name }}</span>
    </div>
    <div class="sinks-note">{{ t('sinks-note') }}</div>

    <!-- fan-out: one core → every sink, exactly how the pipeline ends.
         A light pulse travels the trunk branch by branch, forever. -->
    <div class="fan-native" role="img" :aria-label="t('sinks-detail')">
      <div class="fan-core">{{ t('sinks-core') }}</div>
      <div class="fan-trunk"></div>
      <div class="fan-branches">
        <span v-for="(name, i) in sinks" :key="name" class="fan-branch" :style="{ '--i': i }">{{ name }}</span>
      </div>
    </div>
    <div class="card-caption">{{ t('sinks-detail') }}</div>
  </div>
</template>
