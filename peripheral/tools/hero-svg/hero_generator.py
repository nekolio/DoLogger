#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Generate docs/assets/hero.svg - animated CRT boot hero, pure SMIL.

AUXILIARY TOOL - NOT part of the DoLogger runtime, build, or CI.
This script only regenerates a decorative README/landing-page image.
Deleting it (or the whole peripheral/tools/ directory) has ZERO effect on the
project. See ../README.md.

Effects (SMIL only, no scripts - runs in GitHub READMEs):
  - per-character typing where each glyph rides its own <g>: a block cursor is
    grouped WITH the currently-last-shown character, so the cursor-to-text
    distance is exact by construction (no frame-by-frame cursor distances)
  - cyberpunk CRT: frame energy sweep, cyan/magenta accents, HUD hairline,
    bezel microtext, phosphor-profile scanlines (soft falloff, half-phase
    aligned inside the screen), moire second layer, aperture grille, vignette
  - glitches THROUGHOUT the cycle (typing included, not just the hold):
    fast jitter, RGB channel split, ghosting, screen tearing, glitch lines
  - mirrored power-on/off with real CRT deflection physics: the raster
    collapses vertically into a white-hot scanline, the line shrinks
    horizontally into the screen-center point (with a white surge and
    phosphor afterglow); power-on mirrors it

Usage:
    python3 peripheral/tools/hero-svg/hero_generator.py

