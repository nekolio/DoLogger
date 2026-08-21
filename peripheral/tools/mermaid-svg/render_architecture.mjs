#!/usr/bin/env node

/**
 * Render every architecture Mermaid source into the generated SVG directory.
 *
 * The repository keeps `.mmd` files authoritative. This wrapper deliberately
 * delegates parsing/rendering to the pretty-mermaid skill instead of adding a
 * runtime dependency to DoLogger. Set PRETTY_MERMAID_RENDERER when the skill
 * is installed outside the usual agent directories.
 */

import { existsSync, mkdirSync, readdirSync } from 'node:fs'
import { homedir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const sourceDir = join(root, 'docs', 'assets', 'mmd')
const outputDir = join(root, 'docs', 'assets', 'svg')
const theme = process.env.MERMAID_THEME || 'github-light'
const rendererCandidates = [
  process.env.PRETTY_MERMAID_RENDERER,
  join(homedir(), '.agents', 'skills', 'pretty-mermaid', 'scripts', 'render.mjs'),
  join(homedir(), '.cc-switch', 'skills', 'pretty-mermaid', 'scripts', 'render.mjs')
].filter(Boolean)
const renderer = rendererCandidates.find(existsSync)

if (!renderer) {
  console.error('pretty-mermaid renderer not found; set PRETTY_MERMAID_RENDERER')
  process.exit(2)
}

mkdirSync(outputDir, { recursive: true })
const sources = readdirSync(sourceDir)
  .filter((name) => name.endsWith('.mmd'))
  .sort()

if (sources.length === 0) {
  console.error(`No Mermaid sources found in ${sourceDir}`)
  process.exit(3)
}

for (const source of sources) {
  const input = join(sourceDir, source)
  const output = join(outputDir, `${basename(source, '.mmd')}.svg`)
  const result = spawnSync(process.execPath, [
    renderer,
    '--input', input,
    '--output', output,
    '--format', 'svg',
    '--theme', theme
  ], { stdio: 'inherit' })
  if (result.status !== 0) {
    process.exit(result.status ?? 1)
  }
  console.log(`rendered ${source} -> ${output}`)
}
