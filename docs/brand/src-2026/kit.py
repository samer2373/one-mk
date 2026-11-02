#!/usr/bin/env python3
"""Build the oneMK brand kit (the Pointer mark, Nov 2026) into docs/brand/.

    python3 docs/brand/src-2026/kit.py                 # everything
    python3 docs/brand/src-2026/kit.py logo png app tray web

Needs: Python 3 with numpy and Pillow (the glass app-icon render), and the logo-design skill's
render_png.py (headless Chrome) to rasterise SVG. The wordmark outlines are stored in
wordmark-outlines.json (Geist, SIL OFL 1.1), so fontTools is only needed to regenerate that file
(see make_outlines.py). .icns and .ico are packed here; iconutil is not needed."""
import json, math, os, re, shutil, struct, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import mark as M
from mark import n

BRAND = os.path.dirname(HERE)
RENDER = os.path.expanduser("~/.claude/skills/logo-design/scripts/render_png.py")
BLACK, WHITE = "#000000", "#FFFFFF"


# ================================================================= basics

def svg(w, h, body, title, defs=""):
    d = f"  <defs>\n{defs}  </defs>\n" if defs else ""
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n(w)} {n(h)}" width="{n(w)}" height="{n(h)}" '
            f'role="img" aria-labelledby="title">\n  <title id="title">{title}</title>\n{d}{body}</svg>\n')


def write(rel, text):
    p = os.path.join(BRAND, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, "w").write(text)
    return p


