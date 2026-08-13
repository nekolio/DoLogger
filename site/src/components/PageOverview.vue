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

/* Steam-trading-card style tilt: the card rotates up to ±7° toward the
 * cursor with a moving glare (--mx/--my radial), and the content pops
 * forward on the Z axis. Disabled on touch / reduced-motion via CSS. */
function onMove(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  const rect = card.getBoundingClientRect()
  const px = (e.clientX - rect.left) / rect.width
  const py = (e.clientY - rect.top) / rect.height
  card.style.setProperty('--mx', px * 100 + '%')
  card.style.setProperty('--my', py * 100 + '%')
  const rotateX = (0.5 - py) * 14
  const rotateY = (px - 0.5) * 14
  card.style.transition = 'transform 70ms linear' // snappy while tracking
  card.style.transform = `perspective(900px) rotateX(${rotateX.toFixed(2)}deg) rotateY(${rotateY.toFixed(2)}deg) scale3d(1.02, 1.02, 1.02)`
}
function onLeave(e: MouseEvent) {
  const card = e.currentTarget as HTMLElement
  card.style.transition = '' // falls back to the 0.45s settle in CSS
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
          <div class="card-body card-body-scroll"><PerformanceCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-shield"></use></svg> {{ t('card-sec') }}</h3>
          <div class="card-body card-body-scroll"><SecurityCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-plug"></use></svg> {{ t('card-sinks') }}</h3>
          <div class="card-body card-body-scroll"><SinksCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-branch"></use></svg> {{ t('card-arch') }}</h3>
          <div class="card-body card-body-scroll"><ArchitectureCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-tag"></use></svg> {{ t('card-rel') }}</h3>
          <div class="card-body card-body-scroll"><ReleasesCard /></div>
        </div>
        <div class="card" @mousemove="onMove" @mouseleave="onLeave">
          <h3><svg class="icon"><use href="./assets/icons.svg#icon-users"></use></svg> {{ t('card-comm') }}</h3>
          <div class="card-body card-body-scroll"><CommunityCard /></div>
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
