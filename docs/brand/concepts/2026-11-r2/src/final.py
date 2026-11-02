#!/usr/bin/env python3
"""Write the round-2 concept files: symbols (one colour, white, colour on dark, colour on light), lockups,
wordmark, and pixel-fitted 16/22 px cuts. Run with OUT=<concept folder>."""
import os, sys, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen2 as g
from gen2 import n, svg, write, lockup, wordmark_file, circle

INK, WHITE = "#111318", "#F4F5F8"
WV = "w2"


def lin(id_, x1, y1, x2, y2, stops):
    st = "".join(f'<stop offset="{o}" stop-color="{c}"/>' for o, c in stops)
    return (f'    <linearGradient id="{id_}" x1="{n(x1)}" y1="{n(y1)}" x2="{n(x2)}" y2="{n(y2)}" '
            f'gradientUnits="userSpaceOnUse">{st}</linearGradient>\n')


def paths(parts, evenodd=False, indent="    "):
    fr = ' fill-rule="evenodd"' if evenodd else ""
    return "".join(f'{indent}<path fill="{f}"{fr} d="{d}"/>\n' for d, f in parts)


# ---------------- concept definitions: (geometry, parts for each ground, defs) ----------------

def d1():
    L = g.lens("v2")
    defs_dark = (lin("p1w", 0, 36, 0, 220, [(0, "#FFA45C"), (0.55, "#FF5A6E"), (1, "#E2337E")]) +
                 lin("p1c", 0, 36, 0, 220, [(0, "#B49CFF"), (0.55, "#7F5BFF"), (1, "#5532E0")]) +
                 lin("p1h", 74, 0, 182, 0, [(0, "#FF6672"), (0.5, "#EC5FB6"), (1, "#8F66FF")]))
    colour = [(L["left"], "url(#p1w)"), (L["right"], "url(#p1c)"), (L["disc"], "url(#p1h)")]
    return dict(key="d1-lens", name="Lens", geo=L, d=L["d"], box=L["box"], colour_dark=colour,
                colour_light=colour, defs_dark=defs_dark, defs_light=defs_dark, mk_dark=WHITE, mk_light=INK)


def d2():
    P = g.pointer("v7")
    x0, y0, x1, y1 = P["box"]
    defs_dark = (lin("p2t", x0, y0, x0 + 70, y0 + 70, [(0, "#E8FF5A"), (1, "#2EE6A0")]) +
                 lin("p2b", 0, y0, 0, y1, [(0, "#FFFFFF"), (1, "#D5DAE3")]))
    defs_light = (lin("p2t", x0, y0, x0 + 70, y0 + 70, [(0, "#2FD98A"), (1, "#0E9F6E")]) +
                  lin("p2b", 0, y0, 0, y1, [(0, "#272B33"), (1, "#0E1014")]))
    colour = [(P["body"], "url(#p2b)"), (P["tip"], "url(#p2t)")]
    return dict(key="d2-pointer", name="Pointer", geo=P, d=P["d"], box=P["box"], colour_dark=colour, gap=0.26,
                colour_light=colour, defs_dark=defs_dark, defs_light=defs_light, mk_dark=WHITE, mk_light=INK)


def d3():
    V = g.converge_t("v6")
    end, x0 = V["end"], min(V["P"]["lefts"])

    def defs(mergec):
        return (lin("p3a", x0, 0, end + 2, 0, [(0, "#FFB224"), (1, mergec)]) +
                lin("p3b", x0, 0, end + 2, 0, [(0, "#FF4D6D"), (1, mergec)]) +
                lin("p3c", x0, 0, end + 2, 0, [(0, "#8B5CFF"), (1, mergec)]))
    colour_dark = [(V["block"], WHITE), (V["trails"][0], "url(#p3a)"), (V["trails"][1], "url(#p3b)"),
                   (V["trails"][2], "url(#p3c)")]
    colour_light = [(V["block"], INK)] + colour_dark[1:]
    return dict(key="d3-converge", name="Converge", geo=V, d=V["d"], box=V["box"], colour_dark=colour_dark,
                colour_light=colour_light, defs_dark=defs(WHITE), defs_light=defs(INK), mk_dark=WHITE, mk_light=INK)


