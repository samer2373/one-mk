"""Tiny 2D geometry for the one·mk marks: flattened rounded rects, convex offsets and
polygon difference by convex pieces (so knockouts are real holes, not painted shapes)."""
import math

def F(v):
    s = ("%.2f" % v).rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s

def area(p):
    return sum(p[i][0]*p[(i+1) % len(p)][1] - p[(i+1) % len(p)][0]*p[i][1] for i in range(len(p))) / 2

def arc_pts(cx, cy, r, a0, a1, seg):
    return [(cx + r*math.cos(math.radians(a0 + (a1-a0)*i/seg)), cy + r*math.sin(math.radians(a0 + (a1-a0)*i/seg))) for i in range(seg+1)]

def rrect(x0, y0, x1, y1, r, seg=8):
    """Clockwise (screen coords, y down) rounded rectangle polygon."""
    if r <= 0:
        return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    p = []
    p += arc_pts(x1-r, y0+r, r, -90, 0, seg)
    p += arc_pts(x1-r, y1-r, r, 0, 90, seg)
    p += arc_pts(x0+r, y1-r, r, 90, 180, seg)
    p += arc_pts(x0+r, y0+r, r, 180, 270, seg)
    return p

def frame(x0, y0, x1, y1, t, r, ri=None, seg=8):
    """Rounded-rect ring as one keyhole polygon (outer clockwise, inner counter-clockwise)."""
    outer = rrect(x0, y0, x1, y1, r, seg)
    inner = rrect(x0+t, y0+t, x1-t, y1-t, ri if ri is not None else max(r-t, 1.5), seg)[::-1]
    # bridge along the bottom-left: rotate both so they start near each other
    oi = min(range(len(outer)), key=lambda i: (outer[i][0]-x0)**2 + (outer[i][1]-y1)**2)
    ii = min(range(len(inner)), key=lambda i: (inner[i][0]-x0)**2 + (inner[i][1]-y1)**2)
    o = outer[oi:] + outer[:oi]; n = inner[ii:] + inner[:ii]
    return o + [o[0]] + n + [n[0]]

def clip_half(poly, a, b, keep_left):
    """Sutherland-Hodgman against the line a->b; keep the side where cross(b-a, p-a) >= 0 (keep_left) or <= 0."""
    def f(p):
        v = (b[0]-a[0])*(p[1]-a[1]) - (b[1]-a[1])*(p[0]-a[0])
        return v if keep_left else -v
    out = []
    n = len(poly)
    for i in range(n):
        p, q = poly[i], poly[(i+1) % n]
        fp, fq = f(p), f(q)
        if fp >= 0: out.append(p)
        if (fp >= 0) != (fq >= 0):
            t = fp / (fp - fq); out.append((p[0] + t*(q[0]-p[0]), p[1] + t*(q[1]-p[1])))
    return out

def convex_offset(poly, d, seg=6):
    """Minkowski sum of a convex polygon with a disk of radius d (round corners)."""
    if d <= 0: return list(poly)
    s = 1 if area(poly) > 0 else -1     # area>0 means clockwise on screen (y down)
    out = []; n = len(poly)
    for i in range(n):
        p0, p1, p2 = poly[i-1], poly[i], poly[(i+1) % n]
        def nrm(a, b):
            ex, ey = b[0]-a[0], b[1]-a[1]; L = math.hypot(ex, ey)
            return (ey/L*s, -ex/L*s)       # outward normal
        n1, n2 = nrm(p0, p1), nrm(p1, p2)
        a1, a2 = math.atan2(n1[1], n1[0]), math.atan2(n2[1], n2[0])
        da = a2 - a1
        while da > math.pi: da -= 2*math.pi
        while da < -math.pi: da += 2*math.pi
        k = max(1, int(abs(da) / (math.pi/2) * seg))
        for j in range(k+1):
            a = a1 + da*j/k; out.append((p1[0] + d*math.cos(a), p1[1] + d*math.sin(a)))
    return out

