/// <reference types="vite/client" />

/* Plain-`.ts` files import `.vue` SFCs (src/main.ts). vue-tsc resolves
 * SFC-to-SFC imports itself via Volar; this shim covers the `.ts → .vue`
 * edge so `vue-tsc --noEmit` stays green. */
declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<{}, {}, any>
  export default component
}
