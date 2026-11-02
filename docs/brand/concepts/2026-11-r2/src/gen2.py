#!/usr/bin/env python3
"""oneMK concept round 2 (Nov 2026): geometry, wordmark and flat/colour SVGs.

Needs Python with fontTools (+ brotli to read the woff2). The wordmark uses Geist (SIL OFL 1.1, Vercel),
instanced from the variable font Geist-Variable.woff2 (npm package `geist`, dist/fonts/geist-sans/).
Set GEIST_VF to that file. All final artwork is filled paths: no live text."""
import math, os, sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.environ.get("OUT", os.path.join(HERE, ".."))
GEIST_VF = os.environ.get("GEIST_VF", "/tmp/onemk-r2/dl/npm/package/dist/fonts/geist-sans/Geist-Variable.woff2")
CACHE = os.environ.get("FONT_CACHE", "/tmp/onemk-r2")
INK = "#111111"


def n(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def svg(w, h, title, body, defs=""):
    d = f"  <defs>\n{defs}  </defs>\n" if defs else ""
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n(w)} {n(h)}" width="{n(w)}" height="{n(h)}" '
            f'role="img" aria-labelledby="title">\n  <title id="title">{title}</title>\n{d}{body}</svg>\n')


def write(name, text):
    os.makedirs(os.path.dirname(os.path.join(OUT, name)) or OUT, exist_ok=True)
    with open(os.path.join(OUT, name), "w") as fh:
        fh.write(text)


def srect(x0, y0, x1, y1, r, a_k=1.6, d_k=0.25, corners="tl tr br bl"):
    """Rounded rect with smoothed corners; corners not listed stay square."""
    a, d = a_k * r, d_k * r
    c = corners.split()
    p = []
    p.append(f"M{n(x0 + a if 'tl' in c else x0)} {n(y0)}")
    if "tr" in c:
        p += [f"H{n(x1 - a)}", f"C{n(x1 - d)} {n(y0)} {n(x1)} {n(y0 + d)} {n(x1)} {n(y0 + a)}"]
    else:
        p.append(f"H{n(x1)}")
    if "br" in c:
        p += [f"V{n(y1 - a)}", f"C{n(x1)} {n(y1 - d)} {n(x1 - d)} {n(y1)} {n(x1 - a)} {n(y1)}"]
    else:
        p.append(f"V{n(y1)}")
    if "bl" in c:
        p += [f"H{n(x0 + a)}", f"C{n(x0 + d)} {n(y1)} {n(x0)} {n(y1 - d)} {n(x0)} {n(y1 - a)}"]
    else:
        p.append(f"H{n(x0)}")
    if "tl" in c:
        p += [f"V{n(y0 + a)}", f"C{n(x0)} {n(y0 + d)} {n(x0 + d)} {n(y0)} {n(x0 + a)} {n(y0)}"]
    else:
        p.append(f"V{n(y0)}")
    p.append("Z")
    return " ".join(p)


def circle(cx, cy, r):
    return (f"M{n(cx - r)} {n(cy)} A{n(r)} {n(r)} 0 1 1 {n(cx + r)} {n(cy)} "
            f"A{n(r)} {n(r)} 0 1 1 {n(cx - r)} {n(cy)} Z")


def fillet_poly(pts, radii):
    """Closed polygon with circular fillets. pts in order; radii per vertex (0 = sharp)."""
    m = len(pts)
    segs = []
    for i in range(m):
        A, V, B = pts[i - 1], pts[i], pts[(i + 1) % m]
        r = radii[i]
        u1 = (A[0] - V[0], A[1] - V[1]); l1 = math.hypot(*u1); u1 = (u1[0] / l1, u1[1] / l1)
        u2 = (B[0] - V[0], B[1] - V[1]); l2 = math.hypot(*u2); u2 = (u2[0] / l2, u2[1] / l2)
        if r <= 0:
            segs.append((V, V, None)); continue
        cosang = max(-1, min(1, u1[0] * u2[0] + u1[1] * u2[1]))
        th = math.acos(cosang) / 2
        t = r / math.tan(th)
        t = min(t, 0.48 * l1, 0.48 * l2)
        r_eff = t * math.tan(th)
        P1 = (V[0] + u1[0] * t, V[1] + u1[1] * t)
        P2 = (V[0] + u2[0] * t, V[1] + u2[1] * t)
        cross = (V[0] - A[0]) * (B[1] - V[1]) - (V[1] - A[1]) * (B[0] - V[0])
        sweep = 1 if cross > 0 else 0
        segs.append((P1, P2, (r_eff, sweep)))
    d = [f"M{n(segs[0][1][0])} {n(segs[0][1][1])}"]
    for i in range(1, m + 1):
        P1, P2, arc = segs[i % m]
        d.append(f"L{n(P1[0])} {n(P1[1])}")
        if arc:
            d.append(f"A{n(arc[0])} {n(arc[0])} 0 0 {arc[1]} {n(P2[0])} {n(P2[1])}")
    d.append("Z")
    return " ".join(d)


