<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import PerformanceCard from './cards/PerformanceCard.vue'
import SecurityCard from './cards/SecurityCard.vue'
import SinksCard from './cards/SinksCard.vue'
import ArchitectureCard from './cards/ArchitectureCard.vue'
import ReleasesCard from './cards/ReleasesCard.vue'
import CommunityCard from './cards/CommunityCard.vue'

const { t } = useI18n()
const REPO_URL = 'https://github.com/Nekolio/DoLogger'

/* card spotlight */
function onMove(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  const rect = card.getBoundingClientRect()
  const x = (e.clientX - rect.left) / rect.width * 100
  const y = (e.clientY - rect.top) / rect.height * 100
  card.style.setProperty('--mx', x + '%')
  card.style.setProperty('--my', y + '%')
  const rotateX = (y - 50) * 0.08
  const rotateY = (x - 50) * -0.08
  card.style.transform = `perspective(600px) rotateX(${rotateX}deg) rotateY(${rotateY}deg) translateY(-4px)`
}
function onLeave(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  card.style.transform = ''
  card.style.setProperty('--mx', '50%')
  card.style.setProperty('--my', '50%')
}
</script>

<template>
  <section class="page" id="page3">
    <div class="container">
      <h2>
        <svg class="icon"><use href="./assets/icons.svg#icon-cubes"></use></svg>
        {{ t('project-overview') }}
      </h2>

      <div class="grid">
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-gauge"></use></svg> {{ t('card-perf') }}</h3>
          <div class="card-body"><PerformanceCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-shield"></use></svg> {{ t('card-sec') }}</h3>
          <div class="card-body"><SecurityCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-plug"></use></svg> {{ t('card-sinks') }}</h3>
          <div class="card-body"><SinksCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-branch"></use></svg> {{ t('card-arch') }}</h3>
          <div class="card-body"><ArchitectureCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-tag"></use></svg> {{ t('card-rel') }}</h3>
          <div class="card-body"><ReleasesCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-users"></use></svg> {{ t('card-comm') }}</h3>
          <div class="card-body"><CommunityCard /></div>
        </div>
      </div>

      <footer class="site-footer">
        <a :href="REPO_URL" target="_blank" rel="noopener">
          <svg class="icon"><use href="./assets/icons.svg#icon-github"></use></svg> Nekolio/DoLogger
        </a>
        <span>·</span>
        <span>{{ t('footer-license') }}</span>
        <span>·</span>
        <a href="mailto:nekoliowork+DoLogger@gmail.com">nekoliowork+DoLogger@gmail.com</a>
      </footer>
    </div>
  </section>
</template>