def render(src, out, w, h=None, pad=0):
    os.makedirs(os.path.dirname(out), exist_ok=True)
    cmd = ["python3", RENDER, src, "-o", out, "--width", str(w), "--height", str(h or w)]
    if pad:
        cmd += ["--padding", str(pad)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        print(r.stdout, r.stderr)
        raise SystemExit(f"render failed: {src}")


# ================================================================= wordmark "oneMK"
WM = json.load(open(os.path.join(HERE, "wordmark-outlines.json")))
G = WM["glyphs"]
TRACK = -12          # font units, at each part's own size
E_TO_M = 92          # ink gap e -> M, in units of the "one" size
CAPS = 0.88          # MK drawn as small capitals: 88 % height, weight 660 so stems match "one" at 600


def place(d, s, x, base):
    """Font-unit path (y up) -> output units: x' = x + s*u, y' = base - s*v."""
    out, i = [], 0
    toks = re.findall(r"[A-Za-z]|-?\d*\.?\d+", d)
    cmd = None
    while i < len(toks):
        t = toks[i]
        if t.isalpha():
            cmd = t; out.append(t); i += 1
            if t in "Zz":
                continue
            continue
        if cmd == "H":
            out.append(n(x + s * float(t))); i += 1
        elif cmd == "V":
            out.append(n(base - s * float(t))); i += 1
        else:
            out.append(n(x + s * float(t))); out.append(n(base - s * float(toks[i + 1]))); i += 2
    return " ".join(out).replace(" Z", "Z").replace("Z ", "Z")


def wordmark(x, base, xh):
    """Returns (path d, right ink edge, cap height in output units)."""
    s = xh / WM["x_height"]
    sc = s * CAPS
    parts, cx = [], x
    for ch in "one":
        parts.append(place(G[ch]["d"], s, cx, base))
        last = cx
        cx += (G[ch]["advance"] + TRACK) * s
    e_right = last + G["e"]["bounds"][2] * s
    mx = e_right + E_TO_M * s - G["M"]["bounds"][0] * sc
    parts.append(place(G["M"]["d"], sc, mx, base))
    kx = mx + (G["M"]["advance"] + TRACK) * sc
    parts.append(place(G["K"]["d"], sc, kx, base))
    right = kx + G["K"]["bounds"][2] * sc
    left = x + G["o"]["bounds"][0] * s
    return " ".join(parts), left, right, WM["cap_height"] * sc


# ================================================================= the mark in each colour scheme
# scheme: (body, tip, ring-on-tip or None, wordmark)
SCHEMES = {
    "color": (M.GROUND, M.ACCENT, M.GROUND, M.GROUND),      # light grounds: tip lit, with an ink edge
    "reversed": (M.INK, M.ACCENT, None, M.INK),             # dark grounds (primary)
    "black": (BLACK, BLACK, None, BLACK),
    "white": (WHITE, WHITE, None, WHITE),
}
RING = {256: 9.0, 16: 0.9, 22: 1.15, 24: 1.25}


def mark_body(P, scheme, grid=256):
    body_c, tip_c, ring_c, _ = SCHEMES[scheme]
    ptr = M.pointer(P)
    tip_d, body_d = M.paths(ptr)
    if body_c == tip_c:
        return f'    <path fill="{body_c}" d="{body_d} {tip_d}"/>\n', ptr
    out = f'    <path fill="{body_c}" d="{body_d}"/>\n'
    if ring_c:
        out += f'    <path fill="{ring_c}" d="{tip_d}"/>\n'
        out += f'    <path fill="{tip_c}" d="{M.inset(ptr["tip"], ptr["tip_r"], RING[grid])}"/>\n'
    else:
        out += f'    <path fill="{tip_c}" d="{tip_d}"/>\n'
    return out, ptr


def symbol_files():
    for k in SCHEMES:
        body, _ = mark_body(M.MASTER, k)
        write(f"logo/onemk-symbol-{k}.svg", svg(256, 256, body, "oneMK symbol"))
        for g in (16, 22, 24):
            body, _ = mark_body(M.SMALL[g], k, g)
            write(f"logo/onemk-symbol-small{g}-{k}.svg", svg(g, g, body, f"oneMK symbol, {g} px drawing"))


def wordmark_files():
    for k, (_, _, _, wc) in SCHEMES.items():
        xh = 100
        cap = WM["cap_height"] * xh / WM["x_height"] * CAPS
        base = cap + 0
        d, left, right, _ = wordmark(0, base, xh)
        # tight box: from the o's ink to the K's ink, from cap height to the o's overshoot below the baseline
        bottom = base + 12 * xh / WM["x_height"]
        body = f'  <path fill="{wc}" d="{d}" transform="translate({n(-left)} 0)"/>\n'
        write(f"logo/onemk-wordmark-{k}.svg", svg(right - left, bottom, body, "oneMK"))


def group(P, scheme, x, y, h):
    """The symbol fitted so its ink is h tall, top-left of the ink at (x, y). Returns (svg, width)."""
    ptr = M.pointer(P)
    x0, y0, x1, y1 = ptr["box"]
    s = h / (y1 - y0)
    body, _ = mark_body(P, scheme)
    return (f'  <g transform="translate({n(x - x0 * s)} {n(y - y0 * s)}) scale({s:.5f})">\n{body}  </g>\n',
            (x1 - x0) * s)


def lockup_files():
    xh = 100
    cap = WM["cap_height"] * xh / WM["x_height"] * CAPS          # 116.97
    for k, (_, _, _, wc) in SCHEMES.items():
        # horizontal: the symbol's ink is 2.15 cap heights tall, centred on the cap band; gap = 0.62 x symbol h
        sh = cap * 2.45
        top = 0
        cy = sh / 2
        g, sw = group(M.MASTER, k, 0, top, sh)
        base = cy + cap / 2
        d, left, right, _ = wordmark(0, base, xh)
        gap = sh * 0.17
        tx = sw + gap - left
        body = g + f'  <path fill="{wc}" d="{d}" transform="translate({n(tx)} 0)"/>\n'
        write(f"logo/onemk-lockup-horizontal-{k}.svg", svg(tx + right, sh, body, "oneMK"))
        # stacked: symbol centred above the wordmark
        d2, l2, r2, _ = wordmark(0, 0, xh)
        ww = r2 - l2
        sh2 = cap * 2.6
        ptr = M.pointer(M.MASTER); bx0, by0, bx1, by1 = ptr["box"]
        sw2 = (bx1 - bx0) * sh2 / (by1 - by0)
        W = max(ww, sw2)
        g2, _ = group(M.MASTER, k, (W - sw2) / 2, 0, sh2)
        base2 = sh2 + cap * 0.62 + cap
        body2 = g2 + f'  <path fill="{wc}" d="{d2}" transform="translate({n((W - ww) / 2 - l2)} {n(base2)})"/>\n'
        write(f"logo/onemk-lockup-stacked-{k}.svg", svg(W, base2 + 12 * xh / WM["x_height"], body2, "oneMK"))


def logo_pngs():
    L = os.path.join(BRAND, "logo")
    for k in SCHEMES:
        render(f"{L}/onemk-symbol-{k}.svg", f"{L}/png/onemk-symbol-{k}-1024.png", 1024)
        render(f"{L}/onemk-symbol-{k}.svg", f"{L}/png/onemk-symbol-{k}-256.png", 256)
        render(f"{L}/onemk-symbol-{k}.svg", f"{L}/png/onemk-symbol-{k}-32.png", 32)
        render(f"{L}/onemk-symbol-small24-{k}.svg", f"{L}/png/onemk-symbol-{k}-24.png", 24)
        render(f"{L}/onemk-symbol-small16-{k}.svg", f"{L}/png/onemk-symbol-{k}-16.png", 16)
        for kind, (w, h) in (("wordmark", (1200, 300)), ("lockup-horizontal", (1600, 300)), ("lockup-stacked", (900, 640))):
            render(f"{L}/onemk-{kind}-{k}.svg", f"{L}/png/onemk-{kind}-{k}.png", w, h, pad=0.06)


# ================================================================= app icons

def tile_path(shape, S=1024):
    if shape == "mac":                      # macOS: 824 tile on the 1024 grid, continuous corners
        return M_srect(100, 100, 924, 924, 176), (100, 924)
    if shape == "bleed":
        return f"M0 0 H{S} V{S} H0 Z", (0, S)
    return M_srect(32, 32, 992, 992, 150), (32, 992)   # Windows / Linux / web: rounded square


def M_srect(x0, y0, x1, y1, r, a_k=1.6, d_k=0.25):
    a, d = a_k * r, d_k * r
    return " ".join([
        f"M{n(x0 + a)} {n(y0)} H{n(x1 - a)}", f"C{n(x1 - d)} {n(y0)} {n(x1)} {n(y0 + d)} {n(x1)} {n(y0 + a)}",
        f"V{n(y1 - a)}", f"C{n(x1)} {n(y1 - d)} {n(x1 - d)} {n(y1)} {n(x1 - a)} {n(y1)}",
        f"H{n(x0 + a)}", f"C{n(x0 + d)} {n(y1)} {n(x0)} {n(y1 - d)} {n(x0)} {n(y1 - a)}",
        f"V{n(y0 + a)}", f"C{n(x0)} {n(y0 + d)} {n(x0 + d)} {n(y0)} {n(x0 + a)} {n(y0)} Z"])


# Optical placement in the tile: the notch takes mass out of the lower right, so the pointer's area centroid
# sits up-left of its bounding-box centre. Centre on a point 80 % of the way from the box centre to the
# centroid, then lift it 1 % of the tile (the optical centre sits a touch above the geometric one).
MASS_W, LIFT = 0.80, 0.010
SCALE = {"master": 0.645, "mid": 0.70, "tiny": 0.85, "maskable": 0.515}   # share of the tile (was .60/.66/.80/.48)


def mass_anchor(ptr):
    def cent(p):
        A = cx = cy = 0.0
        for i in range(len(p)):
            (x0, y0), (x1, y1) = p[i], p[(i + 1) % len(p)]
            c = x0 * y1 - x1 * y0
            A += c; cx += (x0 + x1) * c; cy += (y0 + y1) * c
        A /= 2
        return A, cx / (6 * A), cy / (6 * A)
    a1, x1_, y1_ = cent(ptr["body"]); a2, x2_, y2_ = cent(ptr["tip"])
    gx, gy = (a1 * x1_ + a2 * x2_) / (a1 + a2), (a1 * y1_ + a2 * y2_) / (a1 + a2)
    bx0, by0, bx1, by1 = ptr["box"]
    bx, by = (bx0 + bx1) / 2, (by0 + by1) / 2
    return bx + MASS_W * (gx - bx), by + MASS_W * (gy - by)


def placement(ptr, s, share):
    x0, y0, x1, y1 = ptr["box"]
    k = s * share / max(x1 - x0, y1 - y0)
    ax, ay = mass_anchor(ptr)
    return k, 512 - ax * k, 512 - LIFT * s - ay * k


def icon_svg(shape, level, title="oneMK app icon", mark_k=None):
    """Layered app icon as SVG (no filters): tile, light, body, lit tip.
    level: master (>=128 px), mid (48-64 px), tiny (16-32 px, pixel drawing, flat)."""
    d, (a, b) = tile_path(shape)
    s = b - a
    if level == "tiny":
        P, grid, mark_k = M.SMALL[16], 16, (mark_k or SCALE["tiny"])
    else:
        P, grid, mark_k = M.MASTER, 256, (mark_k or SCALE[level])
    ptr = M.pointer(P)
    tip_d, body_d = M.paths(ptr)
    x0, y0, x1, y1 = ptr["box"]
    k, tx, ty = placement(ptr, s, mark_k)
    defs = (f'    <linearGradient id="tile" x1="0" y1="{a}" x2="0" y2="{b}" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="{M.TILE_TOP}"/><stop offset="1" stop-color="{M.TILE_BOTTOM}"/></linearGradient>\n'
            f'    <radialGradient id="glow" cx="{n(tx + (x0 + (x1 - x0) * 0.25) * k)}" cy="{n(ty + (y0 + (y1 - y0) * 0.25) * k)}" r="{n(s * 0.62)}" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="#C8FF4A" stop-opacity=".30"/><stop offset="1" stop-color="#C8FF4A" stop-opacity="0"/></radialGradient>\n'
            f'    <linearGradient id="rim" x1="0" y1="{a}" x2="0" y2="{b}" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="#FFFFFF" stop-opacity=".30"/><stop offset=".5" stop-color="#FFFFFF" stop-opacity=".04"/>'
            f'<stop offset="1" stop-color="#FFFFFF" stop-opacity=".12"/></linearGradient>\n'
            f'    <linearGradient id="body" x1="0" y1="{n(y0)}" x2="0" y2="{n(y1)}" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#CDD3DD"/></linearGradient>\n'
            f'    <linearGradient id="tip" x1="{n(x0)}" y1="{n(y0)}" x2="{n(x0 + (x1 - x0) * 0.4)}" y2="{n(y0 + (y1 - y0) * 0.4)}" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="#E4FF6E"/><stop offset="1" stop-color="#AEE62A"/></linearGradient>\n')
    flat = level == "tiny"
    body = f'  <path d="{d}" fill="url(#tile)"/>\n'
    if shape != "bleed":
        body += f'  <path d="{d}" fill="url(#glow)"/>\n'
        if not flat:
            body += f'  <path d="{d}" fill="none" stroke="url(#rim)" stroke-width="{n(s / 160)}"/>\n'
    else:
        body += f'  <path d="{d}" fill="url(#glow)"/>\n'
    bf = "#F4F6F9" if flat else "url(#body)"
    tf = M.ACCENT if flat else "url(#tip)"
    body += (f'  <g transform="translate({n(tx)} {n(ty)}) scale({k:.5f})">\n'
             f'    <path fill="{bf}" d="{body_d}"/>\n    <path fill="{tf}" d="{tip_d}"/>\n  </g>\n')
    return svg(1024, 1024, body, title, defs)


def glass_master(shape, out):
    """The 1024 px layered-glass render (numpy compositor) used for 128 px and up."""
    import numpy as np
    import glass as GL
    d, (a, b) = tile_path(shape)
    s = b - a
    ptr = M.pointer(M.MASTER)
    tip_d, body_d = M.paths(ptr)
    x0, y0, x1, y1 = ptr["box"]
    k, tx, ty = placement(ptr, s, SCALE["master"])
    gx, gy = tx + (x0 + (x1 - x0) * 0.25) * k, ty + (y0 + (y1 - y0) * 0.25) * k
    tdefs = (f'<linearGradient id="t" x1="0" y1="{a}" x2="0" y2="{b}" gradientUnits="userSpaceOnUse">'
             f'<stop offset="0" stop-color="{M.TILE_TOP}"/><stop offset="1" stop-color="{M.TILE_BOTTOM}"/></linearGradient>'
             f'<radialGradient id="g" cx="{gx:.1f}" cy="{gy:.1f}" r="{s * 0.62:.1f}" gradientUnits="userSpaceOnUse">'
             f'<stop offset="0" stop-color="#C8FF4A" stop-opacity=".34"/><stop offset="1" stop-color="#C8FF4A" stop-opacity="0"/></radialGradient>')
    C = GL.raster(f'<path d="{d}" fill="url(#t)"/><path d="{d}" fill="url(#g)"/>', tdefs)[..., :3]
    tm = GL.raster(f'<path d="{d}" fill="#fff"/>')[..., 3]
    tf = f"translate({tx:.2f} {ty:.2f}) scale({k:.5f})"
    def layer(path, fill, defs=""):
        col = GL.raster(f'<g transform="{tf}"><path d="{path}" fill="{fill}"/></g>', defs)[..., :3]
        m = GL.raster(f'<g transform="{tf}"><path d="{path}" fill="#fff"/></g>')[..., 3]
        return col, m
    col, m = layer(body_d, "url(#w)", f'<linearGradient id="w" x1="0" y1="{y0:.1f}" x2="0" y2="{y1:.1f}" gradientUnits="userSpaceOnUse">'
                   f'<stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#C9CFDA"/></linearGradient>')
    C = GL.glass_layer(C, m, col, tint=0.9, frost=8, refr=14, rim=0.8, spec=0.25)
    col, m = layer(tip_d, "url(#l)", f'<linearGradient id="l" x1="{x0:.1f}" y1="{y0:.1f}" x2="{x0 + (x1 - x0) * .4:.1f}" y2="{y0 + (y1 - y0) * .4:.1f}" gradientUnits="userSpaceOnUse">'
                   f'<stop offset="0" stop-color="#E8FF6E"/><stop offset="1" stop-color="#A8E028"/></linearGradient>')
    C = GL.glass_layer(C, m, col, tint=0.9, frost=6, refr=16, rim=1.0, spec=0.35, shadow=0.30,
                       caustic=0.30, caustic_col=(0.9, 1.0, 0.6))
    if shape == "bleed":
        from PIL import Image
        Image.fromarray((np.clip(C, 0, 1) * 255).astype("uint8"), "RGB").save(out)
    else:
        GL.finish(C, tm, out, shadow=(shape == "mac"))


def downscale(src, out, px):
    from PIL import Image
    im = Image.open(src).convert("RGBA")
    # premultiply-safe downscale: resize in RGBa
    im = im.convert("RGBa").resize((px, px), Image.LANCZOS).convert("RGBA")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    im.save(out)


def pack_ico(pngs, out):
    from PIL import Image
    data = [open(p, "rb").read() for p in pngs]
    sizes = [Image.open(p).size for p in pngs]
    hdr = struct.pack("<HHH", 0, 1, len(data)); off = 6 + 16 * len(data); ents = b""
    for (w, h), d in zip(sizes, data):
        ents += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(d), off); off += len(d)
    open(out, "wb").write(hdr + ents + b"".join(data))