def ink_bbox(d):
    """Bounding box of a path made of M/L/A/Z commands (as written by fillet_poly)."""
    import re
    toks = re.findall(r"[MLAZ]|-?\d*\.?\d+(?:e-?\d+)?", d)
    i = 0; cur = (0.0, 0.0); pts = []
    while i < len(toks):
        t = toks[i]
        if t in ("M", "L"):
            cur = (float(toks[i + 1]), float(toks[i + 2])); pts.append(cur); i += 3
        elif t == "A":
            r = float(toks[i + 1]); sweep = int(float(toks[i + 5]))
            end = (float(toks[i + 6]), float(toks[i + 7]))
            mx, my = (cur[0] + end[0]) / 2, (cur[1] + end[1]) / 2
            dx, dy = end[0] - cur[0], end[1] - cur[1]
            q = math.hypot(dx, dy)
            h = math.sqrt(max(r * r - (q / 2) ** 2, 0))
            # small-arc centre on the side given by sweep
            nx, ny = -dy / q, dx / q
            sgn = 1 if sweep == 1 else -1
            cx, cy = mx + sgn * h * nx, my + sgn * h * ny
            a0 = math.atan2(cur[1] - cy, cur[0] - cx); a1 = math.atan2(end[1] - cy, end[0] - cx)
            da = a1 - a0
            if sweep == 1 and da < 0: da += 2 * math.pi
            if sweep == 0 and da > 0: da -= 2 * math.pi
            for k in range(1, 17):
                a = a0 + da * k / 16
                pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
            cur = end; i += 8
        else:
            i += 1
    xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
    return min(xs), min(ys), max(xs), max(ys)


def clip_poly(pts, keep):
    """Sutherland-Hodgman against a half-plane; keep(x,y)->signed value (>=0 keeps)."""
    out = []
    m = len(pts)
    for i in range(m):
        P, Q = pts[i], pts[(i + 1) % m]
        fp, fq = keep(*P), keep(*Q)
        if fp >= 0:
            out.append(P)
        if (fp >= 0) != (fq >= 0):
            t = fp / (fp - fq)
            out.append((P[0] + t * (Q[0] - P[0]), P[1] + t * (Q[1] - P[1])))
    return out


# ---------------- wordmark "oneMK" ----------------
_fonts = {}


def font(w):
    if w not in _fonts:
        p = os.path.join(CACHE, f"geist-{w}.ttf")
        if not os.path.exists(p):
            f = instancer.instantiateVariableFont(TTFont(GEIST_VF), {"wght": w})
            f.flavor = None
            f.save(p)
        _fonts[w] = TTFont(p)
    return _fonts[w]


def glyph(ch, w, x, base, s):
    f = font(w)
    g = f.getBestCmap()[ord(ch)]
    pen = SVGPathPen(f.getGlyphSet(), ntos=lambda v: n(v))
    f.getGlyphSet()[g].draw(TransformPen(pen, (s, 0, 0, -s, x, base)))
    adv, lsb = f["hmtx"][g]
    return pen.getCommands(), adv, lsb


WM = {
    # one weight / MK weight / MK scale / tracking (font units) / e->M gap (ink to ink, font units at the "one" size)
    "w1": dict(lw=600, cw=600, cs=1.00, tr=-12, em=100),
    "w2": dict(lw=600, cw=660, cs=0.88, tr=-12, em=92),
    "w3": dict(lw=500, cw=680, cs=0.92, tr=-10, em=96),
    "w4": dict(lw=560, cw=640, cs=0.90, tr=-14, em=90),
}