def minus_convex(polys, K):
    """polys − K for a convex polygon K; returns disjoint pieces."""
    left = area(K) > 0     # clockwise on screen: interior is on the right of each edge in math terms -> cross >= 0
    edges = [(K[i], K[(i+1) % len(K)]) for i in range(len(K))]
    res = []
    for P in polys:
        for i, (a, b) in enumerate(edges):
            Q = clip_half(P, a, b, not left)            # outside edge i
            for (c, d) in edges[:i]:
                if len(Q) < 3: break
                Q = clip_half(Q, c, d, left)            # inside earlier edges
            if len(Q) >= 3 and abs(area(Q)) > 1e-3:
                res.append(Q)
    return res

def minus(polys, Ks):
    for K in Ks:
        polys = minus_convex(polys, K)
    return polys

def simplify(p, eps=1e-3):
    out = []
    for q in p:
        if out and abs(q[0]-out[-1][0]) < eps and abs(q[1]-out[-1][1]) < eps: continue
        out.append(q)
    # drop collinear points
    i = 0
    while len(out) > 3 and i < len(out):
        a, b, c = out[i-1], out[i], out[(i+1) % len(out)]
        if abs((b[0]-a[0])*(c[1]-a[1]) - (b[1]-a[1])*(c[0]-a[0])) < 1e-4:
            out.pop(i)
        else:
            i += 1
    return out

def _circle(a, b, c):
    ax, ay = a; bx, by = b; cx, cy = c
    d = 2*(ax*(by-cy) + bx*(cy-ay) + cx*(ay-by))
    if abs(d) < 1e-9: return None
    ux = ((ax*ax+ay*ay)*(by-cy) + (bx*bx+by*by)*(cy-ay) + (cx*cx+cy*cy)*(ay-by)) / d
    uy = ((ax*ax+ay*ay)*(cx-bx) + (bx*bx+by*by)*(ax-cx) + (cx*cx+cy*cy)*(bx-ax)) / d
    return ux, uy, math.hypot(ax-ux, ay-uy)

def _turn(a, b, c):
    v1 = (b[0]-a[0], b[1]-a[1]); v2 = (c[0]-b[0], c[1]-b[1])
    l = math.hypot(*v1)*math.hypot(*v2)
    if l == 0: return 0
    return math.acos(max(-1, min(1, (v1[0]*v2[0]+v1[1]*v2[1])/l)))

def _arcs(p, tol):
    """Turn runs of >= 4 points that lie on one circle into ('A', r, sweep, end) commands."""
    n = len(p); cmds = []; i = 0
    while i < n - 1:
        best = None
        j = i + 3
        while j < n:
            m = (i + j) // 2
            c = _circle(p[i], p[m], p[j])
            if not c or c[2] > 400: break
            ux, uy, r = c
            if any(abs(math.hypot(q[0]-ux, q[1]-uy) - r) > tol for q in p[i:j+1]): break
            if any(math.hypot(p[k+1][0]-p[k][0], p[k+1][1]-p[k][1]) > 0.7*r for k in range(i, j)): break
            if any(_turn(p[k-1], p[k], p[k+1]) > 0.75 for k in range(i+1, j)): break
            ch = [math.hypot(p[k+1][0]-p[k][0], p[k+1][1]-p[k][1]) for k in range(i, j)]
            if max(ch) > 3.0*min(ch) + 1e-6: break
            # all consecutive turns must have the same sign and be small (a real arc, not a corner)
            ok = True; sg = None
            for k in range(i+1, j):
                cr = (p[k][0]-p[k-1][0])*(p[k+1][1]-p[k][1]) - (p[k][1]-p[k-1][1])*(p[k+1][0]-p[k][0])
                s_ = cr > 0
                if sg is None: sg = s_
                elif s_ != sg: ok = False; break
            if not ok: break
            best = (j, r, 1 if sg else 0); j += 1
        if best:
            j, r, sw = best; cmds.append(("A", r, sw, p[j])); i = j
        else:
            cmds.append(("L", p[i+1])); i += 1
    return cmds

