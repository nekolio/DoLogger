<script setup lang="ts">
/* PageNav — right-edge navigation. Notched-diamond chips (cyberpunk
 * clipped corners) that expand into labeled pills on hover/active, so
 * the current section is always readable. Also shows which page the
 * wheel/keyboard navigation considers active. */
import { useI18n } from 'vue-i18n'

defineProps<{ count: number; active: number }>()
const emit = defineEmits<{ (e: 'go', i: number): void }>()
const { t } = useI18n()
const LABELS = ['nav-hero', 'nav-demo', 'nav-overview']
const NUMS = ['01 ·', '02 ·', '03 ·']
</script>

<template>
  <nav class="page-nav" aria-label="sections">
    <button v-for="i in count" :key="i"
            type="button"
            :class="{ active: active === i - 1 }"
            :aria-label="t(LABELS[i - 1])"
            :aria-current="active === i - 1 ? 'true' : undefined"
            @click="emit('go', i - 1)">
      <span class="chip"></span>
      <span class="label"><b>{{ NUMS[i - 1] }}</b>{{ t(LABELS[i - 1]) }}</span>
    </button>
  </nav>
</template>
