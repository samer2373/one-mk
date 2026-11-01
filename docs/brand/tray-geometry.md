# Tray icon geometry (oneMK Pointer)

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
- **dark trays** (macOS dark, Windows dark, GNOME): ink `#EDEFF2`, accent `#C6F432`
- **light trays**: ink `#1B1F24`, accent `#4E7A00` (Volt has no contrast on a light bar)
- **macOS template images** (Off and Idle): pure black with alpha.

In one colour the four states differ by shape: hollow / dim tip / solid / hollow body with a solid tip.

The tray tests in `crates/tray/src/icon.rs` look for amber pixels in Connected and Controlling. With the new art
the lit pixels are Volt `#C6F432` (RGB 198, 244, 50): a test such as
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

### 16 px grid — used for 16–20 px

Coordinates in grid units (x right, y down, origin top-left); scale by `size / 16`.

- **Body polygon:** (6.82, 2.97), (15.50, 5.29), (8.26, 7.23), (8.26, 14.72), (3.43, 6.35)
  - corner radii, same order: 0.12, 1.25, 0.80, 1.25, 0.12
- **Tip polygon:** (0.50, 1.28), (5.70, 2.67), (2.91, 5.46)
  - corner radii, same order: 0.55, 0.12, 0.12
- **Outline width** (hollow states): 1.25
- The cut between them is 1.00 units wide, at 135°.

### 22 px grid — used for 21–23 px

Coordinates in grid units (x right, y down, origin top-left); scale by `size / 22`.

- **Body polygon:** (9.33, 4.14), (21.20, 7.32), (11.36, 9.96), (11.36, 20.15), (4.76, 8.71)
  - corner radii, same order: 0.15, 1.70, 1.10, 1.70, 0.15
- **Tip polygon:** (0.80, 1.85), (7.94, 3.77), (4.11, 7.59)
  - corner radii, same order: 0.70, 0.15, 0.15
- **Outline width** (hollow states): 1.60
- The cut between them is 1.25 units wide, at 135°.

### 24 px grid — used for 24–31 px

Coordinates in grid units (x right, y down, origin top-left); scale by `size / 24`.

- **Body polygon:** (10.21, 4.54), (23.10, 8.00), (12.39, 10.87), (12.39, 21.95), (5.22, 9.53)
  - corner radii, same order: 0.18, 1.90, 1.20, 1.90, 0.18
- **Tip polygon:** (0.90, 2.05), (8.65, 4.12), (4.49, 8.27)
  - corner radii, same order: 0.80, 0.18, 0.18
- **Outline width** (hollow states): 1.70
- The cut between them is 1.40 units wide, at 135°.

### 256 px grid — used for 32 px and up

Coordinates in grid units (x right, y down, origin top-left); scale by `size / 256`.

- **Body polygon:** (109.36, 60.66), (224.00, 91.37), (131.38, 116.19), (131.39, 212.07), (67.90, 102.12)
  - corner radii, same order: 1.50, 18.00, 12.00, 18.00, 1.50
- **Tip polygon:** (32.00, 39.93), (102.11, 58.72), (64.54, 96.29)
  - corner radii, same order: 9.00, 1.50, 1.50
- **Outline width** (hollow states): 17.00
- The cut between them is 6.50 units wide, at 135°.

