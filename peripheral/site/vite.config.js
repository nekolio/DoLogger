import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// base: './' — the site is served from a subpath
// (https://nekolio.github.io/DoLogger/), so every asset URL must be
// relative to the page, not absolute.
//
// All SVG assets (icons.svg, hero.svg, …) live in public/ and are
// referenced as runtime URLs ("./assets/…"). The SFC compiler would
// otherwise rewrite static href/src (including <use href>) into module
// imports that vite resolves against src/assets/*. Disabling needs an
// empty-tags object, not `false`: plugin-vue v6 converts `false` back
// into the default tag list, while an explicit `tags` object replaces
// the defaults entirely (every list empty → no rewrite).
const noAssetTransform = { video: [], source: [], img: [], image: [], use: [] }
export default defineConfig({
  base: './',
  plugins: [vue({ template: { transformAssetUrls: noAssetTransform } })],
  build: { outDir: 'dist' }
})
