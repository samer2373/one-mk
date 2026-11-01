"""oneMK Pointer mark: geometry.

A pointer (no tail) whose tip is cut off by a screen edge: the tip has already crossed to the next
computer and is lit. Everything is built from straight polygons with circular fillets, so the same
construction gives the 256 master, the pixel-fitted 16/22/24 px drawings, hollow (outline) versions
for the tray states, and the inset tip used on light grounds.

Angles are exact: the pointer's long edges run at 15 deg and 60 deg, the notch edges at 165 deg and
90 deg, and the cut at 135 deg (perpendicular to 45 deg)."""
import math

# ---------------------------------------------------------------- palette (see README)
GROUND = "#0F1216"        # window ground; ink on light grounds
INK = "#EDEFF2"           # ink on dark grounds
LIGHT_TRAY_INK = "#1B1F24"
ACCENT = "#C6F432"        # Volt: the lit tip, switches-on, primary button, focus ring, the pointer on the desk
ACCENT_LIT = "#D7FF5E"    # hover
ACCENT_INK = "#141A06"    # text and icons on the accent
ACCENT_DEEP = "#4E7A00"   # accent as a graphic on light grounds (5.1:1 on white)
OK = "#3CCFE6"            # connected / ok status (never the accent)
TILE_TOP, TILE_BOTTOM = "#1D3A2A", "#060B09"   # app-icon tile: deep green-black


def n(v):
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


# ---------------------------------------------------------------- polygon tools

def clip(pts, f):
    """Sutherland-Hodgman: keep the part where f(x, y) >= 0."""
    out = []
    for i in range(len(pts)):
        P, Q = pts[i], pts[(i + 1) % len(pts)]
        fp, fq = f(*P), f(*Q)
        if fp >= 0:
            out.append(P)
        if (fp >= 0) != (fq >= 0):
            t = fp / (fp - fq)
            out.append((P[0] + t * (Q[0] - P[0]), P[1] + t * (Q[1] - P[1])))
    return out


def area(pts):
    return 0.5 * sum(pts[i][0] * pts[(i + 1) % len(pts)][1] - pts[(i + 1) % len(pts)][0] * pts[i][1]
                     for i in range(len(pts)))


def offset(pts, w):
    """Move every edge inward by w (w < 0: outward) and re-intersect. Fine for these simple shapes."""
    s = 1 if area(pts) > 0 else -1   # y-down coordinates: positive area = clockwise on screen
    m = len(pts)
    lines = []
    for i in range(m):
        (x0, y0), (x1, y1) = pts[i], pts[(i + 1) % m]
        dx, dy = x1 - x0, y1 - y0
        L = math.hypot(dx, dy)
        nx, ny = -dy / L * s, dx / L * s          # inward normal
        lines.append(((x0 + nx * w, y0 + ny * w), (dx, dy)))
    out = []
    for i in range(m):
        (p, d), (q, e) = lines[i - 1], lines[i]
        den = d[0] * e[1] - d[1] * e[0]
        if abs(den) < 1e-9:
            out.append(q); continue
        t = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / den
        out.append((p[0] + d[0] * t, p[1] + d[1] * t))
    return out


def fillet(pts, radii):
    """Closed path with circular fillets (radius per vertex, 0 = sharp)."""
    m = len(pts)
    segs = []
    for i in range(m):
        A, V, B = pts[i - 1], pts[i], pts[(i + 1) % m]
        r = radii[i]
        u1 = (A[0] - V[0], A[1] - V[1]); l1 = math.hypot(*u1); u1 = (u1[0] / l1, u1[1] / l1)
        u2 = (B[0] - V[0], B[1] - V[1]); l2 = math.hypot(*u2); u2 = (u2[0] / l2, u2[1] / l2)
        if r <= 0.01:
            segs.append((V, V, None)); continue
        th = math.acos(max(-1, min(1, u1[0] * u2[0] + u1[1] * u2[1]))) / 2
        t = min(r / math.tan(th), 0.48 * l1, 0.48 * l2)
        re = t * math.tan(th)
        P1 = (V[0] + u1[0] * t, V[1] + u1[1] * t)
        P2 = (V[0] + u2[0] * t, V[1] + u2[1] * t)
        cross = (V[0] - A[0]) * (B[1] - V[1]) - (V[1] - A[1]) * (B[0] - V[0])
        segs.append((P1, P2, (re, 1 if cross > 0 else 0)))
    d = [f"M{n(segs[0][1][0])} {n(segs[0][1][1])}"]
    for i in range(1, m + 1):
        P1, P2, arc = segs[i % m]
        d.append(f"L{n(P1[0])} {n(P1[1])}")
        if arc:
            d.append(f"A{n(arc[0])} {n(arc[0])} 0 0 {arc[1]} {n(P2[0])} {n(P2[1])}")
    d.append("Z")
    return " ".join(d)