The whole timeline (line start/end times, wordmark/version ignition windows,
glitch bursts, and the CYCLE length) is COMPUTED from the LINES table and the
Cargo.toml version - there are no hardcoded per-line delays or cursor
distances. Output is deterministic (all randomness is seeded by fixed
timeline values), so regenerating produces a byte-identical file unless the
LINES data or the Cargo.toml version change. Writes docs/assets/hero.svg only
(the single source of truth; the site references it at build time).
"""

import math
import os
import random
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
OUT = os.path.join(ROOT, "docs", "assets", "hero.svg")

# ---------------- layout ----------------
X0 = 44                      # text left edge
CX, CY = 460.0, 172.0        # screen center - collapse anchor AND star anchor

# ---------------- timing constants (physical, not per-line delays) ----------------
POWER_ON = 0.35              # power-on deflection settles before typing starts
RATE = 0.020                 # typing speed: seconds per character (50 chars/s)
LINE_PAUSE = 0.6             # short pause after each typed line (1-2 cursor blinks)
WORDMARK_FLOW = 1.5          # neon-ignition duration for the wordmark
VERSION_FLOW = 1.2           # neon-ignition duration for the version tag
HOLD_MIN = 1.2               # minimum full-boot hold before power-off
CYCLE_MIN = 8.0              # floor for the derived cycle
POWER_OFF_FRAC = 0.93        # normalized power-off start (matches the deflection keyTimes)
GLITCH_DT = 0.033            # ~30 Hz glitch steps
BLINK_DUR = 0.4              # cursor blink period (square wave, 50% duty)

# glitch burst layout (spread programmatically below, seeded -> deterministic)
MICRO_N = 8                  # micro bursts across the typing window
MICRO_LEN = 0.08
MACRO_N = 3                  # macro bursts inside the hold window
MACRO_LEN = 0.18
DIP_N = 11                   # subtle brightness dips across typing + hold

MSG_COLOR = "#B8C2CE"
VERSION_SIZE = 20            # smaller than the logo (56), larger than logs (15)

# Boot canon. c1..c5 are typed, c6 is the wordmark (whole neon ignition),
# c7 is the tagline (typed after the version flow). `pre` marks a colored
# level prefix (left-padded below so every message starts at the same x).
LINES = [
    dict(cid="c1", color="#D9A066", y=64,  size=15,
         text="$ dologctl run --trace --config dologger.toml"),
    dict(cid="c2", pre="[INFO]",   color="#86C97F", y=92,  size=15,
         text="Hello DoLogger"),
    dict(cid="c3", pre="[PLUGIN]", color="#8A9BFF", y=120, size=15,
         text="4 official plugins · trust BLUE"),
    dict(cid="c4", pre="[AUDIT]",  color="#7FC7D9", y=148, size=15,
         text="ed25519 chain armed"),
    dict(cid="c5", pre="[PROCESS]", color="#86C97F", y=176, size=15,
         text="7-stage pipeline online"),
    dict(cid="c6", kind="wordmark", text="DoLogger", y=232, size=56),
    dict(cid="c7", color="#9FB0C9", y=266, size=14,
         text="next-gen secure logging · ed25519 @ lock-free speed"),
]


def read_version():
    """Parse the FIRST `version = "..."` line of the top-level Cargo.toml."""
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as f:
        for line in f:
            stripped = line.strip()
            if stripped.startswith("version") and "=" in stripped:
                return stripped.split("=", 1)[1].strip().strip("\"'")
    raise SystemExit("version not found in Cargo.toml")


VERSION = read_version()


def full_text(L):
    return (L.get("pre_full") or "") + L["text"]


def advance(size):
    # JetBrains Mono measured advance ratio (em): glyph advance = 0.60 * px.
    # The old 0.55 was Consolas' ratio; JetBrains Mono is a touch wider.
    return round(0.60 * size, 1)


# ---------------- alignment: pad prefixes so every message starts at the same x ----------------
MAX_PREFIX = max(len(L["pre"]) for L in LINES if "pre" in L)
COL = MAX_PREFIX + 1         # level column width incl. one separator space
for L in LINES:
    if "pre" in L:
        L["pre_full"] = L["pre"] + " " * (COL - len(L["pre"]))

# ---------------- derive the whole timeline from string lengths (no hardcoded delays) ----------------
WM = next(i for i, L in enumerate(LINES) if L.get("kind") == "wordmark")
TYPED = LINES[:WM]           # c1..c5, typed character-by-character
WORDMARK = LINES[WM]         # c6, whole neon ignition
TAGLINE = LINES[WM + 1]      # c7, typed after the version flow

t = POWER_ON
for L in TYPED:
    L["n"] = len(full_text(L))
    L["start"] = round(t, 4)
    L["end"] = round(t + L["n"] * RATE, 4)
    t = round(L["end"] + LINE_PAUSE, 4)

wm_start = t                              # after c5's end-of-line pause
wm_end = round(wm_start + WORDMARK_FLOW, 4)
ver_start = wm_end                        # version ignites right after the wordmark flow
ver_end = round(ver_start + VERSION_FLOW, 4)

TAGLINE["n"] = len(full_text(TAGLINE))
TAGLINE["start"] = ver_end                # tagline types only after the version flow
TAGLINE["end"] = round(ver_end + TAGLINE["n"] * RATE, 4)

hold_start = round(TAGLINE["end"] + LINE_PAUSE, 4)

# CYCLE: round up so the hold phase ends exactly at the normalized power-off
# start (0.93 of CYCLE), then enforce the 8 s floor. Every master-timeline
# animation normalizes its keyTimes by CYCLE, so power on/off, scanlines,
# frame sweep and HUD all scale proportionally with it.
raw = (hold_start + HOLD_MIN) / POWER_OFF_FRAC
CYCLE = max(CYCLE_MIN, math.ceil(raw))
power_off_start = POWER_OFF_FRAC * CYCLE

# ---------------- glitch bursts + dips, spread programmatically (deterministic) ----------------
def spread_bursts(t0, t1, n, length, seed):
    """Place n `length`-second bursts evenly-ish across [t0, t1]."""
    rng = random.Random(seed)
    slot = (t1 - t0) / n
    out = []
    for i in range(n):
        center = t0 + (i + 0.5) * slot
        jitter = rng.uniform(-0.25, 0.25) * slot
        start = center + jitter - length / 2
        start = max(t0, min(start, t1 - length))
        out.append((round(start, 3), round(start + length, 3)))
    return out


def spread_points(t0, t1, n, seed):
    rng = random.Random(seed)
    slot = (t1 - t0) / n
    out = []
    for i in range(n):
        center = t0 + (i + 0.5) * slot
        jitter = rng.uniform(-0.4, 0.4) * slot
        out.append(round(center + jitter, 3))
    return sorted(out)


MICRO = spread_bursts(POWER_ON, TAGLINE["end"], MICRO_N, MICRO_LEN, 101)
MACRO = spread_bursts(hold_start, power_off_start, MACRO_N, MACRO_LEN, 202)
dips = spread_points(POWER_ON, hold_start, DIP_N, 303)

# deterministic assignment of the generated bursts to each glitch effect
TEAR_WINDOWS = [MICRO[0], MACRO[0]]
STRAY_LINES = [
    ("#E8F4FF", [MICRO[1], MACRO[0]]),
    ("#F472D0", [MICRO[2], MACRO[1]]),
    ("#C792EA", [MICRO[3], MACRO[2]]),
]
RGB_RED = [MICRO[4], MACRO[0]]
RGB_CYAN = [MICRO[5], MACRO[1]]
GHOST = [MICRO[6], MACRO[2]]

# sheen sweep travels the wordmark+version lockup as the neon ramp completes
RAMP_FRAC = 0.30             # ramp-to-full begins 30% into the flow (see NEON_FLICKER)
sweep_start = round(wm_start + RAMP_FRAC * WORDMARK_FLOW, 4)
sweep_end = ver_end

# ---------------- helpers ----------------
def kt(t):
    return f"{t / CYCLE:.4f}"


def fmt(v):
    if isinstance(v, float):
        return f"{v:.2f}"
    return str(v)


def anim(attr, pairs, mode="discrete"):
    """pairs: list of (time_seconds, value) -> <animate> on the master timeline."""
    ks = ";".join(kt(t) for t, _ in pairs)
    vs = ";".join(fmt(v) for _, v in pairs)
    ts = [t for t, _ in pairs]
    assert len(ks.split(";")) == len(vs.split(";")), attr
    assert ts[0] == 0.0 and abs(ts[-1] - CYCLE) < 1e-6, (attr, ts[0], ts[-1])
    assert all(b > a for a, b in zip(ts, ts[1:])), attr
    return (f'<animate attributeName="{attr}" calcMode="{mode}" '
            f'values="{vs}" keyTimes="{ks}" dur="{CYCLE}s" repeatCount="indefinite"/>')


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


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# Neon-tube ignition envelope (fraction of flow, opacity): the tube strikes
# too dim (insufficient current), flickers irregularly, cuts out hard,
# re-ignites, then ramps slowly to full brightness as the sheen sweep crosses.
NEON_FLICKER = [
    (0.00, 0.00),
    (0.02, 0.35),
    (0.04, 0.15),
    (0.06, 0.35),
    (0.08, 0.10),
    (0.10, 0.35),
    (0.12, 0.00),   # hard cut-out
    (0.15, 0.25),   # re-ignite
    (0.19, 0.15),
    (0.23, 0.25),
    (0.30, 0.40),   # slow ramp to full as the sheen crosses
    (1.00, 1.00),
]


def neon_ignition(start, dur):
    """Absolute (time, opacity) pairs for a neon ignition starting at `start`."""
    pairs = [(round(start + frac * dur, 4), op) for frac, op in NEON_FLICKER]
    if pairs[0][0] > 0.0:
        pairs.insert(0, (0.0, 0.0))
    pairs.append((CYCLE, 1.0))
    return pairs


def emit_typed(L, hold_cursor=False):
    """Emit per-character groups (char + its own block cursor) for one typed line.

    With hold_cursor=True (the tagline) the FINAL character's cursor stays
    lit and keeps blinking through the idle hold phase until power-off, so
    "idle" reads as a blinking cursor, not a dead screen.
    """
    y, size, s, n = L["y"], L["size"], L["start"], L["n"]
    a = advance(size)
    pre_full = L.get("pre_full")
    if pre_full is not None:
        chars = [(esc(ch), L["color"]) for ch in pre_full] + [(esc(ch), MSG_COLOR) for ch in L["text"]]
    else:
        chars = [(esc(ch), L["color"]) for ch in L["text"]]
    cw = round(0.5 * a, 1)      # cursor width
    chh = round(1.25 * size, 1) # cursor height
    cxo = round(1.1 * a, 1)     # cursor x inside the group (left edge 0.1 em right of the char)
    cyo = y - size              # cursor top: one font-size above the baseline
    for k, (ch, fill) in enumerate(chars):
        t_k = round(s + k * RATE, 4)
        if k < n - 1:
            t_off = round(s + (k + 1) * RATE, 4)     # hand over to the next char
        elif hold_cursor:
            t_off = round(power_off_start, 4)        # idle blink through the hold phase
        else:
            t_off = round(s + n * RATE + LINE_PAUSE, 4)  # final char blinks through the pause
        gx = round(X0 + k * a, 1)
        ap(f'''            <g transform="translate({gx}, 0)">
              {anim("opacity", [(0.0, 0), (t_k, 1), (CYCLE, 1)], "discrete")}
              <text x="0" y="{y}" font-size="{size}" fill="{fill}">{ch}</text>
              <g>
                {anim("opacity", [(0.0, 0), (t_k, 1), (t_off, 0), (CYCLE, 0)], "discrete")}
                <g>
                  <animate attributeName="opacity" values="1;1;0;0" keyTimes="0;0.5;0.5;1" dur="{BLINK_DUR}s" repeatCount="indefinite"/>
                  <rect x="{cxo}" y="{cyo}" width="{cw}" height="{chh}" fill="#A8E6D8" opacity="0.92"/>
                </g>
              </g>
            </g>''')


parts = []
ap = parts.append

# ================= defs =================
ap('''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 920 344" font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace">
  <!--
    DoLogger boot hero - animated CRT terminal, pure SMIL (no scripts).
    Per-character typing: each glyph lives in its own <g> (translate(X0 +
    k*advance)) whose opacity is gated to turn on exactly at its typing time,
    and a block cursor is grouped INSIDE that same <g> at x = 1.1*advance - so
    the cursor's position relative to the last-shown character is exact by
    construction, with no frame-by-frame cursor distance table. The cursor's
    own inner <g> carries an independent infinite blink (square wave ~0.4 s)
    and an outer discrete opacity gate limits it to the window where its
    character is the last one revealed. Cyberpunk CRT: frame energy sweep,
    cyan/magenta accents, soft-profile scanlines half-phase aligned INSIDE the
    screen, moire second layer, aperture grille. Glitches fire throughout the
    whole cycle - typing included - via fast jitter, RGB channel split,
    ghosting and screen tearing. Power-on and power-off mirror each other
    around the screen center with real CRT deflection physics: vertical
    collapse first (raster squashes into a bright horizontal line), then
    horizontal collapse (line shrinks into a white-hot dot with a phosphor
    afterglow); power-on plays the same sequence backwards. Palette: phosphor
    cyan/amber/green with a single cyan -> violet -> magenta sweep for the
    wordmark - a coherent neon ramp, not a rainbow.
  -->''')
ap(f'''  <defs>
    <!-- cyberpunk wordmark ramp: one continuous cyan -> violet -> magenta sweep -->
    <linearGradient id="title" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0"    stop-color="#7FD5FF"/>
      <stop offset="0.45" stop-color="#8A9BFF"/>
      <stop offset="0.78" stop-color="#C792EA"/>
      <stop offset="1"    stop-color="#F472D0"/>
    </linearGradient>
    <!-- flowing light on the wordmark: a comet band sweeps left -> right
         across the wordmark+version lockup right after the neon-starter
         flicker settles, so the ignition wave travels the tube and the
         letters light up progressively. The band parks off-glyph the rest of
         the cycle; the seam jump back to -280 at t=0 is invisible because
         the tube sits at scale 0.02 there. The sweep window is computed from
         the wordmark+version flow ({sweep_start:.2f}s -> {sweep_end:.2f}s). -->
    <linearGradient id="sheen" gradientUnits="userSpaceOnUse" x1="-120" y1="0" x2="120" y2="0">
      <stop offset="0"    stop-color="#DFF7FF" stop-opacity="0"/>
      <stop offset="0.38" stop-color="#DFF7FF" stop-opacity="0"/>
      <stop offset="0.50" stop-color="#DFF7FF" stop-opacity="0.95"/>
      <stop offset="0.55" stop-color="#DFF7FF" stop-opacity="0.5"/>
      <stop offset="0.68" stop-color="#DFF7FF" stop-opacity="0"/>
      <stop offset="1"    stop-color="#DFF7FF" stop-opacity="0"/>
      <animateTransform attributeName="gradientTransform" type="translate" calcMode="linear"
        values="-280 0;-280 0;480 0;480 0" keyTimes="0;{kt(sweep_start)};{kt(sweep_end)};1"
        dur="{CYCLE}s" repeatCount="indefinite"/>
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
    <!-- subtle bloom for the version tag: a gentle halo merged UNDER a crisp
         copy of the source, so the small 20 px glyphs stay sharp and legible
         while a soft neon spread hugs their edges -->
    <filter id="bloomSoft" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur in="SourceGraphic" stdDeviation="1.1" result="halo"/>
      <feMerge>
        <feMergeNode in="halo"/>
        <feMergeNode in="SourceGraphic"/>
      </feMerge>
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
    </filter>
    <!-- screen-tearing band: a 40 px slice of the content, shifted sideways -->
    <clipPath id="tear"><rect x="20" y="0" width="880" height="40">
      {anim("y", band_y_pairs(TEAR_WINDOWS, 150))}
    </rect></clipPath>
  </defs>''')

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
        dur="{CYCLE}s" repeatCount="indefinite"/>
      <animate attributeName="opacity" calcMode="linear"
        values="1;1;0;0" keyTimes="0;0.968;0.982;1"
        dur="{CYCLE}s" repeatCount="indefinite"/>
      <g transform="translate(-{CX},-{CY})">
        <rect x="20" y="20" width="880" height="304" rx="8" fill="#0A0E15"/>
        <g>
          <animateTransform attributeName="transform" type="translate" calcMode="discrete"
            values="0 0;0 0;6 0;-5 0;4 0;-6 0;3 0;-4 0;2 0;-2 0;0 0;0 0" keyTimes="0;0.945;0.949;0.953;0.957;0.961;0.965;0.969;0.973;0.977;0.982;1"
            dur="{CYCLE}s" repeatCount="indefinite"/>''')

# ================= content (typed lines + cursors) =================
# brightness dips fire all cycle long, typing included
flick = [(0.0, 1)]
for d in dips:
    depth = 0.86 if any(t0 - 0.02 <= d <= t1 + 0.02 for t0, t1 in MACRO) else 0.90
    flick += [(d, 1), (d + 0.02, depth), (d + 0.04, 1)]
flick.append((CYCLE, 1))

jitter_bursts = [(t0, t1, 2) for t0, t1 in MICRO] + [(t0, t1, 3) for t0, t1 in MACRO]

ap(f'''        <g id="content">
          {anim("opacity", flick, "linear")}
          <g>
            {anim("transform", jitter_pairs(jitter_bursts), "discrete")}''')

# typed lines (per-character groups with their own grouped block cursors)
for L in TYPED:
    emit_typed(L)

# ---- wordmark + version: neon-tube ignition (no per-char, no cursor) ----
# The WHOLE sign ignites like a neon tube - it comes on too dim (insufficient
# current), flickers irregularly, cuts out hard, re-ignites, then ramps up
# slowly to full brightness as the sheen sweep travels the tube. Brightness
# only (opacity): glyphs never change size. The version tag re-uses the same
# flow and the title gradient once the wordmark settles, layered as: a faint
# wide bloom (1.6 blur at low opacity, a much weaker echo of the wordmark's
# glow), the crisp glyph with a gentle 1.1 px halo (bloomSoft), and the sheen
# sweep on top - so it keeps a subtle glow while staying legible at 20 px.
wm_adv = advance(WORDMARK["size"])
# version tag sits HALF a logo character width (half of one glyph's advance)
# right of the wordmark - tight, reads as a lockup rather than a caption
VERSION_GAP = round(0.5 * wm_adv, 1)
ver_x = round(X0 + len(WORDMARK["text"]) * wm_adv + VERSION_GAP, 1)
ap(f'''            <g>
              <g>
                {anim("opacity", neon_ignition(wm_start, WORDMARK_FLOW), "linear")}
                <text x="{X0}" y="{WORDMARK['y']}" font-size="{WORDMARK['size']}" font-weight="700"
                      font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace"
                      fill="url(#title)" filter="url(#glow)">DoLogger</text>
                <text x="{X0}" y="{WORDMARK['y']}" font-size="{WORDMARK['size']}" font-weight="700"
                      font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace"
                      fill="url(#sheen)" opacity="0.6">DoLogger</text>
              </g>
              <g>
                {anim("opacity", neon_ignition(ver_start, VERSION_FLOW), "linear")}
                <text x="{ver_x}" y="{WORDMARK['y']}" font-size="{VERSION_SIZE}" font-weight="700"
                      font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace"
                      fill="url(#title)" filter="url(#bloom)" opacity="0.25">v{VERSION}</text>
                <text x="{ver_x}" y="{WORDMARK['y']}" font-size="{VERSION_SIZE}" font-weight="700"
                      font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace"
                      fill="url(#title)" filter="url(#bloomSoft)">v{VERSION}</text>
                <text x="{ver_x}" y="{WORDMARK['y']}" font-size="{VERSION_SIZE}" font-weight="700"
                      font-family="'JetBrains Mono', Consolas, Menlo, 'DejaVu Sans Mono', monospace"
                      fill="url(#sheen)" opacity="0.6">v{VERSION}</text>
              </g>
            </g>''')

# tagline types only after the version flow completes; its cursor keeps
# blinking through the idle hold (idle = blinking, like the old design)
emit_typed(TAGLINE, hold_cursor=True)

ap('''          </g>
        </g>''')

# ================= phosphor bloom copy (permanent soft self-glow) =================
ap('''        <use href="#content" filter="url(#bloom)" opacity="0.30"/>''')

# ================= screen tearing band (shifted content slice) =================
ap(f'''        <g>
          {anim("opacity", flicker_windows(TEAR_WINDOWS, [0.7, 0.5]))}
          <g clip-path="url(#tear)">
            <use href="#content" transform="translate(14,0)" opacity="0.7"/>
          </g>
        </g>''')

# ================= stray glitch lines (one per color, micro + macro) =================
for i, (color, windows) in enumerate(STRAY_LINES):
    ap(f'''        <rect x="20" y="{120 + 40 * i}" width="880" height="2" fill="{color}">
          {anim("opacity", flicker_windows(windows, [0.45, 0.25], 0.0))}
          {anim("y", band_y_pairs(windows, 120 + 40 * i))}
        </rect>''')

# ================= RGB channel split + ghosting (glitch only) =================
ap(f'''        <g>
          {anim("opacity", flicker_windows(RGB_RED, [0.40, 0.25]))}
          <use href="#content" filter="url(#fRed)"/>
        </g>''')
ap(f'''        <g>
          {anim("opacity", flicker_windows(RGB_CYAN, [0.35, 0.22]))}
          <use href="#content" filter="url(#fCyan)"/>
        </g>''')
ap(f'''        <g>
          {anim("opacity", flicker_windows(GHOST, [0.28, 0.18]))}
          <use href="#content" filter="url(#bloom)" transform="translate(10,0)" opacity="0.28"/>
        </g>''')

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
ap(f'''        <rect x="20" y="20" width="880" height="304" fill="url(#surge)">
          <animate attributeName="opacity" calcMode="linear"
            values="0;0.45;0;0;0.55;0.55;0;0" keyTimes="0;0.02;0.05;0.93;0.945;0.968;0.982;1"
            dur="{CYCLE}s" repeatCount="indefinite"/>
        </rect>
        <rect x="20" y="20" width="880" height="304" fill="#000000">
          <animate attributeName="opacity" calcMode="linear"
            values="0;0;0.12;0;0;0;0.08;0;0" keyTimes="0;0.24;0.26;0.28;0.58;0.61;0.63;0.65;1"
            dur="{CYCLE}s" repeatCount="indefinite"/>
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
        dur="{CYCLE}s" repeatCount="indefinite"/>
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
        dur="{CYCLE}s" repeatCount="indefinite"/>
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

# ================= validation =================
root = ET.parse(OUT).getroot()
ns = "{http://www.w3.org/2000/svg}"

# structural self-check: every keyTimed <animate>/<animateTransform> has
# balanced values/keyTimes and a [0,1]-spanning, monotonic keyTimes list.
# (Strict monotonicity for master-timeline animates is already asserted in
# anim(); the only intentional equal-neighbour case is the cursor blink's
# square wave keyTimes="0;0.5;0.5;1".)
for tag in ("animate", "animateTransform"):
    for el in root.iter(ns + tag):
        kt_attr = el.get("keyTimes")
        if kt_attr is None:
            continue
        ks = [float(x) for x in kt_attr.split(";")]
        assert ks[0] == 0.0 and ks[-1] == 1.0, (tag, kt_attr)
        assert all(b >= a for a, b in zip(ks, ks[1:])), (tag, kt_attr)
        vals = el.get("values")
        if vals is not None:
            assert len(vals.split(";")) == len(ks), (tag, kt_attr, vals)

n_anim = svg.count("<animate")
n_clip = svg.count("<clipPath")
n_use = svg.count("<use ")
print(f"wrote {OUT}")
print(f"cycle: {CYCLE:.2f}s (power-off at {power_off_start:.2f}s)")
for L in TYPED + [TAGLINE]:
    print(f"  {L['cid']}: {L['start']:.2f}-{L['end']:.2f}s ({L['n']} chars)")
print(f"  wordmark flow: {wm_start:.2f}-{wm_end:.2f}s")
print(f"  version flow:  {ver_start:.2f}-{ver_end:.2f}s  (version v{VERSION} at x={ver_x})")
print(f"  hold:          {hold_start:.2f}-{power_off_start:.2f}s")
print(f"  sweep:         {sweep_start:.2f}-{sweep_end:.2f}s")
print(f"animates: {n_anim}, clipPaths: {n_clip}, uses: {n_use}, lines: {len(LINES)}")
print(f"noise removed: {'noise' not in svg}")
