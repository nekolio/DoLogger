#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Generate Docs/assets/hero.svg - animated CRT boot hero, pure SMIL.

AUXILIARY TOOL - NOT part of the DoLogger runtime, build, or CI.
This script only regenerates a decorative README/landing-page image.
Deleting it (or the whole tools/ directory) has ZERO effect on the
project. See ../README.md.

Effects (SMIL only, no scripts - runs in GitHub READMEs):
  - per-character typing with a block cursor riding the text edge
  - cyberpunk CRT: frame energy sweep, cyan/magenta accents, HUD hairline,
    bezel microtext, phosphor-profile scanlines (soft falloff, half-phase
    aligned inside the screen), moire second layer, aperture grille,
    vignette
  - glitches THROUGHOUT the cycle (typing included, not just the hold):
    fast jitter, RGB channel split, ghosting, screen tearing, glitch lines
  - mirrored power-on/off with real CRT deflection physics: the raster
    collapses vertically into a white-hot scanline, the line shrinks
    horizontally into the screen-center point (with a white surge and
    phosphor afterglow); power-on mirrors it

Usage:
    python3 tools/hero-svg/hero_gen.py

Output is deterministic (all randomness is seeded by fixed timeline values),
so regenerating produces a byte-identical file unless the LINES/data above
are edited. Writes Docs/assets/hero.svg and keeps site/public/assets/hero.svg
in sync (both are the same brand image; the site build also re-copies the
Docs copy, see scripts/build-site.sh).
"""

import math
import os
import random
import shutil
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
OUT = os.path.join(ROOT, "Docs", "assets", "hero.svg")
SITE_OUT = os.path.join(ROOT, "site", "public", "assets", "hero.svg")

CYCLE = 10.0
X0 = 44                      # text left edge
CX, CY = 460.0, 172.0        # screen center - collapse anchor AND star anchor

# (clip id, colored prefix (or None), text, baseline y, font size,
#  typing start s, seconds per char, kind)
LINES = [
    dict(cid="c1", pre=None,        text="$ dologctl run --config dologger.toml", y=64,  size=15, s=0.34, rate=0.020, kind="log"),
    dict(cid="c2", pre="[INFO] ",   text="Hello DoLogger",                       y=92,  size=15, s=1.22, rate=0.020, kind="log"),
    dict(cid="c3", pre="[PLUGIN] ", text="4 sandboxed · trust BLUE",             y=120, size=15, s=1.82, rate=0.020, kind="log"),
    dict(cid="c4", pre="[AUDIT] ",  text="ed25519 chain armed",                  y=148, size=15, s=2.64, rate=0.020, kind="log"),
    dict(cid="c5", pre="[ OK ]  ",  text="7-stage pipeline online",              y=176, size=15, s=3.36, rate=0.020, kind="log"),
    dict(cid="c6", pre=None,        text="DoLogger",                             y=232, size=56, s=4.20, rate=0.070, kind="wordmark"),
    dict(cid="c7", pre=None,        text="next-gen secure logging · ed25519 @ lock-free speed", y=266, size=14, s=4.96, rate=0.020, kind="log"),
]
PREFIX_COLOR = {"c2": "#86C97F", "c3": "#8A9BFF", "c4": "#7FC7D9", "c5": "#86C97F"}

# micro bursts land DURING typing; macro bursts in the hold phase
MICRO = [(0.70, 0.78), (1.45, 1.52), (2.20, 2.28), (3.00, 3.08),
         (3.66, 3.73), (4.46, 4.53), (5.42, 5.49), (6.06, 6.13)]
MACRO = [(7.30, 7.50), (8.30, 8.50), (8.95, 9.10)]
GLITCH_DT = 0.033  # ~30 Hz steps



def full_text(line):
    return (line["pre"] or "") + line["text"]


def kt(t):
    return f"{t / CYCLE:.4f}"


def fmt(v):
    if isinstance(v, float):
        return f"{v:.1f}"
    return str(v)


def anim(attr, pairs, mode="discrete"):
    """pairs: list of (time_seconds, value) -> <animate> on the 10 s master timeline."""
    ks = ";".join(kt(t) for t, _ in pairs)
    vs = ";".join(fmt(v) for _, v in pairs)
    ts = [t for t, _ in pairs]
    assert ts[0] == 0.0 and abs(ts[-1] - CYCLE) < 1e-6, (attr, ts[0], ts[-1])
    assert all(b > a for a, b in zip(ts, ts[1:])), attr
    return (f'<animate attributeName="{attr}" calcMode="{mode}" '
            f'values="{vs}" keyTimes="{ks}" dur="10s" repeatCount="indefinite"/>')


def flicker_windows(windows, levels, off=0.0):
    """Random on/off flicker inside each window; 0 elsewhere."""
    pairs = [(0.0, off)]
    for t0, t1 in windows:
        pairs.append((t0, off))
        rng = random.Random(int(t0 * 10000))
        t, k = t0 + GLITCH_DT, 0
        while t < t1 - 1e-9:
            pairs.append((t, rng.choice(levels) if k % 2 == 0 else off))
            t += GLITCH_DT
            k += 1
        pairs.append((t1, off))
    pairs.append((CYCLE, off))
    return pairs


def jitter_pairs(bursts):
    """Fast random 2D displacement during bursts; 0 otherwise."""
    pairs = [(0.0, "0 0")]
    for t0, t1, amp in bursts:
        pairs.append((t0, "0 0"))
        rng = random.Random(int(t0 * 10000))
        t = t0 + GLITCH_DT
        while t < t1 - 1e-9:
            pairs.append((t, f"{rng.randint(-amp, amp)} {rng.randint(-1, 1)}"))
            t += GLITCH_DT
        pairs.append((t1, "0 0"))
    pairs.append((CYCLE, "0 0"))
    return pairs


def band_y_pairs(windows, default_y):
    """Bands jump to random heights inside their windows."""
    pairs = [(0.0, default_y)]
    for t0, t1 in windows:
        rng = random.Random(int(t0 * 10000))
        pairs.append((t0, default_y))
        t = t0 + GLITCH_DT
        while t < t1 - 1e-9:
            pairs.append((t, rng.randint(60, 240)))
            t += GLITCH_DT
        pairs.append((t1, default_y))
    pairs.append((CYCLE, default_y))
    return pairs


parts = []
ap = parts.append

# ================= defs =================
ap('''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 920 344" font-family="Consolas, Menlo, 'DejaVu Sans Mono', 'Courier New', monospace">
  <!--
    DoLogger boot hero - animated CRT terminal, pure SMIL (no scripts).
    Per-character typing with a block cursor riding the text edge. Cyberpunk
    CRT: frame energy sweep, cyan/magenta accents, soft-profile scanlines
    half-phase aligned INSIDE the screen, moire second layer, aperture
    grille. Glitches fire throughout the whole cycle - typing included -
    via fast jitter, RGB channel split, ghosting and screen tearing.
    Power-on and power-off mirror each other around the screen center with
    real CRT deflection physics: vertical collapse first (raster squashes
    into a bright horizontal line), then horizontal collapse (line shrinks
    into a white-hot dot with a phosphor afterglow); power-on plays the
    same sequence backwards.
    Palette: phosphor cyan/amber/green with a single cyan -> violet ->
    magenta sweep for the wordmark - a coherent neon ramp, not a rainbow.
  -->''')
ap('''  <defs>
    <!-- cyberpunk wordmark ramp: one continuous cyan -> violet -> magenta sweep -->
    <linearGradient id="title" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0"    stop-color="#7FD5FF"/>
      <stop offset="0.45" stop-color="#8A9BFF"/>
      <stop offset="0.78" stop-color="#C792EA"/>
      <stop offset="1"    stop-color="#F472D0"/>
    </linearGradient>
    <!-- flowing light on the wordmark: a comet band sweeps left -> right
         (4.3s-5.8s each cycle, crossing the glyphs ~4.86s-5.48s) right after
         the neon-starter flicker settles - the ignition wave travels the tube
         and the letters light up progressively. The band parks off-glyph the
         rest of the time; the seam jump back to -280 at t=0 is invisible
         because the tube sits at scale 0.02 there. -->
    <linearGradient id="sheen" gradientUnits="userSpaceOnUse" x1="-120" y1="0" x2="120" y2="0">
      <stop offset="0"    stop-color="#DFF7FF" stop-opacity="0"/>
      <stop offset="0.38" stop-color="#DFF7FF" stop-opacity="0"/>
      <stop offset="0.50" stop-color="#DFF7FF" stop-opacity="0.95"/>
      <stop offset="0.55" stop-color="#DFF7FF" stop-opacity="0.5"/>
      <stop offset="0.68" stop-color="#DFF7FF" stop-opacity="0"/>
      <stop offset="1"    stop-color="#DFF7FF" stop-opacity="0"/>
      <animateTransform attributeName="gradientTransform" type="translate" calcMode="linear"
        values="-280 0;-280 0;480 0;480 0" keyTimes="0;0.43;0.58;1"
        dur="10s" repeatCount="indefinite"/>
    </linearGradient>
    <radialGradient id="vignette" cx="0.5" cy="0.45" r="0.8">
      <stop offset="0"    stop-color="#000000" stop-opacity="0"/>
      <stop offset="0.75" stop-color="#000000" stop-opacity="0.12"/>
      <stop offset="1"    stop-color="#000000" stop-opacity="0.55"/>
    </radialGradient>
    <!-- faint magenta rim light (cyberpunk ambience) -->
    <radialGradient id="vneon" cx="0.5" cy="0.45" r="0.85">
      <stop offset="0"    stop-color="#FF2A6D" stop-opacity="0"/>
      <stop offset="0.85" stop-color="#FF2A6D" stop-opacity="0.05"/>
      <stop offset="1"    stop-color="#FF2A6D" stop-opacity="0.11"/>
    </radialGradient>
    <!-- CRT power-off core: white-hot beam dot, cyan bloom, magenta fringe -->
    <radialGradient id="core">
      <stop offset="0"    stop-color="#FFFFFF"/>
      <stop offset="0.35" stop-color="#CFF4FF"/>
      <stop offset="0.6"  stop-color="#7FD5FF" stop-opacity="0.55"/>
      <stop offset="0.8"  stop-color="#F472D0" stop-opacity="0.25"/>
      <stop offset="1"    stop-color="#F472D0" stop-opacity="0"/>
    </radialGradient>
    <!-- power surge during collapse: soft radial bloom centered on the
         screen, so the brightness rise never reads as a white rectangle -
         the edges fall off to nothing -->
    <radialGradient id="surge" cx="0.5" cy="0.5" r="0.72">
      <stop offset="0"    stop-color="#FFFFFF" stop-opacity="0.95"/>
      <stop offset="0.45" stop-color="#E8F4FF" stop-opacity="0.55"/>
      <stop offset="0.75" stop-color="#FFFFFF" stop-opacity="0.12"/>
      <stop offset="1"    stop-color="#FFFFFF" stop-opacity="0"/>
    </radialGradient>
    <!-- hot scanline while the raster is squashed: white core at screen
         center, fading to cyan/magenta toward the screen edges -->
    <linearGradient id="hotline" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0"    stop-color="#7FD5FF" stop-opacity="0"/>
      <stop offset="0.35" stop-color="#7FD5FF" stop-opacity="0.85"/>
      <stop offset="0.5"  stop-color="#FFFFFF"/>
      <stop offset="0.65" stop-color="#F472D0" stop-opacity="0.85"/>
      <stop offset="1"    stop-color="#F472D0" stop-opacity="0"/>
    </linearGradient>
    <!-- neon frame energy sweep: cyan -> magenta along the bezel path -->
    <linearGradient id="frameSweep" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0"    stop-color="#7FD5FF"/>
      <stop offset="0.5"  stop-color="#C792EA"/>
      <stop offset="1"    stop-color="#F472D0"/>
    </linearGradient>
    <!-- scanlines: phosphor-row profile - dark seam with soft falloff, not a
         hard black edge; tiles are half-phase shifted so the first dark row
         sits 2 px INSIDE the screen top, never on the bezel -->
    <linearGradient id="scanGrad" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0"    stop-color="#000000" stop-opacity="0.30"/>
      <stop offset="0.35" stop-color="#000000" stop-opacity="0.02"/>
      <stop offset="0.65" stop-color="#000000" stop-opacity="0.02"/>
      <stop offset="1"    stop-color="#000000" stop-opacity="0.30"/>
    </linearGradient>
    <linearGradient id="scanGrad2" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0"    stop-color="#000000" stop-opacity="0.16"/>
      <stop offset="0.5"  stop-color="#000000" stop-opacity="0"/>
      <stop offset="1"    stop-color="#000000" stop-opacity="0.16"/>
    </linearGradient>
    <pattern id="scan" width="1" height="4" patternUnits="userSpaceOnUse" patternTransform="translate(20,18)">
      <rect width="1" height="4" fill="url(#scanGrad)"/>
    </pattern>
    <pattern id="scan2" width="1" height="5" patternUnits="userSpaceOnUse" patternTransform="translate(20,17)">
      <rect width="1" height="5" fill="url(#scanGrad2)"/>
    </pattern>
    <!-- aperture grille: vertical RGB subpixel stripes, half-phase inside -->
    <pattern id="grille" width="3" height="1" patternUnits="userSpaceOnUse" patternTransform="translate(22,20)">
      <rect width="1" height="1" fill="#FF5050" opacity="0.05"/>
      <rect x="1" width="1" height="1" fill="#50FF50" opacity="0.05"/>
      <rect x="2" width="1" height="1" fill="#5050FF" opacity="0.05"/>
    </pattern>
    <filter id="glow" x="-40%" y="-40%" width="180%" height="180%">
      <feGaussianBlur in="SourceGraphic" stdDeviation="6" result="wide"/>
      <feGaussianBlur in="SourceGraphic" stdDeviation="2.2" result="near"/>
      <feMerge>
        <feMergeNode in="wide"/>
        <feMergeNode in="near"/>
        <feMergeNode in="SourceGraphic"/>
      </feMerge>
    </filter>
    <filter id="bloom" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur in="SourceGraphic" stdDeviation="1.6"/>
    </filter>
    <!-- channel-separated copies for the RGB split glitch -->
    <filter id="fRed" x="-10%" y="-10%" width="120%" height="120%">
      <feColorMatrix in="SourceGraphic" type="matrix"
        values="1 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0"/>
      <feOffset dx="3" dy="0"/>
    </filter>
    <filter id="fCyan" x="-10%" y="-10%" width="120%" height="120%">
      <feColorMatrix in="SourceGraphic" type="matrix"
        values="0 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 1 0"/>
      <feOffset dx="-3" dy="0"/>
    </filter>''')

# ---- per-line typing clip paths (one keyframe per character) ----
# The reveal advance MUST equal the cursor advance (0.55 * size, the real
# Consolas ratio): with the old 0.6 estimate the clip edge ran ~9% ahead per
# character, so on long lines the last characters appeared while the cursor
# was still several characters behind. Both now step at the measured ratio.
for L in LINES:
    cid, y, size, s, rate, kind = L["cid"], L["y"], L["size"], L["s"], L["rate"], L["kind"]
    if kind == "wordmark":
        # the wordmark ignites as a WHOLE (neon starter), no per-char reveal
        continue
    n = len(full_text(L))
    a = round(size * 0.55, 1)
    e = s + n * rate
    # the wordmark clip must generously contain glyphs + skew lean + glow
    # halo on ALL sides: the 12 px blur halo extends past the glyph bbox,
    # and a tight clip cuts the glow with hard edges (seen on the D's
    # bottom-left corner, then on the r's right glow)
    w_full = round(a * n + (28 if kind == "wordmark" else 4), 1)
    pairs = [(0.0, 0), (s, 0)]
    t = s
    for k in range(1, n):
        t += rate
        pairs.append((t, round(a * k, 1)))
    pairs.append((e, w_full))
    pairs.append((CYCLE, w_full))
    if kind == "wordmark":
        # clip box oversized on all four sides so the glow halo never touches
        # a boundary: glyphs span y 192-241, halo +-12 -> y 180-253
        cy, ch = y - 60, 84
        clip_x = X0 - 10
    elif size >= 15:
        cy, ch = y - 14, 20
        clip_x = X0
    else:
        cy, ch = y - 14, 18
        clip_x = X0
    ap(f'''    <clipPath id="{cid}"><rect x="{clip_x}" y="{cy}" width="0" height="{ch}">
      {anim("width", pairs)}
    </rect></clipPath>''')

# ---- screen-tearing band: a 40 px slice of the content, shifted sideways ----
TEAR_WINDOWS = [(4.46, 4.53), (7.30, 7.50)]
ap('''    <clipPath id="tear"><rect x="20" y="0" width="880" height="40">
      {ANIM}
    </rect></clipPath>
  </defs>'''.replace("{ANIM}", anim("y", band_y_pairs(TEAR_WINDOWS, 150))))

# ================= bezel =================
ap('''  <rect x="8" y="8" width="904" height="328" rx="16" fill="#151A24" stroke="#34304A" stroke-width="1"/>
  <rect x="20" y="20" width="880" height="304" rx="8" fill="#0A0E15"/>''')

# ================= tube: mirrored power-on expand / power-off collapse =================
# Real CRT deflection physics, anchored at the screen center (CX, CY):
# power-off: vertical scan collapses FIRST (0.945-0.955, raster squashes
# into a bright horizontal line - scaleY 0.02 = a 6 px band, the same
# height as the hot scanline so the two fuse), the line holds white-hot
# (0.955-0.968), then horizontal scan dies (0.968-0.982, line shrinks into
# the center point); power-on mirrors it: horizontal line first, then
# vertical expansion. A horizontal deflection jitter (discrete translate on
# the content, 0.945-0.982) makes the collapse read as failing deflection
# instead of a clean rectangle shrink.
ap(f'''  <g transform="translate({CX},{CY})">
    <g>
      <animateTransform attributeName="transform" type="scale" calcMode="linear"
        values="0.02 0.02;0.02 0.02;1 0.02;1 1;1 1;1 1;1 0.02;1 0.02;0.02 0.02;0.02 0.02" keyTimes="0;0.004;0.011;0.026;0.93;0.945;0.955;0.968;0.982;1"
        dur="10s" repeatCount="indefinite"/>
      <animate attributeName="opacity" calcMode="linear"
        values="1;1;0;0" keyTimes="0;0.968;0.982;1"
        dur="10s" repeatCount="indefinite"/>
      <g transform="translate(-{CX},-{CY})">
        <rect x="20" y="20" width="880" height="304" rx="8" fill="#0A0E15"/>
        <g>
          <animateTransform attributeName="transform" type="translate" calcMode="discrete"
            values="0 0;0 0;6 0;-5 0;4 0;-6 0;3 0;-4 0;2 0;-2 0;0 0;0 0" keyTimes="0;0.945;0.949;0.953;0.957;0.961;0.965;0.969;0.973;0.977;0.982;1"
            dur="10s" repeatCount="indefinite"/>''')

# ================= content (typed lines + cursors) =================
# brightness dips fire all cycle long, typing included
dips = [0.55, 1.35, 2.15, 3.10, 3.80, 4.55, 5.30, 6.25, 7.05, 8.05, 8.85]
flick = [(0.0, 1)]
for d in dips:
    depth = 0.86 if any(t0 - 0.02 <= d <= t1 + 0.02 for t0, t1 in MACRO) else 0.90
    flick += [(d, 1), (d + 0.02, depth), (d + 0.04, 1)]
flick.append((CYCLE, 1))

jitter_bursts = [(t0, t1, 2) for t0, t1 in MICRO] + [(t0, t1, 3) for t0, t1 in MACRO]

ap('''        <g id="content">
          {FLICK}
          <g>
            {JITTER}'''.replace("{FLICK}", anim("opacity", flick, "linear"))
                       .replace("{JITTER}", anim("transform", jitter_pairs(jitter_bursts), "discrete")))

# typed lines
for L in LINES:
    cid, y, pre, text = L["cid"], L["y"], L["pre"], L["text"]
    if cid == "c1":
        ap(f'''            <g clip-path="url(#c1)">
              <text x="{X0}" y="{y}" font-size="15" fill="#D9A066">$ dologctl run --config dologger.toml</text>
            </g>''')
    elif cid == "c6":
        # upright wordmark, no cursor, no per-char reveal: the WHOLE sign
        # ignites like a neon tube - it comes on too dim (insufficient
        # current), flickers irregularly, cuts out hard, re-ignites, then
        # ramps up slowly to full brightness as the sheen sweep travels the
        # tube. Brightness-only (opacity): glyphs never change size.
        # JetBrains Mono first, monospace fallbacks.
        ap(f'''            <g>
              <animate attributeName="opacity" calcMode="linear"
                values="0;0;0.35;0.15;0.35;0.1;0.35;0;0.25;0.15;0.25;1;1;1"
                keyTimes="0;0.42;0.43;0.435;0.44;0.445;0.45;0.455;0.465;0.475;0.48;0.56;0.93;1"
                dur="10s" repeatCount="indefinite"/>
              <text x="{X0}" y="{y}" font-size="56" font-weight="700"
                    font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace"
                    fill="url(#title)" filter="url(#glow)">DoLogger</text>
              <text x="{X0}" y="{y}" font-size="56" font-weight="700"
                    font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace"
                    fill="url(#sheen)" opacity="0.6">DoLogger</text>
            </g>''')
    elif cid == "c7":
        ap(f'''            <g clip-path="url(#c7)">
              <text x="{X0}" y="{y}" font-size="14" fill="#9FB0C9">next-gen secure logging · ed25519 @ lock-free speed</text>
            </g>''')
    else:
        ap(f'''            <g clip-path="url(#{cid})">
              <text x="{X0}" y="{y}" font-size="15"><tspan fill="{PREFIX_COLOR[cid]}">{pre}</tspan><tspan fill="#B8C2CE">{text}</tspan></text>
            </g>''')

# ---- cursor A: log lines + tagline, steps per char, blinks at the end ----
# advance 0.55 * size is the real Consolas ratio (measured: line 7 ends at
# x=436, not 472 as the old 0.6 estimate produced - the cursor used to rest
# ~36 px past the last character). Cursor B below is sized to the wordmark.
cx = [(0.0, X0)]
for L in LINES:
    if L["kind"] != "log":
        continue
    s = L["s"]
    cx.append((s, X0))
    n = len(full_text(L))
    a = round(L["size"] * 0.55, 1)
    t = s
    for k in range(1, n + 1):
        t += L["rate"]
        cx.append((t, round(X0 + a * k, 1)))
a7 = round(LINES[6]["size"] * 0.55, 1)
cx.append((CYCLE, round(X0 + a7 * len(full_text(LINES[6])) + 2, 1)))

cy = [(0.0, 50), (0.34, 50), (1.22, 78), (1.82, 106), (2.64, 134), (3.36, 162),
      (4.96, 252), (CYCLE, 252)]

cop = [(0.0, 0), (0.34, 1), (1.08, 0), (1.22, 1), (1.64, 0), (1.82, 1),
       (2.48, 0), (2.64, 1), (3.18, 0), (3.36, 1), (3.98, 0), (4.96, 1)]
t = 6.02
while t < 9.30 - 1e-9:
    cop.append((t, 1))
    t += 0.25
    cop.append((min(t, 9.30), 0))
    t += 0.25
cop += [(9.30, 0), (CYCLE, 0)]

ap(f'''            <rect x="{X0}" y="50" width="9" height="20" fill="#A8E6D8" opacity="0.92">
              {anim("x", cx)}
              {anim("y", cy)}
              {anim("opacity", cop)}
            </rect>''')

# ---- wordmark line has NO cursor: it ignites like a neon tube (starter
# flicker + sheen sweep) instead. A block on the glowing 56 px glyphs reads
# as a slab no matter the draw order, so the ignition carries the typing. ----
ap('''          </g>
        </g>''')

# ================= phosphor bloom copy (permanent soft self-glow) =================
ap('''        <use href="#content" filter="url(#bloom)" opacity="0.30"/>''')

# ================= screen tearing band (shifted content slice) =================
ap('''        <g>
          {OP}
          <g clip-path="url(#tear)">
            <use href="#content" transform="translate(14,0)" opacity="0.7"/>
          </g>
        </g>'''.replace("{OP}", anim("opacity", flicker_windows(TEAR_WINDOWS, [0.7, 0.5]))))

# ================= stray glitch lines (one per color, micro + macro) =================
for i, (color, windows) in enumerate([
        ("#E8F4FF", [(0.70, 0.78), (7.30, 7.50)]),
        ("#F472D0", [(3.00, 3.08), (8.30, 8.50)]),
        ("#C792EA", [(5.42, 5.49), (8.95, 9.10)])]):
    ap(f'''        <rect x="20" y="{120 + 40 * i}" width="880" height="2" fill="{color}">
          {anim("opacity", flicker_windows(windows, [0.45, 0.25], 0.0))}
          {anim("y", band_y_pairs(windows, 120 + 40 * i))}
        </rect>''')

# ================= RGB channel split + ghosting (glitch only) =================
ap('''        <g>
          {OP}
          <use href="#content" filter="url(#fRed)"/>
        </g>'''.replace("{OP}", anim("opacity", flicker_windows([(2.20, 2.28), (7.30, 7.50)], [0.40, 0.25]))))
ap('''        <g>
          {OP}
          <use href="#content" filter="url(#fCyan)"/>
        </g>'''.replace("{OP}", anim("opacity", flicker_windows([(3.66, 3.73), (8.30, 8.50)], [0.35, 0.22]))))
ap('''        <g>
          {OP}
          <use href="#content" filter="url(#bloom)" transform="translate(10,0)" opacity="0.28"/>
        </g>'''.replace("{OP}", anim("opacity", flicker_windows([(1.45, 1.52), (8.95, 9.10)], [0.28, 0.18]))))

# ================= CRT surface: scanlines, grille, vignettes, refresh bands =================
ap('''        <rect x="20" y="20" width="880" height="304" fill="url(#scan)" opacity="0.55"/>
        <rect x="20" y="20" width="880" height="304" fill="url(#scan2)" opacity="0.50"/>
        <rect x="20" y="20" width="880" height="304" fill="url(#grille)"/>
        <rect x="20" y="20" width="880" height="304" fill="url(#vignette)"/>
        <rect x="20" y="20" width="880" height="304" fill="url(#vneon)"/>
        <g opacity="0.06">
          <rect x="20" y="20" width="880" height="34" fill="#9FE8FF">
            <animateTransform attributeName="transform" type="translate" from="0 0" to="0 270" dur="4.2s" repeatCount="indefinite"/>
          </rect>
        </g>
        <g opacity="0.04">
          <rect x="20" y="20" width="880" height="22" fill="#E86FD8">
            <animateTransform attributeName="transform" type="translate" from="0 0" to="0 282" dur="3.1s" repeatCount="indefinite"/>
          </rect>
        </g>''')

# ================= screen-level overlays: power surge (mirrored on/off) =================
ap('''        <rect x="20" y="20" width="880" height="304" fill="url(#surge)">
          <animate attributeName="opacity" calcMode="linear"
            values="0;0.45;0;0;0.55;0.55;0;0" keyTimes="0;0.02;0.05;0.93;0.945;0.968;0.982;1"
            dur="10s" repeatCount="indefinite"/>
        </rect>
        <rect x="20" y="20" width="880" height="304" fill="#000000">
          <animate attributeName="opacity" calcMode="linear"
            values="0;0;0.12;0;0;0;0.08;0;0" keyTimes="0;0.24;0.26;0.28;0.58;0.61;0.63;0.65;1"
            dur="10s" repeatCount="indefinite"/>
        </rect>
        </g>
      </g>
    </g>
  </g>''')

# hot scanline - drawn OUTSIDE the tube (a line inside the tube would be
# squashed to sub-pixel height exactly when it should be visible). Its scale
# animation mirrors the tube's scaleX keyframes so its width always tracks
# the raster while the height stays a crisp 6 px: full width while the
# vertical deflection collapses and while the squashed line holds, then it
# shrinks into the center dot together with the line. The tube collapses to
# scaleY 0.02 (a 6 px band - the same height as this line) so the two fuse
# into one bright line instead of an abrupt swap. Hidden states use scaleY
# 0.001 (0.02 still left a faint full-width streak during the hold/typing
# phase - sub-pixel geometry still paints). Shape is an ellipse, not a
# rect: lens-like curved edges instead of a hard straight stripe. Kept dim
# (static opacity 0.55) so it reads as a glow guiding the collapse, not a
# hard stripe. Visibility is driven by scaleY alone - scale animations are
# the one SMIL feature that proved rock-solid in Chromium, so no opacity
# keyframes here.
ap(f'''  <g transform="translate({CX},{CY})">
    <g>
      <animateTransform attributeName="transform" type="scale" calcMode="linear"
        values="0.001 0.001;0.02 1;1 1;1 1;1 1;1 0.001;1 0.001;1 1;1 1;0.02 1;0.001 0.001;0.001 0.001" keyTimes="0;0.004;0.011;0.012;0.026;0.04;0.93;0.945;0.968;0.982;0.993;1"
        dur="10s" repeatCount="indefinite"/>
      <g transform="translate(-{CX},-{CY})">
        <ellipse cx="{CX}" cy="172" rx="440" ry="3" fill="url(#hotline)" opacity="0.55"/>
      </g>
    </g>
  </g>''')

# ================= CRT beam core (power on/off, both ends) =================
# Real CRT power-off afterglow instead of a stylized star: the collapsing
# raster concentrates the electron beam into a white-hot dot at the screen
# center. While the raster is squashed to a line the beam still sweeps
# horizontally, so the hot scanline (see above) shows during both collapse
# phases; the dot stays hidden through the vertical collapse and the line
# hold (0.95-0.968 - blooming during the collapse read as a timeline bug)
# and only ignites when the line starts shrinking horizontally (0.968),
# blooming as the line shrinks into it and lingering with a slow phosphor
# afterglow (0.986-1). Entrance mirrors the exit: at power-on the beam spot
# flashes briefly (0-0.02, gone before the typing starts) and the line
# expands out of it. Visibility is scale-driven (0.02 = invisible),
# mirroring the tube's proven scale-animation structure - no opacity
# keyframes.
ap(f'''  <g transform="translate({CX},{CY})">
    <g>
      <animateTransform attributeName="transform" type="scale" calcMode="linear"
        values="0.02 0.02;0.6 0.6;0.6 0.6;0.02 0.02;0.02 0.02;0.02 0.02;1.6 1.6;0.02 0.02" keyTimes="0;0.002;0.01;0.02;0.95;0.968;0.986;1"
        dur="10s" repeatCount="indefinite"/>
      <circle r="30" fill="url(#core)"/>
    </g>
  </g>''')

# ================= cyberpunk HUD chrome =================
# Energy pulse traveling around the bezel frame (fake glow: one wide soft
# stroke under a thin bright one - no filter, Chromium silently drops
# filters whose region spans the whole canvas).
FRAME_PATH = ("M24,10 H896 Q910,10 910,24 V320 Q910,334 896,334 H24 Q10,334 10,320 V24 Q10,10 24,10 Z")
FRAME_LEN = 2424  # rounded-rect perimeter: 2*(900+324) - 8*14 + 2*pi*14

ap(f'''  <g>
    <animate attributeName="opacity" values="0.55;0.9;0.55" keyTimes="0;0.5;1" dur="5s" repeatCount="indefinite"/>
    <path d="{FRAME_PATH}" fill="none" stroke="url(#frameSweep)" stroke-width="6"
          stroke-linecap="round" stroke-dasharray="170 {FRAME_LEN - 170}" opacity="0.16">
      <animate attributeName="stroke-dashoffset" from="0" to="-{FRAME_LEN}" dur="9s" repeatCount="indefinite"/>
    </path>
    <path d="{FRAME_PATH}" fill="none" stroke="url(#frameSweep)" stroke-width="2.5"
          stroke-linecap="round" stroke-dasharray="170 {FRAME_LEN - 170}">
      <animate attributeName="stroke-dashoffset" from="0" to="-{FRAME_LEN}" dur="9s" repeatCount="indefinite"/>
    </path>
  </g>''')

# Thin HUD hairline inside the screen glass, one continuous gradient run.
ap('''  <rect x="26" y="26" width="868" height="292" rx="4" fill="none"
        stroke="url(#frameSweep)" stroke-width="1" opacity="0.10"/>''')

# Bezel microtext: dim mono readout on the bottom strip, slow pulse.
ap('''  <text x="460" y="334.5" text-anchor="middle" font-size="8" letter-spacing="1.5" fill="#7FD5FF">
    <animate attributeName="opacity" values="0.30;0.55;0.30" keyTimes="0;0.5;1" dur="4s" repeatCount="indefinite"/>
    DOLOGGER &#183; BY NEKOLIO &#183; FAST &#183; SECURE &#183; MODULAR
  </text>''')

# Corner accents removed: square L-brackets on a rounded bezel read as a
# mismatch (outer radius vs inner right angles). The frame sweep, hairline
# and microtext carry the HUD chrome instead.
ap('</svg>')

svg = "\n".join(parts)

with open(OUT, "w", encoding="utf-8") as f:
    f.write(svg)

# Keep the site copy in sync (same brand image, two consumers: README/docs
# and the landing page). build-site.sh re-copies the Docs copy into dist
# anyway; syncing public/ here keeps local `bun run dev/build` honest too.
with open(SITE_OUT, "w", encoding="utf-8") as f:
    f.write(svg)

# ================= validation =================
ET.parse(OUT)
ET.parse(SITE_OUT)
n_anim = svg.count("<animate")
n_clip = svg.count("<clipPath")
n_use = svg.count("<use ")
print(f"wrote {OUT}")
print(f"wrote {SITE_OUT}")
print(f"animates: {n_anim}, clipPaths: {n_clip}, uses: {n_use}, lines: {len(LINES)}")
print(f"noise removed: {'noise' not in svg}")