def wordmark(x, base, xh, v="w2"):
    """Return (d_one, d_mk, right_edge). xh = x-height in output units."""
    P = WM[v]
    s = xh / 534
    parts_one, cx = [], x
    for ch in "one":
        d, adv, lsb = glyph(ch, P["lw"], cx, base, s)
        parts_one.append(d)
        cx += (adv + P["tr"]) * s
    e_ink_right = cx - (P["tr"] + 39) * s  # e rsb ~39 at 600
    sc = s * P["cs"]
    f = font(P["cw"])
    m_lsb = f["hmtx"][f.getBestCmap()[ord("M")]][1]
    mx = e_ink_right + P["em"] * s - m_lsb * sc
    parts_mk = []
    for ch in "MK":
        d, adv, lsb = glyph(ch, P["cw"], mx, base, sc)
        parts_mk.append(d)
        last_adv, last_lsb = adv, lsb
        mx += (adv + P["tr"]) * sc
    # K ink right edge
    kf = font(P["cw"]); gs = kf.getGlyphSet()
    from fontTools.pens.boundsPen import BoundsPen
    bp = BoundsPen(gs); gs["K"].draw(bp)
    right = mx - (P["tr"] + last_adv - bp.bounds[2]) * sc
    return " ".join(parts_one), " ".join(parts_mk), right


# ---------------- D1: Lens ----------------

def lens(v="v1"):
    P = dict(
        v1=dict(x0=36, x1=220, r=40, g=16, R=50, c=12),
        v2=dict(x0=36, x1=220, r=40, g=18, R=54, c=14),
        v3=dict(x0=40, x1=216, r=40, g=16, R=58, c=14),
        v4=dict(x0=36, x1=220, r=36, g=16, R=46, c=12, ly0=76),
    )[v]
    x0, x1, r, g, R, c = P["x0"], P["x1"], P["r"], P["g"], P["R"], P["c"]
    y0, y1 = x0, x1
    cx = cy = 128
    Rc = R + c                  # radius of the cut around the lens
    xl, xr = cx - g / 2, cx + g / 2
    hy = math.sqrt(Rc ** 2 - (g / 2) ** 2)
    a, d = 1.6 * r, 0.25 * r
    ly0 = P.get("ly0", y0)
    if ly0 != y0:
        cy = (ly0 + y1) / 2 + 8
        hy = math.sqrt(Rc ** 2 - (g / 2) ** 2)
    left = " ".join([
        f"M{n(x0 + a)} {n(ly0)} H{n(xl)} V{n(cy - hy)}",
        f"A{n(Rc)} {n(Rc)} 0 0 0 {n(xl)} {n(cy + hy)}",
        f"V{n(y1)} H{n(x0 + a)}",
        f"C{n(x0 + d)} {n(y1)} {n(x0)} {n(y1 - d)} {n(x0)} {n(y1 - a)}",
        f"V{n(ly0 + a)} C{n(x0)} {n(ly0 + d)} {n(x0 + d)} {n(ly0)} {n(x0 + a)} {n(ly0)} Z"])
    right = " ".join([
        f"M{n(xr)} {n(y0)} H{n(x1 - a)}",
        f"C{n(x1 - d)} {n(y0)} {n(x1)} {n(y0 + d)} {n(x1)} {n(y0 + a)}",
        f"V{n(y1 - a)} C{n(x1)} {n(y1 - d)} {n(x1 - d)} {n(y1)} {n(x1 - a)} {n(y1)}",
        f"H{n(xr)} V{n(cy + hy)}",
        f"A{n(Rc)} {n(Rc)} 0 0 0 {n(xr)} {n(cy - hy)} Z"])
    disc = circle(cx, cy, R)
    return dict(left=left, right=right, disc=disc, d=f"{left} {right} {disc}", box=(x0, y0, x1, y1),
                panes=f"{left} {right}", P=P)


# ---------------- D2: Pointer ----------------