def pack_icns(entries, out):
    body = b""
    for code, p in entries:
        d = open(p, "rb").read()
        body += code.encode() + struct.pack(">I", len(d) + 8) + d
    open(out, "wb").write(b"icns" + struct.pack(">I", len(body) + 8) + body)


def app_icons():
    tmp = os.path.join(HERE, ".build"); os.makedirs(tmp, exist_ok=True)
    for plat, shape in (("macos", "mac"), ("windows-linux", "square")):
        for level in ("master", "mid", "tiny"):
            write(f"app-icon/{plat}/onemk-appicon-{level}.svg", icon_svg(shape, level))
        glass_master(shape, os.path.join(tmp, f"glass-{shape}.png"))

    def make(plat, shape, px, out):
        if px <= 32:
            render(os.path.join(BRAND, f"app-icon/{plat}/onemk-appicon-tiny.svg"), out, px)
        elif px <= 64:
            render(os.path.join(BRAND, f"app-icon/{plat}/onemk-appicon-mid.svg"), out, px)
        else:
            downscale(os.path.join(tmp, f"glass-{shape}.png"), out, px)

    iconset = os.path.join(BRAND, "app-icon/macos/onemk.iconset")
    for pt in (16, 32, 128, 256, 512):
        for sc in (1, 2):
            make("macos", "mac", pt * sc, os.path.join(iconset, f"icon_{pt}x{pt}{'@2x' if sc == 2 else ''}.png"))
    for px in (64, 1024):
        make("macos", "mac", px, os.path.join(BRAND, f"app-icon/macos/png/onemk-{px}.png"))
    I = lambda nm: os.path.join(iconset, nm)
    pack_icns([("icp4", I("icon_16x16.png")), ("ic11", I("icon_16x16@2x.png")), ("icp5", I("icon_32x32.png")),
               ("ic12", I("icon_32x32@2x.png")), ("ic07", I("icon_128x128.png")), ("ic13", I("icon_128x128@2x.png")),
               ("ic08", I("icon_256x256.png")), ("ic14", I("icon_256x256@2x.png")), ("ic09", I("icon_512x512.png")),
               ("ic10", I("icon_512x512@2x.png"))], os.path.join(BRAND, "app-icon/macos/onemk.icns"))
    wdir = os.path.join(BRAND, "app-icon/windows-linux")
    pngs = []
    for px in (16, 20, 24, 32, 40, 48, 64, 256):
        p = os.path.join(wdir, f"ico-png/onemk-{px}.png"); make("windows-linux", "square", px, p); pngs.append(p)
    pack_ico(pngs, os.path.join(wdir, "onemk.ico"))
    for px in (16, 22, 24, 32, 48, 64, 128, 256, 512):
        make("windows-linux", "square", px, os.path.join(wdir, f"hicolor/{px}x{px}/apps/one-mk.png"))
    shutil.copy(os.path.join(wdir, "onemk-appicon-master.svg"), os.path.join(wdir, "hicolor/scalable/apps/one-mk.svg"))


