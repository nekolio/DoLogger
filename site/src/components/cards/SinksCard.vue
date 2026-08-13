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
      <span v-for="name in sinks" :key="name">{{ name }}</span>
    </div>
    <div class="sinks-note">{{ t('sinks-note') }}</div>
  </div>
</template>

<style scoped>
.sinks-note {
  margin-top: 0.8rem;
  font-size: 0.8rem;
  opacity: 0.6;
}
</style>
