#!/usr/bin/env python3
"""Write docs/brand/tray-geometry.md from mark.py, so the Rust tray drawing and the PNGs agree."""
import os, sys
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
import mark as M
from kit import OUTLINE

def pts(p):
    return ", ".join(f"({x:.2f}, {y:.2f})" for x, y in p)

def section(grid, P, use):
    ptr = M.pointer(P)
    return f"""### {grid} px grid — used for {use}

Coordinates in grid units (x right, y down, origin top-left); scale by `size / {grid}`.

- **Body polygon:** {pts(ptr['body'])}
  - corner radii, same order: {", ".join(f"{r:.2f}" for r in ptr['body_r'])}
- **Tip polygon:** {pts(ptr['tip'])}
  - corner radii, same order: {", ".join(f"{r:.2f}" for r in ptr['tip_r'])}
- **Outline width** (hollow states): {OUTLINE[grid]:.2f}
- The cut between them is {P['gap']:.2f} units wide, at 135°.

"""

doc = f"""# Tray icon geometry (oneMK Pointer)

Written by `src-2026/geomdoc.py` from `src-2026/mark.py`, the same source as the PNGs in `tray/`, so code and
artwork match. The app currently embeds the PNGs (`scripts/brand-assets.py` → `crates/tray/assets/*.rgba`);
this file is for drawing them procedurally instead.

## States

| state | body | tip | template (macOS) |
|---|---|---|---|
| **Off**: the daemon is not running | hollow outline, 55 % alpha | hollow outline, 55 % alpha | yes: black, alpha 0.55 |
| **Idle**: running, no other computer | solid ink | ink at **35 %** alpha (unlit) | yes: black |
| **Connected**: at least one computer online | solid ink | **accent** (lit) | no: colour PNG |
| **Controlling**: the pointer is on another computer (or another controls this one) | **hollow** outline | **accent** (lit) | no: colour PNG |

Colours:
- **dark trays** (macOS dark, Windows dark, GNOME): ink `{M.INK}`, accent `{M.ACCENT}`
- **light trays**: ink `{M.LIGHT_TRAY_INK}`, accent `{M.ACCENT_DEEP}` (Volt has no contrast on a light bar)
- **macOS template images** (Off and Idle): pure black with alpha.

In one colour the four states differ by shape: hollow / dim tip / solid / hollow body with a solid tip.

The tray tests in `crates/tray/src/icon.rs` look for amber pixels in Connected and Controlling. With the new art
the lit pixels are Volt `{M.ACCENT}` (RGB 198, 244, 50): a test such as
`p[3] > 128 && p[0] > 150 && p[1] > 210 && p[2] < 120` finds them (white ink fails it on `p[2]`).

## Which drawing at which pixel size

| physical px | drawing |
|---|---|
| 16–20 | 16 px grid |
| 21–23 | 22 px grid |
| 24–31 | 24 px grid |
| 32 and up (all @2x) | 256 master grid |

## Algorithm (per pixel, coverage 0..1)

```text
s     = size / grid; p = (px + 0.5, py + 0.5) / s; aa = 1 / s
body  = clamp(0.5 - sd_rpoly(p, body) / aa, 0, 1)      // sd_rpoly: signed distance to the filleted polygon
tip   = clamp(0.5 - sd_rpoly(p, tip) / aa, 0, 1)
ring  = shape * clamp((-sd_rpoly(p, shape) - w) / aa + 0.5, 0, 1)   // inner part to remove for "hollow"
hollow(shape) = shape - ring
```

A filleted polygon's signed distance is the plain polygon's distance offset by the radius near each corner;
treating corners as sharp below 24 px changes no pixel by more than a few percent.

{section(16, M.SMALL[16], '16–20 px')}{section(22, M.SMALL[22], '21–23 px')}{section(24, M.SMALL[24], '24–31 px')}{section(256, M.MASTER, '32 px and up')}"""
open(os.path.join(os.path.dirname(HERE), "tray-geometry.md"), "w").write(doc)
print("tray-geometry.md written")
