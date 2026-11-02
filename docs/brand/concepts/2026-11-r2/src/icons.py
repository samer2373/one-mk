#!/usr/bin/env python3
"""Build the three macOS-26-style app icons (1024 px PNG) with glass.py. Writes <OUT>/<key>-appicon-1024.png."""
import os, sys
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen2 as g
import glass as G

OUT = g.OUT


def place(box, size_k=0.56, cy=506):
    bx0, by0, bx1, by1 = box
    s = size_k * 824 / max(bx1 - bx0, by1 - by0)
    tx = 512 - (bx0 + bx1) / 2 * s
    ty = cy - (by0 + by1) / 2 * s
    return f"translate({tx:.2f} {ty:.2f}) scale({s:.5f})", s, tx, ty


def lin(id_, x1, y1, x2, y2, stops):
    st = "".join(f'<stop offset="{o}" stop-color="{c}"/>' for o, c in stops)
    return (f'<linearGradient id="{id_}" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" gradientUnits="userSpaceOnUse">'
            f'{st}</linearGradient>')


def layer(tf, paths, defs="", evenodd=False):
    """paths: list of (d, fill). Returns (colour RGB, mask)."""
    fr = ' fill-rule="evenodd"' if evenodd else ""
    body = f'<g transform="{tf}">' + "".join(f'<path{fr} d="{d}" fill="{f}"/>' for d, f in paths) + "</g>"
    mbody = f'<g transform="{tf}">' + "".join(f'<path{fr} d="{d}" fill="#fff"/>' for d, _ in paths) + "</g>"
    col = G.raster(body, defs)
    m = G.raster(mbody)[..., 3]
    return col[..., :3], m


def build(key, out=None):
    out = out or os.path.join(OUT, f"{key}-appicon-1024.png")
    if key == "d1-lens":
        L = g.lens("v2")
        # background layer: the tile is two screens (warm | cool) with a dark seam between them
        from gen2 import srect
        td = srect(100, 100, 924, 924, 176)
        seam_w = 18
        defs = (lin("warm", 0, 100, 0, 924, [(0, "#FFA45C"), (0.55, "#FF5A6E"), (1, "#E2337E")]) +
                lin("cool", 0, 100, 0, 924, [(0, "#B49CFF"), (0.55, "#7F5BFF"), (1, "#5532E0")]) +
                lin("heal", 392, 0, 632, 0, [(0, "#FF6672"), (0.5, "#EC5FB6"), (1, "#8F66FF")]) +
                lin("shade", 0, 100, 0, 924, [(0, "#ffffff"), (1, "#000000")]))
        clipdef = f'<clipPath id="tc"><path d="{td}"/></clipPath>'
        split = (f'<g clip-path="url(#tc)"><rect x="100" y="100" width="{512 - seam_w / 2 - 100}" height="824" fill="url(#warm)"/>'
                 f'<rect x="{512 + seam_w / 2}" y="100" width="{924 - 512 - seam_w / 2}" height="824" fill="url(#cool)"/>'
                 f'<rect x="{512 - seam_w / 2}" y="100" width="{seam_w}" height="824" fill="#0d0b14"/>'
                 f'<rect x="100" y="100" width="824" height="824" fill="url(#shade)" opacity=".10"/></g>')
        healed = (f'<g clip-path="url(#tc)"><rect x="100" y="100" width="412" height="824" fill="url(#warm)"/>'
                  f'<rect x="512" y="100" width="412" height="824" fill="url(#cool)"/>'
                  f'<rect x="392" y="100" width="240" height="824" fill="url(#heal)"/></g>')
        C = G.raster(split, defs + clipdef)[..., :3]
        H = G.raster(healed, defs + clipdef)[..., :3]
        tm = G.raster(f'<path d="{td}" fill="#fff"/>')[..., 3]
        R = 214
        disc = g.circle(512, 500, R)
        m2 = G.raster(f'<path d="{disc}" fill="#fff"/>')[..., 3]
        # lens: clear glass that magnifies the healed picture (the seam is gone inside it)
        C = G.glass_layer(C, m2, H, tint=0.10, frost=0, refr=60, rim=1.1, spec=0.38, magnify=(512, 500, 0.86),
                          chroma=0.03, top_light=0.30, depth=0.10, shadow=0.55, behind_src=H,
                          caustic=0.55, caustic_col=(1.0, 0.85, 0.95))
        glint = G.raster('<ellipse cx="430" cy="372" rx="92" ry="46" transform="rotate(-28 430 372)" fill="url(#gl)"/>',
                         '<radialGradient id="gl"><stop offset="0" stop-color="#fff" stop-opacity=".85"/>'
                         '<stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>')
        a = glint[..., 3:4] * m2[..., None]
        C = C * (1 - a) + 1.0 * a
    elif key == "d2-pointer":
        P = g.pointer("v7")
        tf, s, tx, ty = place(P["box"], 0.64, cy=506)
        C, tm, _ = G.tile("#1d3a2a", "#060b09", (380, 330, 600, "#c8ff4a", 0.34))
        defs = lin("w", 0, 40, 0, 220, [(0, "#FFFFFF"), (1, "#C9CFDA")])
        col, m = layer(tf, [(P["body"], "url(#w)")], defs)
        C = G.glass_layer(C, m, col, tint=0.9, frost=8, refr=14, rim=0.8, spec=0.25)
        defs2 = lin("tipg", 40, 40, 110, 110, [(0, "#E8FF5A"), (1, "#2EE6A0")])
        col2, m2 = layer(tf, [(P["tip"], "url(#tipg)")], defs2)
        C = G.glass_layer(C, m2, col2, tint=0.9, frost=6, refr=16, rim=1.0, spec=0.35, shadow=0.35, caustic=0.35, caustic_col=(0.9, 1.0, 0.6))
    elif key == "d3-converge":
        V = g.converge_t("v6")
        tf, s, tx, ty = place(V["box"], 0.62)
        C, tm, _ = G.tile("#221b2e", "#08070c", (512, 120, 760, "#ff7a59", 0.18))
        end = V["end"]
        x0 = min(V["P"]["lefts"])
        defs = (lin("a", x0, 0, end + 4, 0, [(0, "#FFB224"), (1, "#FFF6E6")]) +
                lin("b", x0, 0, end + 4, 0, [(0, "#FF4D6D"), (1, "#FFF0F3")]) +
                lin("c", x0, 0, end + 4, 0, [(0, "#8B5CFF"), (1, "#F3EEFF")]) +
                lin("w", 0, 40, 0, 216, [(0, "#FFFFFF"), (1, "#DADFEA")]))
        paths = [(V["trails"][0], "url(#a)"), (V["trails"][1], "url(#b)"), (V["trails"][2], "url(#c)"),
                 (V["block"], "url(#w)")]
        col, m = layer(tf, paths, defs)
        C = G.glass_layer(C, m, col, tint=0.9, frost=8, refr=14, rim=0.85, spec=0.28)
    G.finish(C, tm, out)
    return out


if __name__ == "__main__":
    for k in sys.argv[1:] or ["d1-lens", "d2-pointer", "d3-converge"]:
        print(build(k))
