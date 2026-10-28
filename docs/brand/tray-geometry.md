# Tray icon geometry

How to draw the one·mk tray icon procedurally in `crates/tray/src/icon.rs`, with no image files.
Everything below is generated from `src/mark.py`, the same source as the shipped PNGs in `tray/`, so
code and artwork match exactly.

## States

| state | devices | pointer | lit screen | template (macOS) |
|---|---|---|---|---|
| **Off**: the daemon is not running | ink at **45 %** alpha | none, and no clearance cut | no | yes: black, alpha 0.45 |
| **Idle**: running, no computer online | ink | **ink** (same colour as the devices), with clearance | no | yes: black |
| **Connected**: at least one computer online | ink | **amber** `#FFB547`, with clearance | no | no: colour PNG |
| **Controlling**: this computer controls another, or is controlled | ink | amber, with clearance | **amber** monitor screen | no: colour PNG |

Ink colours:
- **dark trays** (macOS dark, Windows dark, GNOME): `#EDEFF2`
- **light trays**: `#1B1F24`. On light trays the pointer also gets a `#1B1F24` ring as wide as the clearance, drawn
  under the amber, so the amber pointer keeps an edge on white. This is the classic black-outlined pointer.
- **macOS template images** (Off and Idle): pure black with alpha. macOS tints them for light and dark menu bars.

## Which drawing at which pixel size

Each pixel size has its own drawing, fitted to whole pixels (pick by the *physical* pixel size):

| physical px | drawing |
|---|---|
| 16–20 | 16 px grid |
| 21–23 | 22 px grid |
| 24–31 | 24 px grid |
| 32 and up (all @2x) | 256 master grid |

## Algorithm (per pixel, coverage 0..1)

The current `icon.rs` helpers already compute anti-aliased coverage for rounded rects (`rrect`). The new icon
needs one more helper: signed distance to a polygon.

```text
s      = size / grid            // grid = 16, 22, 24 or 256
p      = (px + 0.5, py + 0.5) / s     // pixel centre in grid units
aa     = 1 / s                  // one pixel, in grid units

dev    = max(ring(monitor), rect(neck), rrect(foot), ring(laptop), rrect(base))
         // ring(o, i) = min(rrect(o), 1 - rrect(i))
ptr    = coverage of the pointer polygon        = clamp(0.5 - sd_poly(p) / aa, 0, 1)
clear  = clamp((sd_poly(p) - gap) / aa + 0.5, 0, 1)   // 0 inside the clearance band, 1 outside
dev    = dev * clear            // Off: skip (no pointer, no clearance)
lit    = rrect(lit screen) * clear                       // Controlling only

paint order: devices (ink) → lit screen (amber) → [light trays: ring = clamp(0.5 - (sd_poly(p) - gap)/aa, 0, 1) in #1B1F24] → pointer (amber, or ink for Idle)
```

`sd_poly` is the usual signed distance to a closed polygon (negative inside): the minimum distance to each
edge, with the sign taken from a crossing-number test. It is about 20 lines of Rust and works for the
pointer's concave outline as is.

### 16 px grid — used for 16–20 px

Coordinates are in grid units (x right, y down, origin top-left); scale by `size / 16`.

| shape | x0 | y0 | x1 | y1 | corner radius | |
|---|---|---|---|---|---|---|
| monitor (outer) | 6 | 3 | 16 | 9 | 1.5 | ring: subtract inner below |
| monitor (inner, hole) | 7 | 4 | 15 | 8 | 0.5 |  |
| monitor neck | 11 | 9 | 12 | 11 | 0 |  |
| monitor foot | 9 | 11 | 14 | 12 | 0.5 |  |
| laptop screen (outer) | 0 | 6 | 5 | 10 | 1.4 |  |
| laptop screen (inner, hole) | 1 | 7 | 4 | 9 | 0.4 |  |
| laptop base | 0 | 11 | 5.5 | 12 | 0.5 |  |

- **Pointer polygon** (fill, in order): (5.95, 3.4), (5.95, 9.54), (7.5, 7.99), (8.53, 10.47), (9.41, 10.11), (8.45, 7.8), (10.35, 7.8). Round its convex corners by ≈0.22 units (optional below 24 px).
- **Clearance:** remove device coverage within **0.85 units** of the pointer polygon (distance from the polygon edge).
- **Lit screen** (Controlling only): rounded rect (7, 4) – (15, 8), radius 0.6, minus the same clearance.

### 22 px grid — used for 21–23 px

