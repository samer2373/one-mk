"""one·mk Crossing mark: a laptop and a monitor, the pointer on the seam between them.
Devices are built from convex pieces, the pointer's clearance is subtracted (real holes),
and the result is merged back into clean outlines."""
from geom import *

GROUND, PANEL, RAISED, LINE, INK, INK2, AMBER, SIGNAL_INK = "#0F1216", "#13171C", "#171B21", "#252B34", "#EDEFF2", "#9DA6B1", "#FFB547", "#1E1505"

def device_pieces(g):
    seg = g.get("seg", 8)
    P = []
    x0, y0, x1, y1, ri = g["mon"]
    if g.get("solid"):
        P.append(rrect(x0, y0, x1, y1, ri + g["t"], seg))
    else:
        P += ring_pieces(x0, y0, x1, y1, g["t"], ri, seg)
    nx0, ny0, nx1, ny1 = g["neck"]; P.append(rrect(nx0, ny0, nx1, ny1, 0))
    fx0, fy0, fx1, fy1, fr = g["foot"]; P.append(rrect(fx0, fy0, fx1, fy1, fr, seg))
    if g.get("lap"):
        x0, y0, x1, y1, ri = g["lap"]
        if g.get("solid"):
            P.append(rrect(x0, y0, x1, y1, ri + g["t"], seg))
        else:
            P += ring_pieces(x0, y0, x1, y1, g["t"], ri, seg)
        bx0, by0, bx1, by1, br = g["base"]; P.append(rrect(bx0, by0, bx1, by1, br, seg))
    return P

def knock(g):
    tx, ty = g["tip"]
    return cursor_knock(tx, ty, g["h"], g["soft"], g["gap"])

def build(g, ink, sig, lit=None, cursor_on=True, cursor_col=None, outline=None, ink_opacity=None):
    """SVG elements. lit = fill for the monitor screen (Controlling). outline = (colour, width) ring around
    the pointer (light trays). cursor_col overrides the pointer colour (e.g. ink for Idle)."""
    dx, dy = g["shift"]; tx, ty = g["tip"]
    K = knock(g) if cursor_on else []
    pieces = minus(device_pieces(g), K)
    if lit:
        x0, y0, x1, y1, ri = g["mon"]; t = g["t"]; gl = g.get("litgap", t * 0.6)
        pieces_lit = minus([rrect(x0+t+gl, y0+t+gl, x1-t-gl, y1-t-gl, max(ri-gl*0.6, 0.6), g.get("seg", 8))], K)
    op = f' fill-opacity="{ink_opacity}"' if ink_opacity else ""
    out = f'<path fill="{ink}"{op} d="{d_of(union_abutting(pieces), dx, dy)}"/>'
    if lit:
        out += f'<path fill="{lit}" d="{d_of(union_abutting(pieces_lit), dx, dy)}"/>'
    if cursor_on:
        if outline:
            col, w = outline
            ring = cursor_knock(tx, ty, g["h"], g["soft"], w)
            out += f'<path fill="{col}" d="{d_of(ring, dx, dy)}"/>'
        cur = cursor_outline(tx, ty, g["h"], g["soft"] * 2.2, g["soft"] * 0.8)
        out += f'<path fill="{cursor_col or sig}" d="{d_of([cur], dx, dy)}"/>'
    return out

# ---- master, 256 grid ----
M = dict(t=12, mon=(120, 56, 236, 132, 4), neck=(171, 132, 185, 154), foot=(158, 154, 198, 166, 6),
         lap=(28, 96, 104, 148, 3), base=(16, 154, 116, 166, 6),
         tip=(112, 76), h=80, soft=1.6, gap=6, shift=(2, 15), seg=10)
# ---- small drawings on pixel grids (units = px at 1x) ----
# 16 px: two computers kept. 1 px strokes on whole pixels, the pointer's tip in the 1 px seam.
S16 = dict(t=1, mon=(6, 2, 16, 8, 0.5), neck=(11, 8, 12, 10), foot=(9, 10, 14, 11, 0.5),
           lap=(0, 5, 5, 9, 0.4), base=(0, 10, 5.5, 11, 0.5),
           tip=(5.95, 2.4), h=7.2, soft=0.1, gap=0.85, shift=(0, 1), seg=3)
# 16 px fallback: one monitor + pointer.
S16_ONE = dict(t=1.5, mon=(4, 1.5, 15.5, 9.5, 0.5), neck=(9, 9.5, 10.5, 11.5), foot=(6.5, 11.5, 13, 13, 0.75),
               tip=(1.5, 4), h=10.5, soft=0.12, gap=1.0, shift=(0, 0.5), seg=4)
# 22-24 px: two computers, 1.5 px strokes.
S24 = dict(t=1.5, mon=(10, 3, 24, 12, 0.75), neck=(16.25, 12, 17.75, 14.5), foot=(14.5, 14.5, 19.5, 16, 0.75),
           lap=(1, 7.5, 8.5, 13, 0.5), base=(0, 14.5, 9.5, 16, 0.75),
           tip=(9.4, 4.2), h=10.4, soft=0.12, gap=1.0, shift=(0, 2), seg=4)
# 22 px (GNOME / AppIndicator): 2 px monitor frame, whole-pixel edges.
S22 = dict(t=2, mon=(10, 2, 22, 11, 0.6), neck=(15, 11, 17, 13), foot=(13.5, 13, 18.5, 15, 1),
           lap=(1, 6, 8, 12, 0.5), base=(0, 13, 9, 15, 1),
           tip=(9.05, 3.4), h=9.6, soft=0.12, gap=1.0, shift=(0, 2), seg=4)
