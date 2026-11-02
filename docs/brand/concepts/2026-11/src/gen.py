#!/usr/bin/env python3
"""Generator for the 2026-11 one·mk concept round. Writes SVGs into OUT.
All geometry on a 256 grid; final files are filled paths only."""
import math, os, sys
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
FONT = os.path.join(OUT, "..", "..", "..", "..", "crates", "app", "fonts", "Geist-SemiBold.ttf")
INK = "#111111"


def n(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def svg(w, h, title, body):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n(w)} {n(h)}" width="{n(w)}" height="{n(h)}" '
            f'role="img" aria-labelledby="title">\n  <title id="title">{title}</title>\n{body}</svg>\n')


def write(name, text):
    with open(os.path.join(OUT, name), "w") as fh:
        fh.write(text)


# ---------- primitives ----------

def srect(x0, y0, x1, y1, r, a_k=1.6, d_k=0.25, cw=True):
    """Rounded rect with smoothed (continuous-curvature-ish) corners.
    Corner starts a = a_k*r from the corner; both handles sit d = d_k*r from it."""
    a, d = a_k * r, d_k * r
    p = [f"M{n(x0 + a)} {n(y0)}", f"H{n(x1 - a)}",
         f"C{n(x1 - d)} {n(y0)} {n(x1)} {n(y0 + d)} {n(x1)} {n(y0 + a)}", f"V{n(y1 - a)}",
         f"C{n(x1)} {n(y1 - d)} {n(x1 - d)} {n(y1)} {n(x1 - a)} {n(y1)}", f"H{n(x0 + a)}",
         f"C{n(x0 + d)} {n(y1)} {n(x0)} {n(y1 - d)} {n(x0)} {n(y1 - a)}", f"V{n(y0 + a)}",
         f"C{n(x0)} {n(y0 + d)} {n(x0 + d)} {n(y0)} {n(x0 + a)} {n(y0)}", "Z"]
    return " ".join(p)


def circle(cx, cy, r, ccw=False):
    s = 0 if ccw else 1
    return (f"M{n(cx - r)} {n(cy)} A{n(r)} {n(r)} 0 1 {s} {n(cx + r)} {n(cy)} "
            f"A{n(r)} {n(r)} 0 1 {s} {n(cx - r)} {n(cy)} Z")


# ---------- wordmark (Geist SemiBold outlines, OFL) ----------
_font = TTFont(FONT)
_gs = _font.getGlyphSet()
_cmap = _font.getBestCmap()
XH = 534


def glyph_path(ch, x, base, s):
    g = _cmap[ord(ch)]
    pen = SVGPathPen(_gs, ntos=lambda v: n(v))
    tp = TransformPen(pen, (s, 0, 0, -s, x, base))
    _gs[g].draw(tp)
    return pen.getCommands(), _font["hmtx"][g][0] * s


def wordmark(x, base, xh, dot="circle", track=-14, dot_r_k=0.135, dot_gap=None, dot_y_k=0.5):
    """Set 'one·mk'. xh = x-height in output units. Returns (path_d, dot_d, right_edge).
    The interpunct is drawn, not taken from the font: a circle or square centred on the x-height."""
    s = xh / XH
    d_parts, cx = [], x
    for ch in "one":
        d, adv = glyph_path(ch, cx, base, s)
        d_parts.append(d)
        cx += adv + track * s
    # e's right sidebearing ~39; m's left ~70. Space the dot optically.
    r = dot_r_k * XH * s
    gap = (dot_gap if dot_gap is not None else 70) * s
    e_right = cx - (39 + track) * s  # ink edge of e
    dcx = e_right + gap + r
    dcy = base - dot_y_k * xh
    if dot == "circle":
        dot_d = circle(dcx, dcy, r)
    else:
        dot_d = f"M{n(dcx - r)} {n(dcy - r)} H{n(dcx + r)} V{n(dcy + r)} H{n(dcx - r)} Z"
    mx = dcx + r + gap - 70 * s  # m's ink starts at lsb 70
    for ch in "mk":
        d, adv = glyph_path(ch, mx, base, s)
        d_parts.append(d)
        mx += adv + track * s
    right = mx - (628 - 582 + track) * s
    return " ".join(d_parts), dot_d, right


