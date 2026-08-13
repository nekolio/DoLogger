/* cursor.ts — shared cyber-cursor preference.
 *
 * The custom cursor is ON by default on capable devices; the top-bar
 * toggle flips it back to the native pointer (persisted per visitor).
 * Module-level ref so App.vue (button) and CyberCursor.vue (engine)
 * share one state.
 */
import { ref, type Ref } from 'vue'

const enabled = ref(true)
try {
  const saved = localStorage.getItem('dologger:cursor')
  if (saved === '0' || saved === 'false') enabled.value = false
} catch { /* private mode */ }

export function useCursorEnabled(): Ref<boolean> {
  return enabled
}

export function setCursorEnabled(v: boolean): void {
  enabled.value = v
  try { localStorage.setItem('dologger:cursor', v ? '1' : '0') } catch { /* private mode */ }
}
