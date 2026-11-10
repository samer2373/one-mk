"""A tiny, deterministic SVG rasteriser for the oneMK brand files (docs/brand/logo).

It reads exactly what those files use and nothing else, so the icons can be built
on any machine with Python 3 and Pillow (no rsvg, Inkscape, ImageMagick, Chrome or
numpy):

  <rect> with x y width height rx, fill, fill-opacity, stroke, stroke-opacity, stroke-width
  <path> with M L H V Q Z (absolute), fill, fill-opacity (non-zero winding, as in a browser: counters are
         the sub-paths that wind the other way, which is how the outlined wordmark is built)
  <g transform="translate(x y) scale(s)">
  <linearGradient> (top to bottom or left to right) and <radialGradient>, both in
  objectBoundingBox units, with <stop> colours

Anything else raises, so a new feature in a brand file cannot be silently dropped.
Shapes are drawn at SUPERSAMPLE times the size and reduced with a box filter.
"""
import math
import re
import xml.etree.ElementTree as ET

from PIL import Image, ImageChops, ImageDraw

NS = "{http://www.w3.org/2000/svg}"
CURVE_STEPS = 24


def _tag(el):
    return el.tag.replace(NS, "")


def _hex(value):
    h = value.lstrip("#")
    if len(h) == 3:
        h = "".join(c * 2 for c in h)
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


class Gradient:
    def __init__(self, el):
        self.radial = _tag(el) == "radialGradient"
        g = lambda k, d: float(el.get(k, d))
        if self.radial:
            self.cx, self.cy, self.r = g("cx", 0.5), g("cy", 0.5), g("r", 0.5)
        else:
            self.x1, self.y1, self.x2, self.y2 = g("x1", 0), g("y1", 0), g("x2", 1), g("y2", 0)
        self.stops = [(float(s.get("offset")), _hex(s.get("stop-color"))) for s in el if _tag(s) == "stop"]

    def _lut(self, channel):
        out = []
        for i in range(256):
            t = i / 255
            lo = max((s for s in self.stops if s[0] <= t), key=lambda s: s[0], default=self.stops[0])
            hi = min((s for s in self.stops if s[0] >= t), key=lambda s: s[0], default=self.stops[-1])
            if hi[0] == lo[0]:
                out.append(lo[1][channel])
            else:
                f = (t - lo[0]) / (hi[0] - lo[0])
                out.append(round(lo[1][channel] + (hi[1][channel] - lo[1][channel]) * f))
        return out

    def image(self, size):
        """The gradient over a w x h box, as RGB."""
        w, h = size
        if self.radial:
            ramp = Image.new("L", size, 255)
            rw, rh = max(1, round(2 * self.r * w)), max(1, round(2 * self.r * h))
            disc = Image.radial_gradient("L").resize((rw, rh), Image.BICUBIC)
            ramp.paste(disc, (round(self.cx * w - rw / 2), round(self.cy * h - rh / 2)))
        elif self.x1 == self.x2 and self.y1 != self.y2:      # vertical
            ramp = Image.linear_gradient("L").resize(size, Image.BILINEAR)
            if self.y1 > self.y2:
                ramp = ImageChops.invert(ramp)
            a, b = sorted((self.y1, self.y2))
            if (a, b) != (0, 1):                             # a stop range inside the box
                ramp = ramp.point([min(255, max(0, round((i / 255 - a) / (b - a) * 255))) for i in range(256)])
        elif self.y1 == self.y2 and self.x1 != self.x2:      # horizontal
            ramp = Image.linear_gradient("L").rotate(90, expand=True).transpose(Image.FLIP_LEFT_RIGHT).resize(size, Image.BILINEAR)
            if self.x1 > self.x2:
                ramp = ImageChops.invert(ramp)
        else:
            raise ValueError("only vertical and horizontal linear gradients are supported")
        return Image.merge("RGB", [ramp.point(self._lut(c)) for c in range(3)])


def _path_polygons(d, tf):
    """Sub-paths of an absolute M/L/H/V/Q/Z path as lists of points, transformed by tf."""
    toks = re.findall(r"[MLHVQZ]|-?\d*\.?\d+(?:e-?\d+)?", d)
    i, subs, cur, pt, start = 0, [], [], (0.0, 0.0), (0.0, 0.0)
    cmd = None

    def num():
        nonlocal i
        v = float(toks[i])
        i += 1
        return v

    while i < len(toks):
        if toks[i] in "MLHVQZ":
            cmd = toks[i]
            i += 1
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
            cmd = "L"      # further pairs after M are line-tos
        elif cmd == "L":
            pt = (num(), num())
            cur.append(pt)
        elif cmd == "H":
            pt = (num(), pt[1])
            cur.append(pt)
        elif cmd == "V":
            pt = (pt[0], num())
            cur.append(pt)
        elif cmd == "Q":
            c, e = (num(), num()), (num(), num())
            for s in range(1, CURVE_STEPS + 1):
                t = s / CURVE_STEPS
                cur.append(((1 - t) ** 2 * pt[0] + 2 * (1 - t) * t * c[0] + t * t * e[0],
                            (1 - t) ** 2 * pt[1] + 2 * (1 - t) * t * c[1] + t * t * e[1]))
            pt = e
        else:
            raise ValueError("unsupported path command near %r" % toks[i:i + 3])
    if cur:
        subs.append(cur)
    sx, sy, ox, oy = tf
    return [[(ox + x * sx, oy + y * sy) for x, y in s] for s in subs]


