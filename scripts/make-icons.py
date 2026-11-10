#!/usr/bin/env python3
"""Builds every icon oneMK ships from the brand SVGs in docs/brand/logo.

  python3 scripts/make-icons.py            # (re)write everything
  python3 scripts/make-icons.py --check    # fail if a checked-in file is not what the SVGs give now (compared by pixels)

Needs Python 3 and Pillow only. The SVGs are drawn by scripts/brand_render.py (a small
deterministic rasteriser for exactly what those files use), and the output is
byte-for-byte reproducible, so a reviewer sees the art in the diff and CI can check it.

From docs/brand/logo/onemk-appicon.svg:
  docs/brand/app-icon/macos/onemk.icns, onemk.iconset/   the macOS grid: the tile is 824 of 1024 with a soft shadow
  docs/brand/app-icon/windows-linux/onemk.ico            16, 20, 24, 32, 40, 48, 64, 256 (PNG frames)
  docs/brand/app-icon/windows-linux/hicolor/...          16 to 512 PNGs and the scalable SVG (Linux, AppImage, Flatpak, .deb)
  crates/app/assets/icon-256.rgba                        the window icon (raw RGBA, see below)

From the flat symbol (drawn here, pixel-fitted at each size; see TRAY below):
  docs/brand/tray/{dark,light}/tray-<state>-<px>[@2x].png   white ink for dark bars, black ink for light bars
  docs/brand/tray/template/tray-<state>-16Template@2x.png   black + alpha for the macOS menu bar (off and idle)
  crates/tray/assets/<bar>-<state>-<px>.rgba                what one-mk-tray embeds

Also copies the SVGs the window embeds into crates/app/assets/brand/, and writes
docs/brand/overview.png. A raw .rgba file is: width (u32 LE), height (u32 LE), then
width*height*4 bytes of straight (not premultiplied) RGBA, so no image decoder is
needed at run time.
"""
import argparse
import io
import os
import struct
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import brand_render  # noqa: E402

ROOT = os.path.dirname(HERE)
LOGO = os.path.join(ROOT, "docs", "brand", "logo")
APPICON_SVG = os.path.join(LOGO, "onemk-appicon.svg")

# Brand colours (docs/brand/tokens.json).
COBALT_ON_LIGHT = (0x2F, 0x54, 0xEB)   # accent, light theme: a lit pointer on a light bar
COBALT_ON_DARK = (0x4D, 0x6D, 0xFF)    # accent, dark theme: a lit pointer on a dark bar

STATES = ["off", "idle", "connected", "controlling"]
# 1x sizes; each also at 2x. The bar sizes macOS, Windows and GNOME/KDE panels ask for.
TRAY_SIZES = [16, 22, 24]
# The SVGs the window embeds (the in-window logo): copied beside the app's other assets.
APP_BRAND_SVGS = [
    "onemk-symbol-flat-on-light.svg", "onemk-symbol-flat-on-dark.svg",
    "onemk-wordmark-on-light.svg", "onemk-wordmark-on-dark.svg",
]


# ------------------------------------------------------------------ writing ----

class Out:
    """Collects (path -> bytes) so the same code can write the tree or compare it."""

    def __init__(self):
        self.files = {}

    def png(self, rel, im):
        buf = io.BytesIO()
        im.save(buf, "PNG", optimize=False)   # no metadata, fixed compression: reproducible
        self.files[rel] = buf.getvalue()
        return self.files[rel]

    def rgba(self, rel, im):
        im = im.convert("RGBA")
        self.files[rel] = struct.pack("<II", *im.size) + im.tobytes()

    def raw(self, rel, data):
        self.files[rel] = data


def rel(*p):
    return "/".join(p)


# ---------------------------------------------------------------- app icon ----

def appicon(size, ss=4):
    return brand_render.render(APPICON_SVG, size, supersample=ss)


def macos_master():
    """The 1024 canvas of the macOS icon grid: the tile at 824 px, centred, with a soft drop shadow."""
    tile = appicon(824)
    canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    alpha = Image.new("L", (1024, 1024), 0)
    alpha.paste(tile.getchannel("A"), (100, 100 + 10))
    alpha = alpha.filter(ImageFilter.GaussianBlur(12)).point([round(i * 0.30) for i in range(256)])
    canvas.alpha_composite(Image.merge("RGBA", (*Image.new("RGB", canvas.size).split(), alpha)))
    canvas.alpha_composite(tile, (100, 100))
    return canvas


