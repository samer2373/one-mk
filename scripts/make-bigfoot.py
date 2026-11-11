#!/usr/bin/env python3
"""Builds the Bigfoot Technology pictures the window embeds, from its logo in docs/brand/bigfoot.

  python3 scripts/make-bigfoot.py            # (re)write everything
  python3 scripts/make-bigfoot.py --check    # fail if a checked-in file is stale, missing or the wrong size

Needs Python 3 and Pillow only. The mark (docs/brand/bigfoot/bigfoot-mark.svg: an orange tile
with its white Bigfoot face) uses absolute M/C/Z paths filled even-odd, a translate on each path and
one translate+scale group; this draws exactly that and raises on anything else.

The window draws the orange tile itself (a rounded rectangle, sharp at any scale, in the fixed
brand colour style::BIGFOOT_ORANGE) and lays the white art over it, so only that is a
picture: crates/app/assets/bigfoot/mark-<px>.rgba at 1x, 2x and 3x of the 44 point tile
(raw RGBA: width u32 LE, height u32 LE, then straight RGBA, so no decoder is needed at run time).
Also writes docs/brand/bigfoot/bigfoot-mark-512.png (the whole tile, for people to look at).

--check compares a fingerprint of the inputs (the SVG and this script) with the one the build
wrote, and each file's size: another Pillow draws edges a little differently, so pixels are not
compared (as scripts/make-icons.py does).
"""
import argparse
import hashlib
import io
import os
import re
import struct
import sys
import xml.etree.ElementTree as ET

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SVG = "docs/brand/bigfoot/bigfoot-mark.svg"
STAMP = "docs/brand/bigfoot/SOURCE-DIGEST"
TILE_PT = 44
SCALES = (1, 2, 3)
SUPERSAMPLE = 8
CURVE_STEPS = 32


def tag(el):
    return el.tag.split("}")[-1]


def transform(text):
    """(scale, dx, dy) of a translate(...) [scale(s)] attribute."""
    s, dx, dy = 1.0, 0.0, 0.0
    for name, args in re.findall(r"(\w+)\(([^)]*)\)", text or ""):
        v = [float(a) for a in re.split(r"[ ,]+", args.strip())]
        if name == "translate":
            dx, dy = dx + v[0] * s, dy + (v[1] if len(v) > 1 else 0.0) * s
        elif name == "scale" and (len(v) == 1 or v[0] == v[1]):
            s *= v[0]
        else:
            raise ValueError("unsupported transform " + name)
    return s, dx, dy


def sub_paths(d):
    toks = re.findall(r"[A-Za-z]|-?\d*\.?\d+(?:e-?\d+)?", d)
    i, cmd, subs, cur, pt, start = 0, None, [], [], (0.0, 0.0), (0.0, 0.0)

    def num():
        nonlocal i
        i += 1
        return float(toks[i - 1])

    while i < len(toks):
        if toks[i].isalpha():
            cmd = toks[i]
            i += 1
            if cmd not in "MCZ":
                raise ValueError("unsupported path command " + cmd)
            if cmd == "Z":
                if cur:
                    subs.append(cur)
                cur, pt = [], start
                continue
        if cmd == "M":
            if cur:
                subs.append(cur)
            pt = (num(), num())
            start, cur = pt, [pt]
            cmd = "?"   # an implicit line-to after M: not used by the files, so not drawn
        elif cmd == "?":
            raise ValueError("unsupported implicit line-to after M")
        elif cmd == "C":
            c1, c2, e = (num(), num()), (num(), num()), (num(), num())
            for k in range(1, CURVE_STEPS + 1):
                t = k / CURVE_STEPS
                a, b, c, dd = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t * t, t ** 3
                cur.append((a * pt[0] + b * c1[0] + c * c2[0] + dd * e[0], a * pt[1] + b * c1[1] + c * c2[1] + dd * e[1]))
            pt = e
    if cur:
        subs.append(cur)
    return subs


def fill_evenodd(mask, subs):
    """Scan-line even-odd fill at pixel centres."""
    draw = ImageDraw.Draw(mask)
    edges = [(min(y0, y1), max(y0, y1), x0, y0, x1, y1) for s in subs for (x0, y0), (x1, y1) in zip(s, s[1:] + s[:1]) if y0 != y1]
    if not edges:
        return
    top, bottom = min(e[0] for e in edges), max(e[1] for e in edges)
    for row in range(max(0, int(top)), min(mask.height, int(bottom) + 1)):
        yc = row + 0.5
        xs = sorted(x0 + (yc - y0) * (x1 - x0) / (y1 - y0) for lo, hi, x0, y0, x1, y1 in edges if lo <= yc < hi)
        for a, b in zip(xs[0::2], xs[1::2]):
            l, r = int(a + 0.5), int(b + 0.5) - 1
            if r >= l:
                draw.line((l, row, r, row), fill=255)