def bbox(polys):
    xs = [x for p in polys for x, _ in p]; ys = [y for p in polys for _, y in p]
    return min(xs), min(ys), max(xs), max(ys)


# ---------------------------------------------------------------- the pointer

MASTER = dict(L=184, half=22.5, axis=37.5, notch=0.68213, seam=0.36, gap=6.5,
              r_tip=9, r_back=18, r_notch=12, r_cut=1.5, size=192, grid=256, cy_shift=-2)
# before/after: the round-2 concept used gap=14 and r_cut=5 (the tip read as a separate triangle)
CONCEPT_R2 = dict(MASTER, gap=14, r_cut=5)

# Pixel-fitted drawings: same construction, sized for the pixel grid, with a whole-pixel gap.
SMALL = {
    16: dict(MASTER, size=15.0, grid=16, gap=1.0, r_tip=0.55, r_back=1.25, r_notch=0.8, r_cut=0.12, cy_shift=0),
    22: dict(MASTER, size=20.4, grid=22, gap=1.25, r_tip=0.7, r_back=1.7, r_notch=1.1, r_cut=0.15, cy_shift=0),
    24: dict(MASTER, size=22.2, grid=24, gap=1.4, r_tip=0.8, r_back=1.9, r_notch=1.2, r_cut=0.18, cy_shift=0),
}


def pointer(P=MASTER):
    """Returns dict(tip=[pts], body=[pts], tip_r=[radii], body_r=[radii], box) in grid units."""
    L, half, ax = P["L"], math.radians(P["half"]), math.radians(P["axis"])
    T = (0.0, 0.0)
    p1 = (L * math.cos(ax - half), L * math.sin(ax - half))
    p2 = (L * math.cos(ax + half), L * math.sin(ax + half))
    depth = P["notch"] * L * math.cos(half)
    N = (depth * math.cos(ax), depth * math.sin(ax))
    kite = [T, p1, N, p2]
    R = {T: P["r_tip"], p1: P["r_back"], N: P["r_notch"], p2: P["r_back"]}
    u = (math.cos(math.radians(45)), math.sin(math.radians(45)))
    s0 = P["seam"] * L * math.cos(half)
    g = P["gap"]
    # the gap is measured perpendicular to the cut; build at scale 1 first, then fit to the grid
    def build(gap):
        tip = clip(kite, lambda x, y: (s0 - gap / 2) - (x * u[0] + y * u[1]))
        body = clip(kite, lambda x, y: (x * u[0] + y * u[1]) - (s0 + gap / 2))
        return tip, body
    tip, body = build(0)
    x0, y0, x1, y1 = bbox([tip, body])
    S = P["size"] / max(x1 - x0, y1 - y0)
    tip, body = build(g / S)                      # gap given in grid units
    c = P["grid"] / 2
    ox = c - (x0 + x1) / 2 * S
    oy = c + P.get("cy_shift", 0) - (y0 + y1) / 2 * S
    mv = lambda pts: [(x * S + ox, y * S + oy) for x, y in pts]
    rr = lambda pts: [R[q] if q in R else P["r_cut"] for q in pts]
    tr, br = rr(tip), rr(body)
    tip, body = mv(tip), mv(body)
    return dict(tip=tip, body=body, tip_r=tr, body_r=br, box=bbox([tip, body]), P=P)


def paths(ptr):
    return fillet(ptr["tip"], ptr["tip_r"]), fillet(ptr["body"], ptr["body_r"])


def hollow(pts, radii, w):
    """Outline (ring) of a filleted polygon, stroke w inside the edge. Use with fill-rule evenodd."""
    inner = offset(pts, w)
    ir = [max(r - w, min(r, w) * 0.35) for r in radii]
    return fillet(pts, radii) + " " + fillet(inner, ir)


def inset(pts, radii, w):
    inner = offset(pts, w)
    return fillet(inner, [max(r - w, 0.4 * r) for r in radii])