def d_of(polys, dx=0, dy=0, sc=1.0, tol=0.04, min_area=0.02):
    parts = []
    for p in polys:
        p = simplify(p)
        if len(p) < 3 or abs(area(p)) < min_area: continue
        p = [(x*sc+dx, y*sc+dy) for x, y in p]
        # start at a sharp corner if there is one so arcs are not split at the seam
        def turn(k):
            a, b, c = p[k-1], p[k], p[(k+1) % len(p)]
            v1 = (b[0]-a[0], b[1]-a[1]); v2 = (c[0]-b[0], c[1]-b[1])
            l = math.hypot(*v1)*math.hypot(*v2) or 1
            return abs(math.asin(max(-1, min(1, (v1[0]*v2[1]-v1[1]*v2[0])/l))))
        k0 = max(range(len(p)), key=turn)
        p = p[k0:] + p[:k0] + [p[k0]]
        out = [f"M{F(p[0][0])} {F(p[0][1])}"]
        for c in _arcs(p, tol*max(sc, 0.05) if sc < 1 else tol):
            if c[0] == "L": out.append(f"L{F(c[1][0])} {F(c[1][1])}")
            else: out.append(f"A{F(c[1])} {F(c[1])} 0 0 {c[2]} {F(c[3][0])} {F(c[3][1])}")
        out[-1] = out[-1] if not out[-1].startswith("L") else out[-1]
        parts.append("".join(out) + "Z")
    return " ".join(parts)

# ---------- the classic pointer ----------
CURSOR = [(0, 0), (0, 16.2), (4.1, 12.1), (6.82, 18.66), (9.13, 17.70), (6.6, 11.6), (11.6, 11.6)]
def cursor_pieces(tx, ty, h):
    """Convex pieces whose union is the pointer (tip at tx, ty; height h)."""
    u = h / 19.0
    P = [(tx + x*u, ty + y*u) for x, y in CURSOR]
    T, L, A, A2, B2, B, W = P
    return [[T, L, A], [T, A, B], [T, B, W], [A, A2, B2, B]]
def cursor_shape(tx, ty, h, soft):
    return [convex_offset(p, soft) for p in cursor_pieces(tx, ty, h)]
def cursor_knock(tx, ty, h, soft, gap):
    return [convex_offset(p, soft + gap) for p in cursor_pieces(tx, ty, h)]

# ---------- convex decomposition of devices, and union by edge cancellation ----------
def ring_pieces(x0, y0, x1, y1, t, ri, seg=8):
    """Rounded-rect ring (outer radius ri + t, concentric inner radius ri) as convex pieces."""
    r = ri + t
    P = []
    # straight bars (between the corners)
    P.append([(x0+r, y0), (x1-r, y0), (x1-r, y0+t), (x0+r, y0+t)])          # top
    P.append([(x0+r, y1-t), (x1-r, y1-t), (x1-r, y1), (x0+r, y1)])          # bottom
    P.append([(x0, y0+r), (x0+t, y0+r), (x0+t, y1-r), (x0, y1-r)])          # left
    P.append([(x1-t, y0+r), (x1, y0+r), (x1, y1-r), (x1-t, y1-r)])          # right
    for (cx, cy, a0) in ((x1-r, y0+r, -90), (x1-r, y1-r, 0), (x0+r, y1-r, 90), (x0+r, y0+r, 180)):
        o = arc_pts(cx, cy, r, a0, a0+90, seg); i = arc_pts(cx, cy, ri, a0, a0+90, seg)
        for k in range(seg):
            P.append([o[k], o[k+1], i[k+1], i[k]])
    return P

def Q(p, q=100):
    return (round(p[0]*q)/q, round(p[1]*q)/q)

