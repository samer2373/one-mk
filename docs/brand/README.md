# one·mk brand

![one·mk brand guideline](guideline/onemk-brand-guideline.png)

## The idea

A laptop and a monitor, with one pointer standing on the seam between them. That is the product: one mouse and
keyboard, crossing from one computer to the next. The pointer is the classic arrow and the only amber in the mark.
A clear gap separates it from the devices. The gap is a real hole in the artwork, so the mark works on any
background, in one colour and reversed.

Name: **one·mk**, with a middle dot. The CLI is `one-mk`.

## Logo

| file | use |
|---|---|
| `logo/onemk-symbol-reversed.svg` | **Primary.** Mark on dark grounds (ink `#EDEFF2` + amber) |
| `logo/onemk-symbol-color.svg` | Mark on light grounds (ink `#0F1216` + amber) |
| `logo/onemk-symbol-black.svg` / `-white.svg` | One colour |
| `logo/onemk-symbol-small16-*.svg`, `-small22-*`, `-small24-*` | Pixel-fitted drawings for 16–31 px |
| `logo/onemk-lockup-horizontal-*.svg` | Mark + wordmark, the default signature |
| `logo/onemk-lockup-stacked-*.svg` | For square spaces |
| `logo/onemk-wordmark-*.svg` | Wordmark only |
| `logo/png/` | PNGs of all of the above |

The wordmark is set from the real **Geist SemiBold** outlines (OFL) and tracked slightly tight. The interpunct is a
deliberate amber square, centred on the x-height. Every file is filled paths: no live text, no strokes, no masks.

### Small sizes

Below 32 px, use the pixel-fitted drawings, never the master scaled down:

- **16 px:** keeps **both computers and the pointer**. It uses 1 px strokes on whole pixels, a 1 px seam with
  the pointer's tip in it, and a smaller pointer, so the laptop's screen and both stands survive. It was tested
  at true size on dark and light bars and stays readable as "laptop, pointer, monitor".
- **22 px** (GNOME/AppIndicator): 2 px monitor frame, whole-pixel edges.
- **24 px:** 1.5 px strokes.
- **32 px and up:** the master.

Inside an app-icon *tile* at 16–20 px there is too little room for two computers. There, and only there, the
icon falls back to **one monitor + pointer**.

### Clear space and minimum size

- Clear space: **half the mark's height** on every side. In lockups, use the mark's height.
- Minimum: horizontal lockup **120 px** wide (25 mm in print); wordmark **64 px** (14 mm); symbol **16 px**
  (using the 16 px drawing).

## Colour

<!-- tokens:start (scripts/brand-tokens.py writes this table) -->
| token | value | contrast |
|---|---|---|
| ground | `#0F1216` | ink on it 16.3:1 |
| room | `#101318` | ink on it 16.2:1 |
| room-tint | `rgba(16, 19, 24, 0.878)` | 88% over what is under it |
| glass-top | `rgba(54, 56, 61, 0.443)` | 44% over what is under it |
| glass | `rgba(31, 33, 38, 0.42)` | 42% over what is under it |
| glass-clear-top | `rgba(79, 83, 86, 0.278)` | 28% over what is under it |
| glass-clear | `rgba(45, 45, 49, 0.243)` | 24% over what is under it |
| glass-solid-top | `#1C2027` | 1.1:1 on ground |
| glass-solid | `#161A20` | ink on it 15.2:1 |
| glass-rim | `rgba(255, 255, 255, 0.102)` | 10% over what is under it |
| glass-edge | `rgba(255, 255, 255, 0.22)` | 22% over what is under it |
| popover | `#1A1E25` | ink on it 14.5:1 |
| on-glass | `rgba(255, 255, 255, 0.071)` | 7% over what is under it |
| on-glass-lit | `rgba(255, 255, 255, 0.118)` | 12% over what is under it |
| panel | `#13171C` | ink on it 15.6:1 |
| raised | `rgba(255, 255, 255, 0.051)` | 5% over what is under it |
| line | `rgba(255, 255, 255, 0.071)` | 7% over what is under it |
| ink | `#EDEFF2` | 16.3:1 on ground |
| ink2 | `#9DA6B1` | 7.6:1 on ground |
| ink3 | `#8F98A4` | 6.4:1 on ground |
| ink-hover | `#C4CAD2` | 11.4:1 on ground |
| **signal** (amber) | `#FFB547` | 10.7:1 on ground; **1.8:1 on white** |
| signal-lit | `#FFC266` | 11.8:1 on ground |
| signal-ink | `#1E1505` | 10.3:1 on signal |
| error | `#FF6B5E` | 6.7:1 on ground |
| ok | `#5BC98F` | 9.1:1 on ground |
| button | `rgba(255, 255, 255, 0.086)` | 9% over what is under it |
| button-lit | `rgba(255, 255, 255, 0.133)` | 13% over what is under it |
| pill | `rgba(255, 255, 255, 0.125)` | 13% over what is under it |
| track-off | `#4A525E` | 2.4:1 on ground |
| track-edge | `#707A86` | 4.3:1 on ground |
| key-edge | `#697380` | 3.9:1 on ground |
| off | `#707A86` | 4.3:1 on ground |
<!-- tokens:end -->