def pointer(v="v1"):
    P = dict(
        v1=dict(L=176, half=22.5, k=0.70, seam=0.36, g=14, rt=10, rb=16, rn=12, rc=4),
        v2=dict(L=180, half=21, k=0.68, seam=0.40, g=14, rt=8, rb=18, rn=14, rc=4),
        v3=dict(L=180, half=21, k=0.68, seam=0.40, g=16, rt=8, rb=18, rn=14, rc=5, tilt=-8),
        v4=dict(L=184, half=22.5, k=0.66, seam=0.36, g=14, rt=8, rb=20, rn=12, rc=4, perp=True),
        v5=dict(L=184, half=20, k=0.62, seam=0.42, g=16, rt=10, rb=22, rn=14, rc=5, perp=True, tilt=-10),
        v6=dict(L=184, half=22.5, k=0.70, seam=0.30, g=12, rt=8, rb=18, rn=12, rc=4, perp=True, tilt=-12),
        v7=dict(L=184, half=22.5, k=0.68213, seam=0.36, g=14, rt=9, rb=18, rn=12, rc=5, perp=True, tilt=-7.5, seam_ang=45),
        v8=dict(L=184, half=22.5, k=0.82805, seam=0.30, g=14, rt=8, rb=16, rn=10, rc=4, perp=True, tilt=22.5, seam_ang=90),
        v9=dict(L=184, half=22.5, k=0.82805, seam=0.34, g=16, rt=10, rb=20, rn=12, rc=5, perp=True, tilt=22.5, seam_ang=90),
        s2=dict(L=184, half=22.5, k=0.82805, seam=0.36, g=24, rt=6, rb=12, rn=8, rc=3, perp=True, tilt=22.5, seam_ang=90),
        s=dict(L=184, half=22.5, k=0.68213, seam=0.40, g=24, rt=6, rb=12, rn=8, rc=3, perp=True, tilt=-7.5, seam_ang=45),
    )[v]
    L, half, k = P["L"], math.radians(P["half"]), P["k"]
    ax = math.radians(45 + P.get("tilt", 0))   # axis direction (down-right)
    T = (0.0, 0.0)
    p1 = (L * math.cos(ax - half), L * math.sin(ax - half))
    p2 = (L * math.cos(ax + half), L * math.sin(ax + half))
    depth = k * L * math.cos(half)
    N = (depth * math.cos(ax), depth * math.sin(ax))
    kite = [T, p1, N, p2]          # tip, upper-right back corner, notch, lower-left back corner
    rads = {T: P["rt"], p1: P["rb"], N: P["rn"], p2: P["rb"]}
    xs = [q[0] for q in kite]; ys = [q[1] for q in kite]
    # seam: vertical line at seam fraction of the width from the tip
    g = P["g"]
    if P.get("perp"):
        sa = math.radians(P["seam_ang"]) if "seam_ang" in P else ax
        ux, uy = math.cos(sa), math.sin(sa)
        s0 = P["seam"] * L * math.cos(half)
        tip = clip_poly(kite, lambda x, y: (s0 - g / 2) - (x * ux + y * uy))
        body = clip_poly(kite, lambda x, y: (x * ux + y * uy) - (s0 + g / 2))
        sx = s0
    else:
        sx = min(xs) + P["seam"] * (max(xs) - min(xs))
        tip = clip_poly(kite, lambda x, y: (sx - g / 2) - x)
        body = clip_poly(kite, lambda x, y: x - (sx + g / 2))
    # first build at scale 1, measure the real ink (fillets pull corners in), then scale and centre on 256
    def build(S, ox, oy):
        mv = lambda pts: [(x * S + ox, y * S + oy) for x, y in pts]
        rr = {q: r_ * S for q, r_ in rads.items()}
        rc = P["rc"] * S
        t_ = fillet_poly(mv(tip), [rr.get(q, rc) for q in tip])
        b_ = fillet_poly(mv(body), [rr.get(q, rc) for q in body])
        w_ = fillet_poly(mv(kite), [rr[q] for q in kite])
        return t_, b_, w_
    t0, b0, _ = build(1, 0, 0)
    x0a, y0a, x1a, y1a = ink_bbox(t0 + " " + b0)
    S = P.get("size", 192) / max(x1a - x0a, y1a - y0a)
    ox = 128 - (x0a + x1a) / 2 * S
    oy = 126 - (y0a + y1a) / 2 * S          # a touch above centre (optical)
    tip_d, body_d, whole_d = build(S, ox, oy)
    bx0, by0, bx1, by1 = ink_bbox(tip_d + " " + body_d)
    ox = oy = 0
    return dict(tip=tip_d, body=body_d, whole=whole_d, d=f"{tip_d} {body_d}",
                box=(bx0, by0, bx1, by1), P=P, seam_x=sx)


# ---------------- D3: Converge ----------------