# ================================================================= tray
STATES = ("off", "idle", "connected", "controlling")
OUTLINE = {16: 1.25, 22: 1.6, 24: 1.7, 256: 17.0}


def tray_body(state, grid, theme):
    P = M.MASTER if grid == 256 else M.SMALL[grid]
    ptr = M.pointer(P)
    tip_d, body_d = M.paths(ptr)
    w = OUTLINE[grid]
    if theme == "dark":
        ink, acc, ring = M.INK, M.ACCENT, None
    elif theme == "light":
        # Volt has no contrast on a light bar; at tray sizes an ink edge would eat it, so the lit tip
        # uses the deep accent instead (5.1:1 on white).
        ink, acc, ring = M.LIGHT_TRAY_INK, M.ACCENT_DEEP, None
    else:
        ink, acc, ring = BLACK, BLACK, None
    hb = M.hollow(ptr["body"], ptr["body_r"], w)
    ht = M.hollow(ptr["tip"], ptr["tip_r"], w * 0.9)
    def lit_tip():
        if ring:
            return (f'  <path fill="{ring}" d="{tip_d}"/>\n'
                    f'  <path fill="{acc}" d="{M.inset(ptr["tip"], ptr["tip_r"], RING[grid])}"/>\n')
        return f'  <path fill="{acc}" d="{tip_d}"/>\n'
    if state == "off":        # hollow and dim: nothing is running
        return f'  <path fill="{ink}" fill-opacity=".55" fill-rule="evenodd" d="{hb} {ht}"/>\n'
    if state == "idle":       # the whole pointer, tip unlit (dim): running, nobody to cross to yet
        return f'  <path fill="{ink}" d="{body_d}"/>\n  <path fill="{ink}" fill-opacity=".35" d="{tip_d}"/>\n'
    if state == "connected":  # tip lit: another computer is there
        return f'  <path fill="{ink}" d="{body_d}"/>\n' + lit_tip()
    # controlling: the pointer has crossed; only its lit tip is solid, the body here is hollow
    return f'  <path fill="{ink}" fill-rule="evenodd" d="{hb}"/>\n' + lit_tip()