def make_app_icons(out):
    # macOS: iconset + icns (PNG entries; iconutil is not needed).
    master = macos_master()
    sets = {  # file name -> pixels
        "icon_16x16.png": 16, "icon_16x16@2x.png": 32, "icon_32x32.png": 32, "icon_32x32@2x.png": 64,
        "icon_128x128.png": 128, "icon_128x128@2x.png": 256, "icon_256x256.png": 256, "icon_256x256@2x.png": 512,
        "icon_512x512.png": 512, "icon_512x512@2x.png": 1024,
    }
    cache = {}
    for name, px in sets.items():
        cache.setdefault(px, master if px == 1024 else master.resize((px, px), Image.LANCZOS))
        out.png(rel("docs/brand/app-icon/macos/onemk.iconset", name), cache[px])
    # icns entry types for PNG data: icp4 16, icp5 32, ic11 16@2x, ic12 32@2x, ic07 128, ic13 128@2x,
    # ic08 256, ic14 256@2x, ic09 512, ic10 512@2x (1024).
    entries = [("icp4", 16), ("icp5", 32), ("ic11", 32), ("ic12", 64), ("ic07", 128), ("ic13", 256),
               ("ic08", 256), ("ic14", 512), ("ic09", 512), ("ic10", 1024)]
    body = b""
    for code, px in entries:
        data = out.files["docs/brand/app-icon/macos/onemk.iconset/" + {v: k for k, v in sets.items()}[px]]
        body += code.encode() + struct.pack(">I", len(data) + 8) + data
    out.raw("docs/brand/app-icon/macos/onemk.icns", b"icns" + struct.pack(">I", len(body) + 8) + body)
    out.png("docs/brand/app-icon/macos/png/onemk-1024.png", master)
    out.png("docs/brand/app-icon/macos/png/onemk-64.png", cache[64])

    # Windows and Linux: the full-bleed tile, drawn at each size.
    per_size = {px: appicon(px, ss=4 if px < 512 else 2) for px in [16, 20, 22, 24, 32, 40, 48, 64, 128, 256, 512]}
    for px in [16, 22, 24, 32, 48, 64, 128, 256, 512]:
        out.png(rel("docs/brand/app-icon/windows-linux/hicolor/%dx%d/apps/one-mk.png" % (px, px)), per_size[px])
    with open(APPICON_SVG, "rb") as f:
        out.raw("docs/brand/app-icon/windows-linux/hicolor/scalable/apps/one-mk.svg", f.read())
    ico_sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    pngs = []
    for px in ico_sizes:
        buf = io.BytesIO()
        per_size[px].save(buf, "PNG", optimize=False)
        pngs.append(buf.getvalue())
    head = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    directory = b""
    for px, data in zip(ico_sizes, pngs):
        directory += struct.pack("<BBBBHHII", px % 256, px % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    out.raw("docs/brand/app-icon/windows-linux/onemk.ico", head + directory + b"".join(pngs))

    # The window icon the app embeds (title bar, task switcher): the tile, 256 px.
    out.rgba("crates/app/assets/icon-256.rgba", per_size[256])


# -------------------------------------------------------------------- tray ----
# The flat keycap, redrawn on whole pixels at each size (a straight reduction of the 64-grid
# drawing would put the cut edges between pixels). All numbers are 1x pixels; the 2x drawing
# is the same layout doubled. The cap is x, y, w, h, r; the top plate likewise; the pointer is
# placed by its left edge, its top and its height (its width follows).
#
#   px   cap                  top plate           pointer (left, top, height)
TRAY_GEOMETRY = {
    16: dict(cap=(1, 1, 14, 14, 4.0), top=(3, 2, 10, 10, 3.0), pointer=(6, 4, 6)),
    22: dict(cap=(2, 2, 18, 18, 5.0), top=(4, 3, 14, 14, 4.0), pointer=(8, 6, 9)),
    24: dict(cap=(2, 2, 20, 20, 5.5), top=(4, 3, 16, 16, 4.5), pointer=(9, 7, 10)),
}
# The pointer of the 64 grid: M24 17 L24 39 L29.2 34 L32.6 42 L36.4 40.4 L33 32.6 L40 32.6 Z
POINTER = [(24, 17), (24, 39), (29.2, 34), (32.6, 42), (36.4, 40.4), (33, 32.6), (40, 32.6)]
POINTER_BOX = (24, 17, 25)   # left, top, height in the 64 grid
DIM = 0.45                   # Off: the whole drawing at this much of full ink
TRAY_SS = 16


def _rrect(draw, box, r, k, fill):
    x, y, w, h = box
    draw.rounded_rectangle((x * k, y * k, (x + w) * k - 1, (y + h) * k - 1), radius=r * k, fill=fill)


def tray_icon(state, px, scale, ink, cobalt):
    """One tray drawing: `px` is the 1x size (16, 22 or 24), `scale` 1 or 2; returns an RGBA image."""
    g = TRAY_GEOMETRY[px]
    size = px * scale
    k = TRAY_SS * scale     # drawing units to supersampled pixels

    def mask(draw_fn):
        m = Image.new("L", (px * TRAY_SS * scale,) * 2, 0)
        draw_fn(ImageDraw.Draw(m))
        return m

    cx, cy, cw, ch, cr = g["cap"]
    tx, ty, tw, th, tr = g["top"]
    cap = mask(lambda d: _rrect(d, (cx, cy, cw, ch), cr, TRAY_SS * scale, 255))
    top = mask(lambda d: _rrect(d, (tx, ty, tw, th), tr, TRAY_SS * scale, 255))
    ring = mask(lambda d: _rrect(d, (cx + 1, cy + 1, cw - 2, ch - 2), max(cr - 1, 0), TRAY_SS * scale, 255))
    ring = ImageChops.subtract(cap, ring)      # a 1 px outline, drawn inside the cap's edge
    left, ptop, height = g["pointer"]
    s = height / POINTER_BOX[2]
    pts = [((left + (x - POINTER_BOX[0]) * s) * k, (ptop + (y - POINTER_BOX[1]) * s) * k) for x, y in POINTER]
    pointer = mask(lambda d: d.polygon(pts, fill=255))

    hollow = state in ("off", "controlling")
    body = ring if hollow else ImageChops.subtract(cap, top)    # solid: the top plate is open, so the pointer sits in it
    lit = state in ("connected", "controlling")
    layers = [(body, ink, DIM if state == "off" else 1.0), (pointer, cobalt if lit else ink, DIM if state == "off" else 1.0)]
    im = Image.new("RGBA", (px * TRAY_SS * scale,) * 2, (0, 0, 0, 0))
    for m, colour, a in layers:
        if a < 1:
            m = m.point([round(i * a) for i in range(256)])
        im.alpha_composite(Image.merge("RGBA", (*Image.new("RGB", im.size, colour).split(), m)))
    return im.resize((size, size), Image.BOX)


def make_tray(out):
    bars = {"dark": ((255, 255, 255), COBALT_ON_DARK), "light": ((0, 0, 0), COBALT_ON_LIGHT)}
    for bar, (ink, cobalt) in bars.items():
        for state in STATES:
            for px in TRAY_SIZES:
                for scale in (1, 2):
                    im = tray_icon(state, px, scale, ink, cobalt)
                    name = "tray-%s-%d%s.png" % (state, px, "@2x" if scale == 2 else "")
                    out.png(rel("docs/brand/tray", bar, name), im)
                    out.rgba(rel("crates/tray/assets", "%s-%s-%d.rgba" % (bar, state, px * scale)), im)
    # macOS: black + alpha template images (the menu bar recolours them); only the plain states have one.
    for state in ("off", "idle"):
        im = tray_icon(state, 16, 2, (0, 0, 0), (0, 0, 0))
        out.png(rel("docs/brand/tray/template", "tray-%s-16Template@2x.png" % state), im)
        out.rgba(rel("crates/tray/assets", "%s-template-32.rgba" % state), im)


# ---------------------------------------------------------------- the rest ----

def copy_app_svgs(out):
    for name in APP_BRAND_SVGS:
        with open(os.path.join(LOGO, name), "rb") as f:
            out.raw(rel("crates/app/assets/brand", name), f.read())


def overview(out):
    """docs/brand/overview.png: the kit on one sheet, for the README and for reviewing the art."""
    W, H = 1600, 1000
    sheet = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    light = Image.new("RGBA", (W // 2, H), (0xF4, 0xF3, 0xF0, 255))
    dark = Image.new("RGBA", (W // 2, H), (0x11, 0x12, 0x14, 255))
    sheet.paste(light, (0, 0))
    sheet.paste(dark, (W // 2, 0))
    for x0, ground, lockup, flat in ((0, "light", "onemk-lockup-on-light.svg", "onemk-symbol-flat-on-light.svg"),
                                     (W // 2, "dark", "onemk-lockup-on-dark.svg", "onemk-symbol-flat-on-dark.svg")):
        sheet.alpha_composite(brand_render.render(os.path.join(LOGO, lockup), 560), (x0 + 80, 50))
        x = x0 + 80
        for px in (96, 64, 32, 24, 16):      # the flat symbol at its sizes
            sheet.alpha_composite(brand_render.render(os.path.join(LOGO, flat), px), (x, 330 - px))
            x += px + 36
        # the tray states at 16, 22 and 24 px, shown at 4x with nearest-neighbour so the pixels are visible
        ink, cobalt = ((0, 0, 0), COBALT_ON_LIGHT) if ground == "light" else ((255, 255, 255), COBALT_ON_DARK)
        y = 370
        for px in TRAY_SIZES:
            for col, state in enumerate(STATES):
                im = tray_icon(state, px, 1, ink, cobalt).resize((px * 4, px * 4), Image.NEAREST)
                sheet.alpha_composite(im, (x0 + 80 + col * 130, y))
            y += px * 4 + 16
    sheet.alpha_composite(macos_master().resize((280, 280), Image.LANCZOS), (80, 700))
    sheet.alpha_composite(appicon(280), (420, 700))
    sheet.alpha_composite(brand_render.render(os.path.join(LOGO, "onemk-symbol-sculpted.svg"), 280), (W // 2 + 80, 700))
    out.png("docs/brand/overview.png", sheet)


# ------------------------------------------------------------------- check ----
# --check compares pictures by their pixels, not their bytes: another Pillow or zlib version may
# compress the same picture differently, round an edge pixel by a level, or soften a shadow a
# little differently (CI's Ubuntu has an older Pillow than a developer's machine). So two pictures
# are the same when no pixel differs by more than TOLERANCE, or when they agree once averaged down
# to a coarse grid and their average difference is tiny. A real change (another colour, a moved
# or reshaped part) moves whole areas, which the coarse grid sees; a different resampler does not.
TOLERANCE = 3
COARSE = 16
COARSE_TOLERANCE = 10
MEAN_TOLERANCE = 3.0


LAST = {}  # how far apart the last pictures compared were, for the report of a failed check


def close(ia, ib):
    worst = max(hi for lo, hi in ImageChops.difference(ia, ib).getextrema())
    LAST.update(worst=worst, coarse=None, mean=None)
    if worst <= TOLERANCE:
        return True
    grid = (min(COARSE, ia.width), min(COARSE, ia.height))
    ca, cb = ia.resize(grid, Image.BOX), ib.resize(grid, Image.BOX)
    LAST["coarse"] = max(hi for lo, hi in ImageChops.difference(ca, cb).getextrema())
    if LAST["coarse"] > COARSE_TOLERANCE:
        return False
    # Averaged over every pixel and channel, how far apart they are.
    histogram = ImageChops.difference(ia, ib).convert("L").histogram()
    LAST["mean"] = sum(level * count for level, count in enumerate(histogram)) / max(1, ia.width * ia.height)
    return LAST["mean"] <= MEAN_TOLERANCE


def frames(path, data):
    """The pictures inside a file as (name, RGBA image); None for a file that is not a picture."""
    if path.endswith(".png"):
        return [("png", Image.open(io.BytesIO(data)).convert("RGBA"))]
    if path.endswith(".rgba"):
        w, h = struct.unpack("<II", data[:8])
        return [("rgba", Image.frombytes("RGBA", (w, h), data[8:]))] if len(data) == 8 + w * h * 4 else [("bad", None)]
    if path.endswith(".icns"):
        out, i = [], 8
        while i < len(data):
            n = struct.unpack(">I", data[i + 4:i + 8])[0]
            out.append((data[i:i + 4], Image.open(io.BytesIO(data[i + 8:i + n])).convert("RGBA")))
            i += n
        return out
    if path.endswith(".ico"):
        count = struct.unpack("<H", data[4:6])[0]
        out = []
        for j in range(count):
            size, off = struct.unpack("<II", data[6 + 16 * j + 8:6 + 16 * j + 16])
            out.append((j, Image.open(io.BytesIO(data[off:off + size])).convert("RGBA")))
        return out
    return None


def same(path, full, data):
    if not os.path.isfile(full):
        return False
    with open(full, "rb") as f:
        old = f.read()
    if old == data:
        return True
    a, b = frames(path, old), frames(path, data)
    if a is None or b is None or len(a) != len(b):
        return False
    for (na, ia), (nb, ib) in zip(a, b):
        if na != nb or ia is None or ia.size != ib.size:
            return False
        if not close(ia, ib):
            return False
    return True


def build():
    out = Out()
    make_app_icons(out)
    make_tray(out)
    copy_app_svgs(out)
    overview(out)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="fail if a checked-in file differs from what is built now")
    args = ap.parse_args()
    out = build()
    if args.check:
        bad = []
        for path, data in sorted(out.files.items()):
            LAST.clear()
            if not same(path, os.path.join(ROOT, path), data):
                bad.append(path)
                how = "missing" if not os.path.isfile(os.path.join(ROOT, path)) else "differs (worst pixel %s, coarse %s, mean %s)" % (LAST.get("worst"), LAST.get("coarse"), None if LAST.get("mean") is None else round(LAST["mean"], 2))
                print("stale or missing:", path, "-", how)
        print("%s" % ("all %d icon files match" % len(out.files) if not bad else "%d icon file(s) out of date: run python3 scripts/make-icons.py" % len(bad)))
        sys.exit(1 if bad else 0)
    for path, data in sorted(out.files.items()):
        full = os.path.join(ROOT, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "wb") as f:
            f.write(data)
    print("wrote %d files" % len(out.files))


if __name__ == "__main__":
    main()