# ---------- concept A: Edge ----------

def edge(v):
    """Pane (smoothed rounded rect) + a dot centred on its right edge: the half inside is a hole,
    the half outside is solid. The pane's straight edge always covers the dot (no corner slivers)."""
    P = dict(
        v9=dict(w=128, dr=54, r=22, m=4, dy=0),
        v12=dict(w=128, dr=52, r=20, m=10, dy=0),
        v10=dict(w=136, dr=56, r=24, m=8, dy=0),
        v11=dict(w=120, dr=58, r=20, m=4, dy=0),
        s16=dict(w=124, dr=60, r=18, m=2, dy=0),
    )[v]
    w, dr, r, m = P["w"], P["dr"], P["r"], P["m"]
    h = 2 * dr + 2 * 1.6 * r + 2 * m
    y0, y1 = 128 - h / 2, 128 + h / 2
    tw = w + dr
    x0 = (256 - tw) / 2
    x1 = x0 + w
    cy = 128 + P["dy"]
    pane = srect(x0, y0, x1, y1, r)
    hole = f"M{n(x1)} {n(cy - dr)} A{n(dr)} {n(dr)} 0 0 0 {n(x1)} {n(cy + dr)} Z"
    bump = f"M{n(x1)} {n(cy - dr)} A{n(dr)} {n(dr)} 0 0 1 {n(x1)} {n(cy + dr)} Z"
    return dict(pane=pane, hole=hole, bump=bump, x1=x1, cy=cy, dr=dr,
                d=f"{pane} {hole} {bump}", geom=(x0, y0, x1, y1, r), box=(x0, y0, x1 + dr, y1))


# ---------- concept B: Seam glyph ]·[ ----------

def bracket(xs, y0, y1, L, t, ro, ri, side):
    """side='R' -> ']' with stem at xs..xs+t (stem's left edge xs) and arms to the left.
    side='L' -> '[' mirrored."""
    a, d = 1.6 * ro, 0.25 * ro
    ai, di = 1.6 * ri, 0.25 * ri
    if side == "R":
        xr = xs + t; xl = xr - L; xi = xs
        return " ".join([
            f"M{n(xl)} {n(y0)} H{n(xr - a)}",
            f"C{n(xr - d)} {n(y0)} {n(xr)} {n(y0 + d)} {n(xr)} {n(y0 + a)}",
            f"V{n(y1 - a)}",
            f"C{n(xr)} {n(y1 - d)} {n(xr - d)} {n(y1)} {n(xr - a)} {n(y1)}",
            f"H{n(xl)} V{n(y1 - t)} H{n(xi - ai)}",
            f"C{n(xi - di)} {n(y1 - t)} {n(xi)} {n(y1 - t - di)} {n(xi)} {n(y1 - t - ai)}",
            f"V{n(y0 + t + ai)}",
            f"C{n(xi)} {n(y0 + t + di)} {n(xi - di)} {n(y0 + t)} {n(xi - ai)} {n(y0 + t)}",
            f"H{n(xl)} Z"])
    else:
        xl = xs; xr = xl + L; xi = xs + t
        return " ".join([
            f"M{n(xr)} {n(y0)} H{n(xl + a)}",
            f"C{n(xl + d)} {n(y0)} {n(xl)} {n(y0 + d)} {n(xl)} {n(y0 + a)}",
            f"V{n(y1 - a)}",
            f"C{n(xl)} {n(y1 - d)} {n(xl + d)} {n(y1)} {n(xl + a)} {n(y1)}",
            f"H{n(xr)} V{n(y1 - t)} H{n(xi + ai)}",
            f"C{n(xi + di)} {n(y1 - t)} {n(xi)} {n(y1 - t - di)} {n(xi)} {n(y1 - t - ai)}",
            f"V{n(y0 + t + ai)}",
            f"C{n(xi)} {n(y0 + t + di)} {n(xi + di)} {n(y0 + t)} {n(xi + ai)} {n(y0 + t)}",
            f"H{n(xr)} Z"])