SMALL = {
    "d1-lens": {
        16: ("M4 1 H7 V3.61 A4.5 4.5 0 0 0 7 12.39 V15 H4 A3 3 0 0 1 1 12 V4 A3 3 0 0 1 4 1 Z "
             "M9 1 H12 A3 3 0 0 1 15 4 V12 A3 3 0 0 1 12 15 H9 V12.39 A4.5 4.5 0 0 0 9 3.61 Z " + circle(8, 8, 3)),
        22: ("M5 1 H10 V4.58 A6.5 6.5 0 0 0 10 17.42 V21 H5 A4 4 0 0 1 1 17 V5 A4 4 0 0 1 5 1 Z "
             "M12 1 H17 A4 4 0 0 1 21 5 V17 A4 4 0 0 1 17 21 H12 V17.42 A6.5 6.5 0 0 0 12 4.58 Z " + circle(11, 11, 4.5)),
    },
    "d3-converge": {
        16: ("M8 1 A7 7 0 0 1 8 15 Z "
             "M6 1 H8.5 V5 H6 A2 2 0 0 1 6 1 Z "
             "M3 6 H8.5 V10 H3 A2 2 0 0 1 3 6 Z "
             "M5 11 H8.5 V15 H5 A2 2 0 0 1 5 11 Z"),
        22: ("M11 2 A9.5 9.5 0 0 1 11 21 Z "
             "M8.5 2 H11.5 V7 H8.5 A2.5 2.5 0 0 1 8.5 2 Z "
             "M3.5 9 H11.5 V14 H3.5 A2.5 2.5 0 0 1 3.5 9 Z "
             "M6.5 16 H11.5 V21 H6.5 A2.5 2.5 0 0 1 6.5 16 Z"),
    },
}


def pointer_small(size):
    """Pointer small cut: the small-size variant (wider seam, tighter corners) scaled onto the pixel grid."""
    P = g.pointer("s")
    x0, y0, x1, y1 = P["box"]
    pad = 1.0 if size == 16 else 1.5
    s = (size - 2 * pad) / max(x1 - x0, y1 - y0)
    tx = (size - (x1 - x0) * s) / 2 - x0 * s
    ty = (size - (y1 - y0) * s) / 2 - y0 * s
    return P["d"], f"translate({tx:.3f} {ty:.3f}) scale({s:.5f})"


def main():
    for C in (d1(), d2(), d3()):
        k, nm = C["key"], C["name"]
        eo = False
        write(f"{k}-symbol.svg", svg(256, 256, f"oneMK {nm} (one colour)", f'  <path fill="{INK}" d="{C["d"]}"/>\n'))
        write(f"{k}-symbol-white.svg", svg(256, 256, f"oneMK {nm} (white)", f'  <path fill="#FFFFFF" d="{C["d"]}"/>\n'))
        write(f"{k}-symbol-color.svg", svg(256, 256, f"oneMK {nm} (colour, for dark grounds)",
                                           paths(C["colour_dark"], indent="  "), C["defs_dark"]))
        write(f"{k}-symbol-color-light.svg", svg(256, 256, f"oneMK {nm} (colour, for light grounds)",
                                                 paths(C["colour_light"], indent="  "), C["defs_light"]))
        lockup(f"{k}-lockup.svg", f"oneMK {nm} lockup (one colour)", f'    <path fill="{INK}" d="{C["d"]}"/>\n',
               C["box"], WV, gap_k=C.get("gap", 0.30), word_fill=INK)
        lockup(f"{k}-lockup-white.svg", f"oneMK {nm} lockup (white)", f'    <path fill="#FFFFFF" d="{C["d"]}"/>\n',
               C["box"], WV, gap_k=C.get("gap", 0.30), word_fill="#FFFFFF")
        lockup(f"{k}-lockup-color.svg", f"oneMK {nm} lockup (colour, dark grounds)", paths(C["colour_dark"]),
               C["box"], WV, gap_k=C.get("gap", 0.30), word_fill=C["mk_dark"], defs=C["defs_dark"])
        lockup(f"{k}-lockup-color-light.svg", f"oneMK {nm} lockup (colour, light grounds)", paths(C["colour_light"]),
               C["box"], WV, gap_k=C.get("gap", 0.30), word_fill=C["mk_light"], defs=C["defs_light"])
        for size in (16, 22):
            if k == "d2-pointer":
                d, tf = pointer_small(size)
                body = f'  <g transform="{tf}"><path fill="{INK}" d="{d}"/></g>\n'
            else:
                body = f'  <path fill="{INK}" d="{SMALL[k][size]}"/>\n'
            write(f"{k}-small{size}.svg", svg(size, size, f"oneMK {nm}, {size} px cut", body))
    wordmark_file("wordmark.svg", WV, INK)
    wordmark_file("wordmark-white.svg", WV, "#FFFFFF")


if __name__ == "__main__":
    main()