def _transform(el, tf):
    """Composes translate()/scale() (uniform) of a <g> onto tf = (sx, sy, ox, oy)."""
    sx, sy, ox, oy = tf
    for name, args in re.findall(r"(\w+)\(([^)]*)\)", el.get("transform", "")):
        v = [float(a) for a in re.split(r"[ ,]+", args.strip())]
        if name == "translate":
            ox, oy = ox + v[0] * sx, oy + (v[1] if len(v) > 1 else 0) * sy
        elif name == "scale":
            sx, sy = sx * v[0], sy * (v[1] if len(v) > 1 else v[0])
        else:
            raise ValueError("unsupported transform " + name)
    return sx, sy, ox, oy


def _fill_nonzero(draw, subs, height):
    """Fills the sub-paths with the non-zero winding rule (what a browser does), one scan line at a time,
    sampling each pixel at its centre. Overlapping contours unite and counters (wound the other way) stay open."""
    edges = []
    for s in subs:
        for (x0, y0), (x1, y1) in zip(s, s[1:] + s[:1]):
            if y0 != y1:
                edges.append((min(y0, y1), max(y0, y1), x0, y0, x1, y1, 1 if y1 > y0 else -1))
    if not edges:
        return
    top, bottom = min(e[0] for e in edges), max(e[1] for e in edges)
    for row in range(max(0, int(top)), min(height, int(bottom) + 1)):
        yc = row + 0.5
        hits = sorted((x0 + (yc - y0) * (x1 - x0) / (y1 - y0), d) for lo, hi, x0, y0, x1, y1, d in edges if lo <= yc < hi)
        wind, start = 0, 0.0
        for x, d in hits:
            if wind == 0:
                start = x
            wind += d
            if wind == 0:
                a, b = math.ceil(start - 0.5), math.ceil(x - 0.5) - 1
                if b >= a:
                    draw.line((a, row, b, row), fill=255)


def parse(path):
    root = ET.parse(path).getroot()
    vb = [float(v) for v in root.get("viewBox").split()]
    grads = {g.get("id"): Gradient(g) for g in root.iter() if _tag(g) in ("linearGradient", "radialGradient")}
    return root, vb, grads


def render(path, width, supersample=4, height=None):
    """Draws the SVG at `width` px wide (height follows the viewBox unless given) as RGBA."""
    root, (vx, vy, vw, vh), grads = parse(path)
    height = height or round(width * vh / vw)
    ss = supersample
    W, H = width * ss, height * ss
    base = (W / vw, H / vh, -vx * W / vw, -vy * H / vh)
    canvas = Image.new("RGBA", (W, H), (0, 0, 0, 0))

    def paint(mask, fill, opacity, bbox):
        if opacity < 1:
            mask = mask.point([round(i * opacity) for i in range(256)])
        if fill.startswith("url("):
            g = grads[fill[5:-1]]
            x0, y0, x1, y1 = (round(v) for v in bbox)
            layer = Image.new("RGB", (W, H))
            layer.paste(g.image((x1 - x0, y1 - y0)), (x0, y0))
        else:
            layer = Image.new("RGB", (W, H), _hex(fill))
        canvas.alpha_composite(Image.merge("RGBA", (*layer.split(), mask)))

    def walk(el, tf):
        for ch in el:
            t = _tag(ch)
            if t in ("defs", "linearGradient", "radialGradient", "title", "desc"):
                continue
            if t == "g":
                walk(ch, _transform(ch, tf))
                continue
            sx, sy, ox, oy = tf
            fill = ch.get("fill", "#000000")
            fo = float(ch.get("fill-opacity", 1)) * float(ch.get("opacity", 1))
            if t == "rect":
                x, y = float(ch.get("x", 0)), float(ch.get("y", 0))
                w, h = float(ch.get("width")), float(ch.get("height"))
                rx = float(ch.get("rx", ch.get("ry", 0)))
                box = (ox + x * sx, oy + y * sy, ox + (x + w) * sx, oy + (y + h) * sy)
                if fill != "none":
                    m = Image.new("L", (W, H), 0)
                    ImageDraw.Draw(m).rounded_rectangle((box[0], box[1], box[2] - 1, box[3] - 1), radius=rx * sx, fill=255)
                    paint(m, fill, fo, box)
                stroke = ch.get("stroke")
                if stroke and stroke != "none":
                    sw = float(ch.get("stroke-width", 1)) * sx
                    outer, inner = Image.new("L", (W, H), 0), Image.new("L", (W, H), 0)
                    ImageDraw.Draw(outer).rounded_rectangle(
                        (box[0] - sw / 2, box[1] - sw / 2, box[2] + sw / 2 - 1, box[3] + sw / 2 - 1), radius=rx * sx + sw / 2, fill=255)
                    ImageDraw.Draw(inner).rounded_rectangle(
                        (box[0] + sw / 2, box[1] + sw / 2, box[2] - sw / 2 - 1, box[3] - sw / 2 - 1), radius=max(0, rx * sx - sw / 2), fill=255)
                    ring = ImageChops.subtract(outer, inner)
                    paint(ring, stroke, float(ch.get("stroke-opacity", 1)) * float(ch.get("opacity", 1)), box)
            elif t == "path":
                if ch.get("stroke") not in (None, "none"):
                    raise ValueError("stroked paths are not supported")
                subs = _path_polygons(ch.get("d"), tf)
                m = Image.new("L", (W, H), 0)
                _fill_nonzero(ImageDraw.Draw(m), subs, H)
                pts = [p for s in subs for p in s]
                bbox = (min(p[0] for p in pts), min(p[1] for p in pts), max(p[0] for p in pts), max(p[1] for p in pts))
                paint(m, fill, fo, bbox)
            else:
                raise ValueError("unsupported element <%s>" % t)

    walk(root, base)
    return canvas.resize((width, height), Image.BOX) if ss > 1 else canvas