def converge(v="v1"):
    P = dict(
        v1=dict(x0=40, x1=216, y0=40, y1=216, slit=16, end=136, r=40, lefts=(40, 40, 40)),
        v2=dict(x0=36, x1=220, y0=44, y1=212, slit=16, end=132, r=40, lefts=(36, 60, 48)),
        v3=dict(x0=36, x1=220, y0=40, y1=216, slit=18, end=138, r=44, lefts=(60, 36, 48)),
        v4=dict(x0=28, x1=228, y0=40, y1=216, slit=16, end=140, r=88, lefts=(60, 28, 60), a_k=1.0),
        v5=dict(x0=28, x1=228, y0=44, y1=212, slit=14, end=150, r=84, lefts=(72, 28, 48), a_k=1.0),
    )[v]
    x1, y0, y1, sl, end, r = P["x1"], P["y0"], P["y1"], P["slit"], P["end"], P["r"]
    h = (y1 - y0 - 2 * sl) / 3
    rr = h / 2
    bands = [(y0 + i * (h + sl), y0 + i * (h + sl) + h) for i in range(3)]
    a_k = P.get("a_k", 1.6)
    a, d = a_k * r, (0.25 if a_k > 1.2 else 0.5523 * 0 + 0.4477) * r
    L = P["lefts"]
    (t0, t1), (m0, m1), (b0, b1) = bands
    s2 = sl / 2
    dd = " ".join([
        f"M{n(L[0] + rr)} {n(t0)} H{n(x1 - a)}",
        f"C{n(x1 - d)} {n(y0)} {n(x1)} {n(y0 + d)} {n(x1)} {n(y0 + a)}",
        f"V{n(y1 - a)} C{n(x1)} {n(y1 - d)} {n(x1 - d)} {n(y1)} {n(x1 - a)} {n(y1)}",
        f"H{n(L[2] + rr)} A{n(rr)} {n(rr)} 0 0 1 {n(L[2] + rr)} {n(b0)}",
        f"H{n(end)} A{n(s2)} {n(s2)} 0 0 0 {n(end)} {n(m1)}",
        f"H{n(L[1] + rr)} A{n(rr)} {n(rr)} 0 0 1 {n(L[1] + rr)} {n(m0)}",
        f"H{n(end)} A{n(s2)} {n(s2)} 0 0 0 {n(end)} {n(t1)}",
        f"H{n(L[0] + rr)} A{n(rr)} {n(rr)} 0 0 1 {n(L[0] + rr)} {n(t0)} Z"])
    strips = []
    for (b0_, b1_), lx in zip(bands, L):
        strips.append(f"M{n(lx + rr)} {n(b0_)} H{n(end + s2)} V{n(b1_)} H{n(lx + rr)} "
                      f"A{n(rr)} {n(rr)} 0 0 1 {n(lx + rr)} {n(b0_)} Z")
    x0 = min(L)
    # centre horizontally
    return dict(d=dd, strips=strips, bands=bands, box=(x0, y0, x1, y1), end=end, P=P)


# ---------------- files ----------------

def symbol_file(name, title, d, fill=INK):
    write(name, svg(256, 256, title, f'  <path id="symbol" fill="{fill}" d="{d}"/>\n'))


def lockup(name, title, sym_body, box, wv="w2", ink_h=180, xh_k=0.40, gap_k=0.30, word_fill=INK, mk_fill=None,
           defs=""):
    bx0, by0, bx1, by1 = box
    s = ink_h / (by1 - by0)
    tx = 28 - bx0 * s
    ty = 128 - ((by0 + by1) / 2) * s
    sym_right = tx + bx1 * s
    xh = xh_k * ink_h
    # centre the band between baseline and cap height (MK) on the symbol's middle
    capk = 710 / 534 * WM[wv]["cs"]
    base = 128 + (xh * capk) / 2
    d1, d2, right = wordmark(sym_right + gap_k * ink_h, base, xh, wv)
    W = math.ceil(right + 28)
    body = (f'  <g id="symbol" transform="translate({n(tx)} {n(ty)}) scale({s:.5f})">\n{sym_body}  </g>\n'
            f'  <g id="wordmark">\n    <path fill="{word_fill}" d="{d1}"/>\n    <path fill="{mk_fill or word_fill}" d="{d2}"/>\n  </g>\n')
    write(name, svg(W, 256, title, body, defs))


