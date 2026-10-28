import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mark import M, S16, S22, S24
from geom import CURSOR, F
def g2(g, unit):
    dx, dy = g["shift"]; t = g["t"]
    rows = []
    def R(name, x0, y0, x1, y1, r, note=""):
        rows.append(f"| {name} | {F(x0+dx)} | {F(y0+dy)} | {F(x1+dx)} | {F(y1+dy)} | {F(r)} | {note} |")
    x0, y0, x1, y1, ri = g["mon"]
    R("monitor (outer)", x0, y0, x1, y1, ri+t, f"ring: subtract inner below")
    R("monitor (inner, hole)", x0+t, y0+t, x1-t, y1-t, ri, "")
    R("monitor neck", *g["neck"], 0)
    fx0, fy0, fx1, fy1, fr = g["foot"]; R("monitor foot", fx0, fy0, fx1, fy1, fr)
    x0, y0, x1, y1, ri = g["lap"]
    R("laptop screen (outer)", x0, y0, x1, y1, ri+t)
    R("laptop screen (inner, hole)", x0+t, y0+t, x1-t, y1-t, ri)
    bx0, by0, bx1, by1, br = g["base"]; R("laptop base", bx0, by0, bx1, by1, br)
    tx, ty = g["tip"]; u = g["h"] / 19.0
    pts = ", ".join(f"({F(tx+x*u+dx)}, {F(ty+y*u+dy)})" for x, y in CURSOR)
    gl = g.get("litgap", 0 if t < 4 else t*0.6)
    mx0, my0, mx1, my1, ri = g["mon"]
    lit = f"({F(mx0+t+gl+dx)}, {F(my0+t+gl+dy)}) – ({F(mx1-t-gl+dx)}, {F(my1-t-gl+dy)}), radius {F(max(ri-gl*0.6, 0.6))}"
    return rows, pts, g["gap"], g["soft"]*2.2, lit
out = []
for name, g, unit, use in (("16 px grid", S16, 16, "16 and 20 px (and @1x 16)"), ("22 px grid", S22, 22, "22 px"),
                           ("24 px grid", S24, 24, "24 px"), ("256 grid (master)", M, 256, "32 px and up, every @2x size")):
    rows, pts, gap, soft, lit = g2(g, unit)
    out.append(f"### {name} — used for {use}\n\nCoordinates are in grid units (x right, y down, origin top-left); scale by `size / {unit}`.\n\n"
               "| shape | x0 | y0 | x1 | y1 | corner radius | |\n|---|---|---|---|---|---|---|\n" + "\n".join(rows) +
               f"\n\n- **Pointer polygon** (fill, in order): {pts}. Round its convex corners by ≈{F(soft)} units (optional below 24 px).\n"
               f"- **Clearance:** remove device coverage within **{F(gap)} units** of the pointer polygon (distance from the polygon edge).\n"
               f"- **Lit screen** (Controlling only): rounded rect {lit}, minus the same clearance.\n")
print("\n".join(out))