Coordinates are in grid units (x right, y down, origin top-left); scale by `size / 22`.

| shape | x0 | y0 | x1 | y1 | corner radius | |
|---|---|---|---|---|---|---|
| monitor (outer) | 10 | 4 | 22 | 13 | 2.6 | ring: subtract inner below |
| monitor (inner, hole) | 12 | 6 | 20 | 11 | 0.6 |  |
| monitor neck | 15 | 13 | 17 | 15 | 0 |  |
| monitor foot | 13.5 | 15 | 18.5 | 17 | 1 |  |
| laptop screen (outer) | 1 | 8 | 8 | 14 | 2.5 |  |
| laptop screen (inner, hole) | 3 | 10 | 6 | 12 | 0.5 |  |
| laptop base | 0 | 15 | 9 | 17 | 1 |  |

- **Pointer polygon** (fill, in order): (9.05, 5.4), (9.05, 13.59), (11.12, 11.51), (12.5, 14.83), (13.66, 14.34), (12.38, 11.26), (14.91, 11.26). Round its convex corners by ≈0.26 units (optional below 24 px).
- **Clearance:** remove device coverage within **1 units** of the pointer polygon (distance from the polygon edge).
- **Lit screen** (Controlling only): rounded rect (12, 6) – (20, 11), radius 0.6, minus the same clearance.

### 24 px grid — used for 24–31 px

Coordinates are in grid units (x right, y down, origin top-left); scale by `size / 24`.

| shape | x0 | y0 | x1 | y1 | corner radius | |
|---|---|---|---|---|---|---|
| monitor (outer) | 10 | 5 | 24 | 14 | 2.25 | ring: subtract inner below |
| monitor (inner, hole) | 11.5 | 6.5 | 22.5 | 12.5 | 0.75 |  |
| monitor neck | 16.25 | 14 | 17.75 | 16.5 | 0 |  |
| monitor foot | 14.5 | 16.5 | 19.5 | 18 | 0.75 |  |
| laptop screen (outer) | 1 | 9.5 | 8.5 | 15 | 2 |  |
| laptop screen (inner, hole) | 2.5 | 11 | 7 | 13.5 | 0.5 |  |
| laptop base | 0 | 16.5 | 9.5 | 18 | 0.75 |  |

- **Pointer polygon** (fill, in order): (9.4, 6.2), (9.4, 15.07), (11.64, 12.82), (13.13, 16.41), (14.4, 15.89), (13.01, 12.55), (15.75, 12.55). Round its convex corners by ≈0.26 units (optional below 24 px).
- **Clearance:** remove device coverage within **1 units** of the pointer polygon (distance from the polygon edge).
- **Lit screen** (Controlling only): rounded rect (11.5, 6.5) – (22.5, 12.5), radius 0.75, minus the same clearance.

### 256 grid (master) — used for 32 px and up, every @2x size

Coordinates are in grid units (x right, y down, origin top-left); scale by `size / 256`.

| shape | x0 | y0 | x1 | y1 | corner radius | |
|---|---|---|---|---|---|---|
| monitor (outer) | 122 | 71 | 238 | 147 | 16 | ring: subtract inner below |
| monitor (inner, hole) | 134 | 83 | 226 | 135 | 4 |  |
| monitor neck | 173 | 147 | 187 | 169 | 0 |  |
| monitor foot | 160 | 169 | 200 | 181 | 6 |  |
| laptop screen (outer) | 30 | 111 | 106 | 163 | 15 |  |
| laptop screen (inner, hole) | 42 | 123 | 94 | 151 | 3 |  |
| laptop base | 18 | 169 | 118 | 181 | 6 |  |

- **Pointer polygon** (fill, in order): (114, 91), (114, 159.21), (131.26, 141.95), (142.72, 169.57), (152.44, 165.53), (141.79, 139.84), (162.84, 139.84). Round its convex corners by ≈3.52 units (optional below 24 px).
- **Clearance:** remove device coverage within **6 units** of the pointer polygon (distance from the polygon edge).
- **Lit screen** (Controlling only): rounded rect (141.2, 90.2) – (218.8, 127.8), radius 0.6, minus the same clearance.


## Shipped files

- `tray/dark/tray-<state>-<size>[@2x].png`: colour, for dark trays
- `tray/light/tray-<state>-<size>[@2x].png`: colour, for light trays
- `tray/template/tray-<off|idle>-<size>Template[@2x].png`: macOS template images
- `tray/svg/<theme>/tray-<state>-<s16|s22|s24|m>.svg`: the vector sources