def wordmark_file(name, wv, fill=INK, mk_fill=None):
    xh = 100
    capk = 710 / 534 * WM[wv]["cs"]
    base = 128 + xh * capk / 2
    d1, d2, right = wordmark(24, base, xh, wv)
    body = (f'  <g id="wordmark">\n    <path fill="{fill}" d="{d1}"/>\n    <path fill="{mk_fill or fill}" d="{d2}"/>\n  </g>\n')
    write(name, svg(math.ceil(right + 24), 256, "oneMK wordmark", body))


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "explore"
    if mode == "explore":
        vs = sys.argv[2:] or ["v1", "v2", "v3"]
        for v in vs:
            for nm, fn in (("d1-lens", lens), ("d2-pointer", pointer), ("d3-converge", converge)):
                try:
                    symbol_file(f"iterations/{nm}-{v}.svg", f"oneMK {nm} {v}", fn(v)["d"])
                except KeyError:
                    pass
        for w in WM:
            wordmark_file(f"iterations/wordmark-{w}.svg", w)


# ---------------- D1 (round 2b): MK ligature with one shared stem ----------------

def para(xa, ya, xb, yb, w):
    """Parallelogram strip with horizontal width w whose centre line runs (xa,ya)->(xb,yb)."""
    return [(xa - w / 2, ya), (xa + w / 2, ya), (xb + w / 2, yb), (xb - w / 2, yb)]


def poly_d(pts):
    return "M" + " L".join(f"{n(x)} {n(y)}" for x, y in pts) + " Z"


def mk(v="v1"):
    P = dict(
        v1=dict(t=34, y0=44, y1=212, x0=24, gap=62, ang=60, kj=0.52, kx=56, vtx=None),
        v2=dict(t=36, y0=44, y1=212, x0=22, gap=58, ang=60, kj=0.55, kx=60, vtx=None, apex=True),
        v3=dict(t=38, y0=48, y1=208, x0=26, gap=56, ang=60, kj=0.56, kx=58, vtx=None, apex=True, base_v=True),
    )[v]
    t, y0, y1, x0 = P["t"], P["y0"], P["y1"], P["x0"]
    H = y1 - y0
    tan = math.tan(math.radians(P["ang"]))
    wd = t / math.sin(math.radians(P["ang"]))          # horizontal width of a diagonal of stroke t
    s1 = (x0, x0 + t)                                     # left stem
    s2 = (x0 + t + P["gap"], x0 + 2 * t + P["gap"])       # shared stem
    cxm = (s1[0] + s2[1]) / 2                              # M vertex x
    # M diagonals: outer edges start at the stems' outer top corners
    ldx = cxm - (s1[0] + wd / 2)
    vy = y0 + ldx * tan                                    # where the centre lines meet
    shapes = []
    shapes.append([(s1[0], y1), (s1[0], y0), (s1[1], y0), (s1[1], y1)])
    shapes.append([(s2[0], y1), (s2[0], y0), (s2[1], y0), (s2[1], y1)])
    shapes.append(para(s1[0] + wd / 2, y0, cxm, vy, wd))
    shapes.append(para(s2[1] - wd / 2, y0, cxm, vy, wd))
    # K arms from the shared stem
    jy = y0 + P["kj"] * H
    kx_end = s2[1] + P["kx"]
    up_dx = (jy - y0) / tan
    shapes.append(para(s2[1] - wd / 2 + 2, jy, s2[1] - wd / 2 + 2 + up_dx + wd / 2, y0, wd))
    lo_start_y = jy - 0.18 * H
    lo_dx = (y1 - lo_start_y) / tan
    shapes.append(para(s2[1] - wd / 2 + 6, lo_start_y, s2[1] - wd / 2 + 6 + lo_dx + wd / 2, y1, wd))
    # clip everything to the cap band
    clipped = []
    for s in shapes:
        s = clip_poly(s, lambda x, y: y - y0)
        s = clip_poly(s, lambda x, y: y1 - y)
        clipped.append(s)
    xs = [x for s in clipped for x, _ in s]
    ox = 128 - (min(xs) + max(xs)) / 2
    oy = 128 - (y0 + y1) / 2
    d = " ".join(poly_d([(x + ox, y + oy) for x, y in s]) for s in clipped)
    return dict(d=d, box=(min(xs) + ox, y0 + oy, max(xs) + ox, y1 + oy), P=P)


