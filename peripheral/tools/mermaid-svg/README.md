# Mermaid SVG Renderer

This maintainer-only tool renders the authoritative Mermaid sources under
`docs/assets/mmd/` into generated assets under `docs/assets/svg/`.

```text
docs/assets/mmd/*.mmd  ──>  pretty-mermaid  ──>  docs/assets/svg/*.svg
```

Run from the repository root:

```bash
node peripheral/tools/mermaid-svg/render_architecture.mjs
```

Set `MERMAID_THEME` to override the default `github-light` theme. If the
pretty-mermaid skill is installed elsewhere, set `PRETTY_MERMAID_RENDERER` to
its `scripts/render.mjs` path.

Rules:

- Edit `.mmd` sources, never hand-edit generated SVG files.
- Do not import this tool from runtime, CLI, plugins, site, or build code.
- Review the Mermaid source and generated SVG together when changing a public
  architecture diagram.