def tray():
    pick = lambda px: 16 if px <= 20 else (22 if px <= 23 else (24 if px < 32 else 256))
    names = {16: "s16", 22: "s22", 24: "s24", 256: "m"}
    for theme in ("dark", "light", "template"):
        for st in STATES:
            if theme == "template" and st in ("connected", "controlling"):
                continue
            for grid, gk in names.items():
                write(f"tray/svg/{theme}/tray-{st}-{gk}.svg", svg(grid, grid, tray_body(st, grid, theme), f"oneMK tray, {st}"))
            for size in (16, 22, 24, 32):
                for sc in (1, 2):
                    px = size * sc
                    gk = names[pick(px)]
                    nm = f"tray-{st}-{size}{'@2x' if sc == 2 else ''}.png"
                    if theme == "template":
                        nm = f"tray-{st}-{size}Template{'@2x' if sc == 2 else ''}.png"
                    render(os.path.join(BRAND, f"tray/svg/{theme}/tray-{st}-{gk}.svg"), os.path.join(BRAND, f"tray/{theme}/{nm}"), px)


# ================================================================= web

def web():
    W = os.path.join(BRAND, "web")
    write("web/src/favicon-16.svg", icon_svg("square", "tiny", "oneMK"))
    write("web/src/favicon.svg", icon_svg("square", "mid", "oneMK"))
    write("web/src/touch.svg", icon_svg("bleed", "master", "oneMK"))
    write("web/src/maskable.svg", maskable())
    shutil.copy(os.path.join(W, "src/favicon.svg"), os.path.join(W, "favicon.svg"))
    jobs = [("favicon-16", 16, "favicon-16"), ("favicon", 32, "favicon-32"), ("favicon", 48, "favicon-48"),
            ("touch", 180, "apple-touch-icon"), ("touch", 192, "icon-192"), ("touch", 512, "icon-512"),
            ("maskable", 512, "maskable-512")]
    for src, px, name in jobs:
        render(os.path.join(W, f"src/{src}.svg"), os.path.join(W, f"{name}.png"), px)
    pack_ico([os.path.join(W, f"favicon-{p}.png") for p in (16, 32, 48)], os.path.join(W, "favicon.ico"))
    man = {"name": "oneMK", "short_name": "oneMK",
           "icons": [{"src": "/icon-192.png", "sizes": "192x192", "type": "image/png"},
                     {"src": "/icon-512.png", "sizes": "512x512", "type": "image/png"},
                     {"src": "/maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable"}],
           "theme_color": M.GROUND, "background_color": M.TILE_BOTTOM, "display": "standalone"}
    open(os.path.join(W, "site.webmanifest"), "w").write(json.dumps(man, indent=2) + "\n")


def maskable():
    """Full-bleed tile, mark inside the 80 % safe circle."""
    return icon_svg("bleed", "master", "oneMK", mark_k=SCALE["maskable"])


if __name__ == "__main__":
    steps = sys.argv[1:] or ["logo", "png", "app", "tray", "web"]
    if "logo" in steps:
        symbol_files(); wordmark_files(); lockup_files()
    if "png" in steps:
        logo_pngs()
    if "app" in steps:
        app_icons()
    if "tray" in steps:
        tray()
    if "web" in steps:
        web()
    print("done")