def converge_t(v="v6"):
    """Converge with tapered trails: three trails, thin at the left, merging into one disc-ended block."""
    P = dict(
        v6=dict(y0=44, y1=212, x1=228, end=150, lefts=(70, 28, 50), tip=0.42, slit=12),
        v7=dict(y0=40, y1=216, x1=228, end=142, lefts=(56, 28, 56), tip=0.36, slit=12),
    )[v]
    y0, y1, x1, end, sl = P["y0"], P["y1"], P["x1"], P["end"], P["slit"]
    H = y1 - y0
    h = (H - 2 * sl) / 3
    R = H / 2
    cx = x1 - R
    bands = [(y0 + i * (h + sl), y0 + i * (h + sl) + h) for i in range(3)]
    shapes = []
    # block: rect from end to cx + semicircle on the right
    block = (f"M{n(end)} {n(y0)} H{n(cx)} A{n(R)} {n(R)} 0 0 1 {n(cx)} {n(y1)} H{n(end)} Z")
    trails = []
    for (b0, b1), lx in zip(bands, P["lefts"]):
        mid = (b0 + b1) / 2
        hl = P["tip"] * h / 2
        # tapered trail: round tip radius hl at the left, widening to full band height at x=end
        trails.append(f"M{n(lx + hl)} {n(mid - hl)} L{n(end + 1)} {n(b0)} V{n(b1)} L{n(lx + hl)} {n(mid + hl)} "
                      f"A{n(hl)} {n(hl)} 0 0 1 {n(lx + hl)} {n(mid - hl)} Z")
    d = block + " " + " ".join(trails)
    x0 = min(P["lefts"])
    return dict(d=d, block=block, trails=trails, box=(x0, y0, x1, y1), P=P, end=end)


def lens2(v="v5"):
    """Lens, implied: the seam runs in from the top and bottom and stops at the lens circle;
    inside the circle the two panes are one. Optional ring (c>0) or capsule lens."""
    P = dict(
        v5=dict(x0=36, x1=220, r=42, g=18, R=56, c=0),
        v6=dict(x0=36, x1=220, r=42, g=16, R=48, c=0),
        v7=dict(x0=36, x1=220, r=42, g=16, R=44, c=10, cap=36),   # capsule lens with a clearance ring
    )[v]
    x0, x1, r, g, R, c = P["x0"], P["x1"], P["r"], P["g"], P["R"], P["c"]
    y0, y1 = x0, x1
    cx = cy = 128
    a, d = 1.6 * r, 0.25 * r
    xl, xr = cx - g / 2, cx + g / 2
    out = " ".join([
        f"M{n(x0 + a)} {n(y0)} H{n(x1 - a)}",
        f"C{n(x1 - d)} {n(y0)} {n(x1)} {n(y0 + d)} {n(x1)} {n(y0 + a)}",
        f"V{n(y1 - a)} C{n(x1)} {n(y1 - d)} {n(x1 - d)} {n(y1)} {n(x1 - a)} {n(y1)}",
        f"H{n(x0 + a)} C{n(x0 + d)} {n(y1)} {n(x0)} {n(y1 - d)} {n(x0)} {n(y1 - a)}",
        f"V{n(y0 + a)} C{n(x0)} {n(y0 + d)} {n(x0 + d)} {n(y0)} {n(x0 + a)} {n(y0)} Z"])
    if not c:
        hy = math.sqrt(R ** 2 - (g / 2) ** 2)
        # notches (holes, opposite winding): top one ends with the lens arc
        top = (f"M{n(xl)} {n(y0 - 1)} V{n(cy - hy)} A{n(R)} {n(R)} 0 0 0 {n(xr)} {n(cy - hy)} V{n(y0 - 1)} Z")
        bot = (f"M{n(xl)} {n(y1 + 1)} H{n(xr)} V{n(cy + hy)} A{n(R)} {n(R)} 0 0 0 {n(xl)} {n(cy + hy)} Z")
        return dict(d=f"{out} {top} {bot}", evenodd=True, outer=out, notches=f"{top} {bot}",
                    lens=circle(cx, cy, R), box=(x0, y0, x1, y1), P=P)
    # capsule lens: horizontal pill (height 2*cap/.. ) with clearance c, slit elsewhere
    hw = R + 20; hh = P["cap"]
    pill = srect(cx - hw, cy - hh, cx + hw, cy + hh, hh / 1.6 * 0.999)
    cut = srect(cx - hw - c, cy - hh - c, cx + hw + c, cy + hh + c, (hh + c) / 1.6 * 0.999)
    slit = f"M{n(xl)} {n(y0 - 1)} H{n(xr)} V{n(y1 + 1)} H{n(xl)} Z"
    return dict(d=None, outer=out, pill=pill, cut=cut, slit=slit, box=(x0, y0, x1, y1), P=P)