def hex_rgb(v):
    v = v.lstrip("#")
    return tuple(int(v[k:k + 2], 16) for k in (0, 2, 4))


def render(px, tile):
    """The mark `px` square: with its orange tile, or only the white art on transparent (the window draws the tile)."""
    root = ET.parse(os.path.join(ROOT, SVG)).getroot()
    vx, vy, vw, vh = (float(v) for v in root.get("viewBox").split())
    W = px * SUPERSAMPLE
    k = W / vw
    canvas = Image.new("RGBA", (W, W), (0, 0, 0, 0))

    def paint(mask, colour):
        canvas.alpha_composite(Image.merge("RGBA", (*Image.new("RGB", (W, W), hex_rgb(colour)).split(), mask)))

    def walk(el, s, dx, dy):
        for ch in el:
            t = tag(ch)
            if t == "g":
                gs, gx, gy = transform(ch.get("transform"))
                walk(ch, s * gs, dx + gx * s, dy + gy * s)
            elif t == "rect":
                if not tile:
                    continue
                m = Image.new("L", (W, W), 0)
                x, y, w, h = (float(ch.get(a, 0)) for a in ("x", "y", "width", "height"))
                r = float(ch.get("rx", 0))
                ImageDraw.Draw(m).rounded_rectangle(((x - vx) * k, (y - vy) * k, (x - vx + w) * k - 1, (y - vy + h) * k - 1), radius=r * k, fill=255)
                paint(m, ch.get("fill"))
            elif t == "path":
                if ch.get("fill-rule") != "evenodd":
                    raise ValueError("only even-odd paths are supported")
                ps, px_, py_ = transform(ch.get("transform"))
                subs = [[(((dx + (px_ + x * ps) * s) - vx) * k, ((dy + (py_ + y * ps) * s) - vy) * k) for x, y in sp] for sp in sub_paths(ch.get("d"))]
                m = Image.new("L", (W, W), 0)
                fill_evenodd(m, subs)
                paint(m, ch.get("fill"))
            else:
                raise ValueError("unsupported element <%s>" % t)

    walk(root, 1.0, 0.0, 0.0)
    return canvas.resize((px, px), Image.BOX)


def rgba(im):
    return struct.pack("<II", im.width, im.height) + im.tobytes()


def png(im):
    b = io.BytesIO()
    im.save(b, "PNG", optimize=True)
    return b.getvalue()


def outputs():
    out = {"crates/app/assets/bigfoot/mark-%d.rgba" % (TILE_PT * s): rgba(render(TILE_PT * s, False)) for s in SCALES}
    out["docs/brand/bigfoot/bigfoot-mark-512.png"] = png(render(512, True))
    return out


def digest():
    h = hashlib.sha256()
    for path in (SVG, "scripts/make-bigfoot.py"):
        with open(os.path.join(ROOT, path), "rb") as f:
            h.update(path.encode() + b"\0" + f.read() + b"\0")
    return h.hexdigest() + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    if args.check:
        bad = []
        with open(os.path.join(ROOT, STAMP)) as f:
            if f.read() != digest():
                bad.append(STAMP + ": made from another SVG or script (run scripts/make-bigfoot.py)")
        for s in SCALES:
            path = os.path.join(ROOT, "crates/app/assets/bigfoot/mark-%d.rgba" % (TILE_PT * s))
            if not os.path.exists(path):
                bad.append(path + ": missing")
                continue
            data = open(path, "rb").read()
            w, h = struct.unpack("<II", data[:8])
            if (w, h) != (TILE_PT * s, TILE_PT * s) or len(data) != 8 + w * h * 4:
                bad.append(path + ": not %d px square RGBA" % (TILE_PT * s))
        if bad:
            print("\n".join(bad))
            sys.exit(1)
        print("bigfoot pictures: up to date")
        return
    for path, data in outputs().items():
        full = os.path.join(ROOT, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "wb") as f:
            f.write(data)
        print(path)
    with open(os.path.join(ROOT, STAMP), "w") as f:
        f.write(digest())


if __name__ == "__main__":
    main()