Amber is the only signal colour. Use it for the pointer, live state and the one primary action. Do not use it for
decoration, large fills or text on light backgrounds. On amber, use the black one-colour mark.

Type: **Geist** 400/600 at 15 px (panel and sheet titles), 13 px (body, rows, buttons) and 12 px (chips, keys,
values), line height 1.45; **Geist Mono** for numbers, timings and keys. Corners: 20 px panels, 12 px inside them
and on toasts; 12 px between panels.

Tokens: [`tokens.json`](tokens.json) and [`tokens.css`](tokens.css), written from the window's own style
(`crates/app/src/style.rs`) by `python3 scripts/brand-tokens.py`; never edit them by hand.

## Approved backgrounds

- Ground and raised: the reversed mark.
- White and paper: the colour mark.
- Amber: black only.
- Photos and gradients: white only.
- Any background: black or white one-colour.

## Misuse

Don't:
- stretch, rotate or tilt the mark;
- recolour the pointer, or make the devices amber;
- remove the clearance around the pointer;
- add glows or shadows;
- place the mark on busy or mid-grey grounds;
- outline the pointer.

All of these are shown in the guideline sheet above.

## App icons

| platform | files |
|---|---|
| macOS | `app-icon/macos/onemk.icns`; `onemk.iconset/` (16–512 pt, @1x and @2x); master `onemk-appicon-master.svg` (1024, Big Sur squircle grid) |
| Windows | `app-icon/windows-linux/onemk.ico` (16, 20, 24, 32, 40, 48, 64, 256) |
| Linux | `app-icon/windows-linux/hicolor/<size>/apps/one-mk.png` (16–512); `hicolor/scalable/apps/one-mk.svg` |
| Web | `web/favicon.ico`, `favicon.svg`, `favicon-16/32/48.png`, `apple-touch-icon.png`, `icon-192/512.png`, `maskable-512.png`, `site.webmanifest`, `head-snippet.html` |

The tile is dark (`#1B2027` → `#0D1014`, a subtle top light) with a faint rim highlight. The `-tiny`, `-mid`
and `-master` SVGs are the drawings used for ≤20 px, ≤48 px and larger sizes.

## Tray icons

There are four states, matching `IconState` in `crates/tray/src/icon.rs`:
- **Off:** dimmed, no pointer.
- **Idle:** ink pointer.
- **Connected:** amber pointer.
- **Controlling:** amber pointer, and the far screen lit amber.

Sizes are 16, 22, 24 and 32 px at @1x and @2x:
- `tray/dark/`: dark trays
- `tray/light/`: light trays; the pointer gets a dark edge
- `tray/template/`: macOS template images for Off and Idle (black + alpha)

The geometry for drawing these procedurally in Rust is in [`tray-geometry.md`](tray-geometry.md).

## Presentation

![presentation board](guideline/onemk-presentation-board.png)

- `guideline/onemk-presentation-board.png`: app window, menu bars, app icons, README, terminal, browser tab,
  website.
- `guideline/board/slide-0*.png`: the same as slides.

## Rebuilding

Everything is generated from `src/`:
- `mark.py`: the geometry of the master and the pixel-fitted drawings
- `geom.py`: real knockouts by polygon difference
- `wordmark.py`: reads Geist with a tiny TrueType reader
- `kit.py`: writes every file
- `guide.py`: renders the guideline sheet

```sh
python3 docs/brand/src/kit.py logo app tray png web
python3 docs/brand/src/guide.py
```

It needs Python 3, Pillow (only to read PNG sizes) and the logo-design skill's `render_png.py`, which uses
headless Chrome. The `.icns` and `.ico` files are packed by `kit.py` itself, so `iconutil` is not needed.

Earlier concept rounds are in `concepts/`.
