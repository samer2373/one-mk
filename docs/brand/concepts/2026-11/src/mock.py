#!/usr/bin/env python3
"""macOS 26-style layered app-icon mocks (1024 px): a rounded-square tile + the mark split into 1-3 flat
layers, each rendered as glass (soft shadow, translucent body, light top, specular rim).
This is a presentation mock (uses gradients and filters); the masters stay flat."""
import importlib.util, os, subprocess, sys

spec = importlib.util.spec_from_file_location("gen", os.path.join(os.path.dirname(os.path.abspath(__file__)), "gen.py"))
g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
OUT = g.OUT
R = "/home/samer/.claude/skills/logo-design/scripts/render_png.py"

TILE = (100, 100, 924, 924)


def tile_path():
    return g.srect(*TILE, 176)


def mock(name, layers, box, tile_top, tile_bot, size_k=0.60, dx=0.0, glass_white="#FFFFFF"):
    bx0, by0, bx1, by1 = box
    s = size_k * 824 / max(bx1 - bx0, by1 - by0)
    cx, cy = (bx0 + bx1) / 2, (by0 + by1) / 2
    tx, ty = 512 - cx * s + dx, 512 - cy * s - 6  # optical: a touch above centre
    sw = 1 / s
    defs = [f'''<linearGradient id="tile" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="{tile_top}"/><stop offset="1" stop-color="{tile_bot}"/></linearGradient>
    <linearGradient id="tileRim" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity=".28"/><stop offset=".5" stop-color="#fff" stop-opacity=".04"/>
      <stop offset="1" stop-color="#fff" stop-opacity=".12"/></linearGradient>
    <radialGradient id="tileGlow" cx="512" cy="180" r="700" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity=".10"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>
    <filter id="drop" x="-30%" y="-30%" width="160%" height="160%">
      <feDropShadow dx="0" dy="{n(14 * sw)}" stdDeviation="{n(18 * sw)}" flood-color="#000" flood-opacity=".38"/></filter>
    <filter id="tileShadow" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="10" stdDeviation="14" flood-color="#000" flood-opacity=".28"/></filter>''']
    body = []
    for i, L in enumerate(layers):
        c0, c1, op = L["top"], L["bot"], L.get("opacity", 0.92)
        defs.append(f'''<linearGradient id="b{i}" x1="0" y1="{n(by0)}" x2="0" y2="{n(by1)}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="{c0}"/><stop offset="1" stop-color="{c1}"/></linearGradient>
    <linearGradient id="r{i}" x1="0" y1="{n(by0)}" x2="0" y2="{n(by1)}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity=".85"/><stop offset=".45" stop-color="#fff" stop-opacity=".18"/>
      <stop offset="1" stop-color="#fff" stop-opacity=".45"/></linearGradient>
    <linearGradient id="h{i}" x1="0" y1="{n(by0)}" x2="0" y2="{n(by1)}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity=".38"/><stop offset=".42" stop-color="#fff" stop-opacity="0"/></linearGradient>''')
        fr = ' fill-rule="evenodd"' if L.get("evenodd") else ""
        d = L["d"]
        body.append(f'''    <g filter="url(#drop)"><path{fr} d="{d}" fill="url(#b{i})" fill-opacity="{op}"/></g>
    <path{fr} d="{d}" fill="url(#h{i})"/>
    <path{fr} d="{d}" fill="none" stroke="url(#r{i})" stroke-width="{n(2.2 * sw)}"/>''')
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <title>{name} — app icon mock (macOS 26 style, layered glass)</title>
  <defs>
    {chr(10).join(defs)}
  </defs>
  <path d="{tile_path()}" fill="url(#tile)" filter="url(#tileShadow)"/>
  <path d="{tile_path()}" fill="url(#tileGlow)"/>
  <path d="{tile_path()}" fill="none" stroke="url(#tileRim)" stroke-width="3"/>
  <g transform="translate({n(tx)} {n(ty)}) scale({s:.5f})">
{chr(10).join(body)}
  </g>
</svg>
'''
    p = os.path.join(OUT, f"{name}-appicon-mock.svg")
    open(p, "w").write(svg)
    subprocess.run(["python3", R, p, "-o", os.path.join(OUT, f"{name}-appicon-1024.png"), "--size", "1024"],
                   check=True, stdout=subprocess.DEVNULL)


n = g.n
e, b, c = g.edge("v12"), g.seam("v4"), g.shift("v5")

# A Edge: dark graphite tile; layer 1 = frosted white pane (with the hole); layer 2 = lime half-dot.
mock("a-edge", [
    dict(d=f'{e["pane"]} {e["hole"]}', evenodd=True, top="#FFFFFF", bot="#D9DEE6", opacity=0.94),
    dict(d=e["bump"], top="#E2FF8A", bot="#A9DA2C", opacity=0.97),
], e["box"], "#2A2E36", "#0B0D11", dx=-4)

# B Seam: light porcelain tile; layer 1 = ink brackets; layer 2 = violet dot.
mock("b-seam", [
    dict(d=b["frame"], top="#262A31", bot="#0E1014", opacity=0.96),
    dict(d=b["dot"], top="#A79BFF", bot="#6352F5", opacity=0.97),
], b["box"], "#FFFFFF", "#DCE0E8")

# C Shift: deep ink tile; layer 1 = white half; layer 2 = magenta half.
mock("c-shift", [
    dict(d=c["left"], top="#FFFFFF", bot="#D5DAE3", opacity=0.94),
    dict(d=c["right"], top="#FF85C0", bot="#E02F86", opacity=0.97),
], c["box"], "#2B2733", "#0C0B10")
print("ok")
