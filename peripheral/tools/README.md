# DoLogger Auxiliary Tools

> **These tools are NOT part of the DoLogger runtime, build, or CI.**
> They exist only for maintainers' convenience — regenerating a decorative
> image, formatting a file, or otherwise doing something that has nothing to
> do with how DoLogger works. Nothing here is compiled, linked, tested, or
> shipped. **Deleting this directory has zero effect on the project.**

This is the convention many open-source projects use for tooling that is
*"we had to keep this somewhere"*: it stays in the repository so it is not
lost, but it is clearly fenced off from everything the project actually is.
If you read this README and think "this has nothing to do with DoLogger" —
that is exactly right.

## Convention

- Each tool lives in its own subdirectory: `tools/<tool-name>/`.
- Each tool is self-contained: no code here may be `use`d or `#include`d by
  the core, CLI, plugins, site, or any build script.
- Only commit things that are *useful indefinitely*. Temporary one-off
  scripts belong in your scratch space, not here — they will be deleted
  eventually, and this directory should not be a graveyard.
- If a tool produces output that the project *does* use (e.g. an SVG in
  `docs/assets/`), it regenerates it — it never replaces it.

## Tools

### `hero-svg/` — regenerate `docs/assets/svg/hero.svg`

Regenerates the animated CRT-boot hero image used in the READMEs and the
landing page. Pure decoration: the image has no effect on how DoLogger runs.

```
python3 tools/hero-svg/hero_generator.py
```

- Writes `docs/assets/svg/hero.svg` only — the single source of truth. The site
  references it at build time (via the Vite plugin in `vite.config.js`)
  instead of keeping its own copy.
- Output is deterministic: with unchanged inputs, regeneration is a no-op.
- Requires only the Python 3 standard library.
- Timing and animation are computed dynamically from the `LINES` table and
  the `Cargo.toml` version — no hardcoded per-line delays or cursor distances.

Regenerate when the hero's text/visuals change (e.g. the typed lines in the
`LINES` table at the top of `hero_generator.py`) or when the project version
in `Cargo.toml` changes.

### `mermaid-svg/` — render `docs/assets/mmd/*.mmd` into `docs/assets/svg/*.svg`

The architecture diagrams are generated from Mermaid sources (the `.mmd`
files are the source of truth, the `.svg` files are build output):

- `docs/assets/mmd/architecture.mmd` — English diagram
- `docs/assets/mmd/architecture-zh.mmd` — Chinese diagram

Render them with the `pretty-mermaid` skill (installed via cc-switch):

```
node peripheral/tools/mermaid-svg/render_architecture.mjs
```

- Never hand-edit the SVG — edit the `.mmd` and re-render.
- `docs/assets/svg/architecture-zh.svg` is what `README.zh_CN.md` embeds; the
  English README embeds `docs/assets/svg/architecture.svg`.
- The `pretty-mermaid` skill's flowchart renderer is single-line label
  only (`<br/>` is not supported); keep each node label on one line.
- The renderer needs a Windows fix to load `beautiful-mermaid` (dynamic
  `import()` must use a `file://` URL); both `~/.agents` and
  `~/.cc-switch` copies of `render.mjs` already carry the fix.
