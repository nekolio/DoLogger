<script setup lang="ts">
/* PageNav — right-edge dot navigation. Also shows which page the
 * wheel/keyboard navigation considers active. */
defineProps<{ count: number; active: number }>()
const emit = defineEmits<{ (e: 'go', i: number): void }>()
</script>

<template>
  <nav class="page-nav" aria-label="sections">
    <button v-for="i in count" :key="i"
            type="button"
            :class="{ active: active === i - 1 }"
            :aria-label="'section ' + i"
            @click="emit('go', i - 1)">
      <span></span>
    </button>
  </nav>
</template>
