import os
from ttf import TTF, path
FONT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '../../../crates/app/fonts/Geist-SemiBold.ttf')
_f = TTF(FONT)
def bbox(ch):
    cs = _f.contours(_f.cmap[ord(ch)]); xs = [p[0] for c in cs for p in c]; ys = [p[1] for c in cs for p in c]
    return min(xs), min(ys), max(xs), max(ys)
XH = bbox('x')[3]          # x-height in font units
def word(text, x, base, size, fill, kern=None, track=0):
    """size = font-size in px; returns (svg elements, advance width). kern: dict index->extra units after char."""
    sc = size / _f.upm; els = []; cx = x
    for i, ch in enumerate(text):
        g = _f.cmap[ord(ch)]
        d = path(_f.contours(g), cx, base, sc)
        if d: els.append((ch, d))
        cx += (_f.adv(g) + track + (kern or {}).get(i, 0)) * sc
    return els, cx - x