def union_abutting(polys):
    """Union of polygons that only touch along edges (no overlaps): cancel shared edges, chain the rest.
    All input polygons are re-oriented to positive area first."""
    polys = [[Q(p) for p in P] for P in polys if abs(area(P)) > 1e-4]
    polys = [P if area(P) > 0 else P[::-1] for P in polys]
    verts = sorted(set(p for P in polys for p in P))
    edges = []
    for P in polys:
        n = len(P)
        for i in range(n):
            a, b = P[i], P[(i+1) % n]
            if a == b: continue
            # split edge at any vertex lying on it
            ax, ay, bx, by = a[0], a[1], b[0], b[1]
            L2 = (bx-ax)**2 + (by-ay)**2
            pts = [a, b]
            minx, maxx, miny, maxy = min(ax, bx)-1e-6, max(ax, bx)+1e-6, min(ay, by)-1e-6, max(ay, by)+1e-6
            for v in verts:
                if v == a or v == b or not (minx <= v[0] <= maxx and miny <= v[1] <= maxy): continue
                cr = (bx-ax)*(v[1]-ay) - (by-ay)*(v[0]-ax)
                if abs(cr) / math.sqrt(L2) < 0.006:
                    pts.append(v)
            pts.sort(key=lambda v: (v[0]-ax)*(bx-ax) + (v[1]-ay)*(by-ay))
            for j in range(len(pts)-1):
                if pts[j] != pts[j+1]: edges.append((pts[j], pts[j+1]))
    from collections import Counter
    cnt = Counter(edges)
    keep = []
    for e, c in cnt.items():
        rc = cnt.get((e[1], e[0]), 0)
        for _ in range(max(0, c - rc)): keep.append(e)
    nxt = {}
    for a, b in keep: nxt.setdefault(a, []).append(b)
    loops = []
    while nxt:
        a = next(iter(nxt)); start = a; loop = [a]
        while True:
            lst = nxt.get(a)
            if not lst: break
            b = lst.pop()
            if not lst: del nxt[a]
            if b == start: break
            loop.append(b); a = b
        if len(loop) >= 3: loops.append(loop)
    return loops

def fillet(poly, r, seg=4, reflex_r=None):
    """Round each corner of a simple polygon with radius r (convex corners) / reflex_r (reflex)."""
    s = 1 if area(poly) > 0 else -1
    out = []; n = len(poly)
    for i in range(n):
        p0, p1, p2 = poly[i-1], poly[i], poly[(i+1) % n]
        v1 = (p0[0]-p1[0], p0[1]-p1[1]); v2 = (p2[0]-p1[0], p2[1]-p1[1])
        l1, l2 = math.hypot(*v1), math.hypot(*v2)
        u1 = (v1[0]/l1, v1[1]/l1); u2 = (v2[0]/l2, v2[1]/l2)
        cr = (p1[0]-p0[0])*(p2[1]-p1[1]) - (p1[1]-p0[1])*(p2[0]-p1[0])
        convex = cr * s > 0
        rr = r if convex else (reflex_r if reflex_r is not None else 0)
        ang = math.acos(max(-1, min(1, u1[0]*u2[0] + u1[1]*u2[1])))
        if rr <= 0 or ang < 1e-3:
            out.append(p1); continue
        d = rr / math.tan(ang/2); d = min(d, l1*0.45, l2*0.45); rr = d * math.tan(ang/2)
        a = (p1[0]+u1[0]*d, p1[1]+u1[1]*d); b = (p1[0]+u2[0]*d, p1[1]+u2[1]*d)
        bis = (u1[0]+u2[0], u1[1]+u2[1]); bl = math.hypot(*bis); bis = (bis[0]/bl, bis[1]/bl)
        c = (p1[0]+bis[0]*rr/math.sin(ang/2), p1[1]+bis[1]*rr/math.sin(ang/2))
        a0 = math.atan2(a[1]-c[1], a[0]-c[0]); a1 = math.atan2(b[1]-c[1], b[0]-c[0])
        da = a1 - a0
        while da > math.pi: da -= 2*math.pi
        while da < -math.pi: da += 2*math.pi
        for k in range(seg+1):
            t = a0 + da*k/seg; out.append((c[0]+rr*math.cos(t), c[1]+rr*math.sin(t)))
    return out

def cursor_outline(tx, ty, h, r_convex, r_reflex=0):
    u = h / 19.0
    P = [(tx + x*u, ty + y*u) for x, y in CURSOR]
    return fillet(P, r_convex, 4, r_reflex)
