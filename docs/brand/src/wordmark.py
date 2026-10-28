"""The one·mk wordmark from real Geist SemiBold outlines, with the interpunct as an amber pixel."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wm
from geom import F

def rrect_d(x0, y0, x1, y1, r):
    return (f"M{F(x0+r)} {F(y0)} H{F(x1-r)} A{F(r)} {F(r)} 0 0 1 {F(x1)} {F(y0+r)} V{F(y1-r)} A{F(r)} {F(r)} 0 0 1 {F(x1-r)} {F(y1)} "
            f"H{F(x0+r)} A{F(r)} {F(r)} 0 0 1 {F(x0)} {F(y1-r)} V{F(y0+r)} A{F(r)} {F(r)} 0 0 1 {F(x0+r)} {F(y0)} Z")

XH = 534; ASC = 710    # Geist SemiBold x-height and k ascender (font units)
def wordmark(ink, sig, x0, base, size):
    """Returns (svg elements, right edge x). Left ink edge sits at x0."""
    sc = size / 1000.0; track = -14
    f = wm._f; parts = []
    cx = x0 - 39*sc                       # o's left side bearing -> ink starts at x0
    for ch in "one":
        g = f.cmap[ord(ch)]; parts.append(wm.path(f.contours(g), cx, base, sc)); cx += (f.adv(g)+track)*sc
    e_right = cx - (591+track)*sc + 551*sc
    gap = 118*sc; d = 150*sc; mid = base - XH*sc/2 - 8*sc
    dot = rrect_d(e_right+gap, mid-d/2, e_right+gap+d, mid+d/2, 6*sc)
    cx = e_right + gap + d + gap - 70*sc
    for ch in "mk":
        g = f.cmap[ord(ch)]; parts.append(wm.path(f.contours(g), cx, base, sc)); cx += (f.adv(g)+track)*sc
    right = cx - (628+track)*sc + 582*sc
    return f'<path fill="{ink}" d="{" ".join(parts)}"/><path fill="{sig}" d="{dot}"/>', right