def seam(v):
    P = dict(
        v1=dict(H=176, t=28, L=60, G=72, r=18, ro=30, ri=4, dot="circle"),
        v2=dict(H=168, t=30, L=56, G=64, r=20, ro=34, ri=6, dot="square"),
        v3=dict(H=176, t=32, L=58, G=68, r=20, ro=36, ri=6, dot="circle"),
        v4=dict(H=180, t=32, L=60, G=64, r=20, ro=36, ri=6, dot="circle"),
        s16=dict(H=192, t=40, L=64, G=72, r=24, ro=36, ri=4, dot="circle"),
    )[v]
    H, t, L, G, r = P["H"], P["t"], P["L"], P["G"], P["r"]
    y0, y1 = 128 - H / 2, 128 + H / 2
    left = bracket(128 - G / 2 - t, y0, y1, L, t, P["ro"], P["ri"], "R")
    right = bracket(128 + G / 2, y0, y1, L, t, P["ro"], P["ri"], "L")
    if P["dot"] == "circle":
        dot = circle(128, 128, r)
    else:
        dot = f"M{n(128 - r)} {n(128 - r)} H{n(128 + r)} V{n(128 + r)} H{n(128 - r)} Z"
    return dict(d=f"{left} {right} {dot}", frame=f"{left} {right}", dot=dot, P=P,
                box=(128 - G / 2 - L, y0, 128 + G / 2 + L, y1))


# ---------- concept C: Shift ----------

def shift(v):
    P = dict(
        v1=dict(R=96, g=16, dl=12),
        v2=dict(R=94, g=18, dl=16),
        v3=dict(R=94, g=20, dl=20),
        v4=dict(R=96, g=18, dl=18),
        v5=dict(R=88, g=18, dl=16),
        s16=dict(R=100, g=28, dl=22),
    )[v]
    R, g, dl = P["R"], P["g"], P["dl"]
    cxl, cyl = 128 - g / 2, 128 - dl
    cxr, cyr = 128 + g / 2, 128 + dl
    left = f"M{n(cxl)} {n(cyl - R)} A{n(R)} {n(R)} 0 0 0 {n(cxl)} {n(cyl + R)} Z"
    right = f"M{n(cxr)} {n(cyr - R)} A{n(R)} {n(R)} 0 0 1 {n(cxr)} {n(cyr + R)} Z"
    return dict(d=f"{left} {right}", left=left, right=right, P=P,
                box=(cxl - R, cyl - R, cxr + R, cyr + R))


# ---------- output ----------

def symbol_file(name, title, d, evenodd=False):
    fr = ' fill-rule="evenodd"' if evenodd else ""
    write(name, svg(256, 256, title, f'  <path id="symbol" fill="{INK}"{fr} d="{d}"/>\n'))


