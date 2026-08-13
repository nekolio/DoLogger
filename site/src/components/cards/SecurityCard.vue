<script setup lang="ts">
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const items = ['sec-chain', 'sec-sandbox', 'sec-trust', 'sec-worm', 'sec-priority']
</script>

<template>
  <ul>
    <li v-for="key in items" :key="key">{{ t(key) }}</li>
  </ul>
</template>
