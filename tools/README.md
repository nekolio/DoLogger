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
  `Docs/assets/`), it regenerates it — it never replaces it.

## Tools

### `hero-svg/` — regenerate `Docs/assets/hero.svg`

Regenerates the animated CRT-boot hero image used in the READMEs and the
landing page. Pure decoration: the image has no effect on how DoLogger runs.

```
python3 tools/hero-svg/hero_gen.py
```

- Writes `Docs/assets/hero.svg` (source of truth).
- Also syncs `site/public/assets/hero.svg` so local site builds never ship a
  stale copy (`scripts/build-site.sh` re-copies the Docs copy at CI time).
- Output is deterministic: with unchanged inputs, regeneration is a no-op.
- Requires only the Python 3 standard library.

Regenerate when the hero's text/visuals change (e.g. the typed lines in the
`LINES` table at the top of `hero_gen.py`).