def lockup_file(name, title, sym_d, evenodd, dotstyle, box, ink_h=192, xh_k=0.5, gap_k=0.3,
                fills=None, word_fill=None, dot_fill=None, title_suffix=""):
    """Horizontal lockup, height 256. box = symbol ink box on its 256 grid (x0, y0, x1, y1).
    The symbol is scaled so its ink is ink_h tall; the x-height band is centred on the symbol's middle,
    so the interpunct sits on the symbol's horizontal axis."""
    bx0, by0, bx1, by1 = box
    s = ink_h / (by1 - by0)
    tx = 24 - bx0 * s
    ty = 128 - ((by0 + by1) / 2) * s
    sym_right = tx + bx1 * s
    xh = xh_k * ink_h
    gap = gap_k * ink_h
    base = 128 + xh / 2
    wd, dot, right = wordmark(sym_right + gap, base, xh, dot=dotstyle)
    W = right + 24
    fr = ' fill-rule="evenodd"' if evenodd else ""
    if fills is None:
        sym = f'    <path fill="{INK}"{fr} d="{sym_d}"/>\n'
    else:
        sym = "".join(f'    <path fill="{c}"{fr} d="{d}"/>\n' for d, c in fills)
    wf = word_fill or INK
    df = dot_fill or wf
    body = (f'  <g id="symbol" transform="translate({n(tx)} {n(ty)}) scale({n(s) if len(n(s))<7 else round(s,5)})">\n'
            f'{sym}  </g>\n'
            f'  <g id="wordmark">\n    <path fill="{wf}" d="{wd}"/>\n    <path id="interpunct" fill="{df}" d="{dot}"/>\n  </g>\n')
    write(name, svg(math.ceil(W), 256, title, body))


def wordmark_file(name, title, dotstyle):
    xh = 104
    wd, dot, right = wordmark(16, 128 + xh / 2, xh, dot=dotstyle)
    body = (f'  <g id="wordmark" fill="{INK}">\n    <path d="{wd}"/>\n    <path id="interpunct" d="{dot}"/>\n  </g>\n')
    write(name, svg(math.ceil(right + 16), 256, title, body))


COL = {
    "a-edge": dict(ink="#121418", acc="#C6F24E", light="#F2F4F7"),
    "b-seam": dict(ink="#121418", acc="#7B6CFF", light="#F2F4F7"),
    "c-shift": dict(ink="#121418", acc="#F0459A", light="#F2F4F7"),
}


def colour_parts(k, obj, ground):
    """(d, fill) parts for the colour versions. ground='light' or 'dark'."""
    c = COL[k]
    base = c["ink"] if ground == "light" else c["light"]
    if k == "a-edge":
        # pane (with the hole) in ink; the half of the dot that has crossed in the accent
        return [(f'{obj["pane"]} {obj["hole"]}', base), (obj["bump"], c["acc"])]
    if k == "b-seam":
        return [(obj["frame"], base), (obj["dot"], c["acc"])]
    return [(obj["left"], base), (obj["right"], c["acc"])]


if __name__ == "__main__":
    A, B, C = sys.argv[1:4] if len(sys.argv) > 3 else ("v12", "v4", "v5")
    e, b, c = edge(A), seam(B), shift(C)
    for k, (obj, eo, dot, nm) in {"a-edge": (e, True, "circle", "Edge"), "b-seam": (b, False, "circle", "Seam"),
                                  "c-shift": (c, False, "circle", "Shift")}.items():
        lockup_file(f"{k}-lockup.svg", f"one·mk {nm} lockup", obj["d"], eo, dot, obj["box"])
        symbol_file(f"{k}-symbol.svg", f"one·mk {nm}", obj["d"], evenodd=eo)
        col = COL[k]
        for ground in ("light", "dark"):
            parts = colour_parts(k, obj, ground)
            fr = ' fill-rule="evenodd"' if eo else ""
            body = "".join(f'  <path fill="{f}"{fr} d="{d}"/>\n' for d, f in parts)
            suffix = "color" if ground == "light" else "reversed"
            write(f"{k}-symbol-{suffix}.svg", svg(256, 256, f"one·mk {nm} ({suffix})", body))
            wf = col["ink"] if ground == "light" else col["light"]
            lockup_file(f"{k}-lockup-{suffix}.svg", f"one·mk {nm} lockup ({suffix})", None, eo, dot, obj["box"],
                        fills=parts, word_fill=wf, dot_fill=col["acc"])
    wordmark_file("wordmark.svg", "one·mk wordmark (Geist SemiBold outlines, custom interpunct)", "circle")
