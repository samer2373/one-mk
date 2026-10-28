"""Build the one·mk logo kit into docs/brand/. Run: python3 docs/brand/src/kit.py
Needs only Python 3 + Pillow (for .ico/.icns packing is done by hand; Pillow is used to read PNG sizes)
and a renderer for render_png.py (headless Chrome here)."""
import os, sys, json, math, struct, subprocess, shutil
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
from geom import *
from mark import *
from wordmark import wordmark, rrect_d, XH, ASC

BRAND = os.path.dirname(HERE)
RENDER = os.path.expanduser("~/.claude/skills/logo-design/scripts/render_png.py")
BLACK, WHITE = "#000000", "#FFFFFF"
LIGHT_INK = "#0F1216"          # ink on light backgrounds = the ground colour

def svg(w, h, body, title, vb=None):
    vb = vb or f"0 0 {F(w)} {F(h)}"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" width="{F(w)}" height="{F(h)}" role="img" aria-labelledby="title">\n'
            f'  <title id="title">{title}</title>\n  {body}\n</svg>\n')
def write(rel, text):
    p = os.path.join(BRAND, rel); os.makedirs(os.path.dirname(p), exist_ok=True); open(p, "w").write(text); return p
def render(src, out, w, h=None, pad=0):
    os.makedirs(os.path.dirname(out), exist_ok=True)
    cmd = ["python3", RENDER, src, "-o", out, "--width", str(w), "--height", str(h or w)]
    if pad: cmd += ["--padding", str(pad)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode: print(r.stdout, r.stderr)

# ------------------------------------------------------------------ 1. symbol masters
MB = (18, 71, 238, 181)          # tight bbox of M after its shift (x0, y0, x1, y1)
SCHEMES = {                       # name: (ink, signal)
    "color": (LIGHT_INK, AMBER),  # for light backgrounds
    "reversed": (INK, AMBER),     # for dark backgrounds (primary)
    "black": (BLACK, BLACK),
    "white": (WHITE, WHITE),
}
def symbol_files():
    out = {}
    for k, (ink, sig) in SCHEMES.items():
        out[k] = write(f"logo/onemk-symbol-{k}.svg", svg(256, 256, build(M, ink, sig), "one·mk symbol"))
    for name, g, vb in (("small16", S16, 16), ("small22", S22, 22), ("small24", S24, 24)):
        for k in ("color", "reversed", "black", "white"):
            ink, sig = SCHEMES[k]
            write(f"logo/onemk-symbol-{name}-{k}.svg", svg(vb, vb, build(g, ink, sig), f"one·mk symbol, {vb} px drawing"))
    return out

# ------------------------------------------------------------------ 2. wordmark + lockups
WM_SIZE = 200
def wordmark_files():
    base = 100 + XH*WM_SIZE/1000
    for k, (ink, sig) in SCHEMES.items():
        body, right = wordmark(ink, sig, 0, base, WM_SIZE)
        top = base - ASC*WM_SIZE/1000
        write(f"logo/onemk-wordmark-{k}.svg", svg(right, base - top + 4, body, "one·mk", vb=f"0 {F(top)} {F(right)} {F(base-top+4)}"))
def mark_group(ink, sig, x, y, h):
    """The full-size mark scaled so its tight bbox is h tall, with its top-left at (x, y)."""
    s = h / (MB[3]-MB[1])
    return f'<g transform="translate({F(x - MB[0]*s)} {F(y - MB[1]*s)}) scale({F(s)})">{build(M, ink, sig)}</g>', (MB[2]-MB[0])*s
def lockup_files():
    xh = XH*WM_SIZE/1000; asc = ASC*WM_SIZE/1000
    for k, (ink, sig) in SCHEMES.items():
        # horizontal: mark bottom on the baseline, mark height = 1.25 x-height + ascender difference
        mh = asc * 1.0
        base = asc + 4
        g, mw = mark_group(ink, sig, 0, base - mh, mh)
        gap = xh * 0.55
        body, right = wordmark(ink, sig, mw + gap, base, WM_SIZE)
        write(f"logo/onemk-lockup-horizontal-{k}.svg", svg(right, base + 4, g + body, "one·mk"))
        # stacked: mark centred above the wordmark
        wbody, wr = wordmark(ink, sig, 0, 0, WM_SIZE)
        mw2 = wr * 0.62; mh2 = mw2 * (MB[3]-MB[1]) / (MB[2]-MB[0])
        g2, _ = mark_group(ink, sig, (wr - mw2)/2, 0, mh2)
        base2 = mh2 + xh*0.75 + asc
        wbody, wr = wordmark(ink, sig, 0, base2, WM_SIZE)
        write(f"logo/onemk-lockup-stacked-{k}.svg", svg(wr, base2 + 4, g2 + wbody, "one·mk"))

# ------------------------------------------------------------------ 3. app icons
def squircle(x, y, s, n=5.0, k=128):
    c = s/2; pts = []
    for i in range(k):
        t = 2*math.pi*i/k; ct, st = math.cos(t), math.sin(t)
        pts.append((x + c + c*math.copysign(abs(ct)**(2/n), ct), y + c + c*math.copysign(abs(st)**(2/n), st)))
    return "M" + " L".join(f"{F(a)} {F(b)}" for a, b in pts) + " Z"
def tile_svg(drawing, vb, shape="squircle", mark_w=0.70, title="one·mk app icon"):
    """1024 canvas. macOS: squircle 824 with 100 margin (Big Sur grid). Windows/Linux: rounded square, 32 margin."""
    S = 1024
    if shape == "squircle":
        m = 100; s = S - 2*m; d = squircle(m, m, s)
    elif shape == "bleed":
        m = 0; s = S; d = f"M0 0 H{S} V{S} H0 Z"
    else:
        m = 32; s = S - 2*m; d = rrect_d(m, m, m+s, m+s, s*0.2)
    defs = (f'<defs><linearGradient id="tile" x1="0" y1="{m}" x2="0" y2="{m+s}" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="#1B2027"/><stop offset="1" stop-color="#0D1014"/></linearGradient>'
            f'<linearGradient id="rim" x1="0" y1="{m}" x2="0" y2="{m+s}" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="#FFFFFF" stop-opacity=".18"/><stop offset=".45" stop-color="#FFFFFF" stop-opacity="0"/></linearGradient></defs>')
    tile = f'<path d="{d}" fill="url(#tile)"/>' + ("" if shape == "bleed" else f'<path d="{d}" fill="none" stroke="url(#rim)" stroke-width="{F(s/206)}"/>')
    g, bbox, gvb = drawing
    bw, bh = bbox[2]-bbox[0], bbox[3]-bbox[1]
    k = s*mark_w / bw
    cx, cy = S/2, S/2 - s*0.01
    tx, ty = cx - (bbox[0] + bw/2)*k, cy - (bbox[1] + bh/2)*k
    return svg(S, S, defs + tile + f'<g transform="translate({F(tx)} {F(ty)}) scale({F(k)})">{g}</g>', title)
def drawing(gm, ink=INK, sig=AMBER):
    """(svg body, tight bbox, grid) for a geometry dict."""
    dx, dy = gm["shift"]
    xs = [gm["mon"][0], gm["mon"][2], gm["foot"][0], gm["foot"][2]] + ([gm["lap"][0], gm["base"][0], gm["base"][2]] if gm.get("lap") else [gm["tip"][0]])
    ys = [gm["mon"][1], gm["foot"][3]] + ([gm["base"][3]] if gm.get("lap") else [])
    return build(gm, ink, sig), (min(xs)+dx, min(ys)+dy, max(xs)+dx, max(ys)+dy), None

def pack_ico(pngs, out):
    data = [open(p, "rb").read() for p in pngs]
    from PIL import Image
    sizes = [Image.open(p).size for p in pngs]
    hdr = struct.pack("<HHH", 0, 1, len(data)); off = 6 + 16*len(data); ents = b""
    for (w, h), d in zip(sizes, data):
        ents += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(d), off); off += len(d)
    open(out, "wb").write(hdr + ents + b"".join(data))
def pack_icns(entries, out):
    """entries: list of (OSType, png path). PNG payloads are valid for every modern icon type."""
    body = b""
    for code, p in entries:
        d = open(p, "rb").read(); body += code.encode() + struct.pack(">I", len(d) + 8) + d
    open(out, "wb").write(b"icns" + struct.pack(">I", len(body) + 8) + body)

def app_icons():
    big = drawing(M); mid = drawing(S24); one = drawing(S16_ONE)
    mac = {"master": tile_svg(big, None, "squircle", 0.74), "mid": tile_svg(mid, None, "squircle", 0.80),
           "tiny": tile_svg(one, None, "squircle", 0.86)}
    win = {"master": tile_svg(big, None, "square", 0.76), "mid": tile_svg(mid, None, "square", 0.84),
           "tiny": tile_svg(one, None, "square", 0.90)}
    for k, v in mac.items(): write(f"app-icon/macos/onemk-appicon-{k}.svg", v)
    for k, v in win.items(): write(f"app-icon/windows-linux/onemk-appicon-{k}.svg", v)
    pick = lambda px: "tiny" if px <= 20 else ("mid" if px <= 48 else "master")
    # macOS iconset
    iconset = os.path.join(BRAND, "app-icon/macos/onemk.iconset"); os.makedirs(iconset, exist_ok=True)
    names = []
    for pt in (16, 32, 128, 256, 512):
        for sc in (1, 2):
            px = pt*sc; n = f"icon_{pt}x{pt}{'@2x' if sc == 2 else ''}.png"
            render(os.path.join(BRAND, f"app-icon/macos/onemk-appicon-{pick(px)}.svg"), os.path.join(iconset, n), px)
            names.append((pt, sc, n))
    for px in (64, 1024):
        render(os.path.join(BRAND, f"app-icon/macos/onemk-appicon-{pick(px)}.svg"), os.path.join(BRAND, f"app-icon/macos/png/onemk-{px}.png"), px)
    I = lambda n: os.path.join(iconset, n)
    pack_icns([("icp4", I("icon_16x16.png")), ("ic11", I("icon_16x16@2x.png")), ("icp5", I("icon_32x32.png")),
               ("ic12", I("icon_32x32@2x.png")), ("ic07", I("icon_128x128.png")), ("ic13", I("icon_128x128@2x.png")),
               ("ic08", I("icon_256x256.png")), ("ic14", I("icon_256x256@2x.png")), ("ic09", I("icon_512x512.png")),
               ("ic10", I("icon_512x512@2x.png"))], os.path.join(BRAND, "app-icon/macos/onemk.icns"))
    for pt in (16, 32, 128, 256, 512):
        pass
    # Windows .ico
    wdir = os.path.join(BRAND, "app-icon/windows-linux/ico-png"); pngs = []
    for px in (16, 20, 24, 32, 40, 48, 64, 256):
        p = os.path.join(wdir, f"onemk-{px}.png"); render(os.path.join(BRAND, f"app-icon/windows-linux/onemk-appicon-{pick(px)}.svg"), p, px); pngs.append(p)
    pack_ico(pngs, os.path.join(BRAND, "app-icon/windows-linux/onemk.ico"))
    # Linux hicolor
    for px in (16, 22, 24, 32, 48, 64, 128, 256, 512):
        render(os.path.join(BRAND, f"app-icon/windows-linux/onemk-appicon-{pick(px)}.svg"),
               os.path.join(BRAND, f"app-icon/windows-linux/hicolor/{px}x{px}/apps/one-mk.png"), px)
    sc = os.path.join(BRAND, "app-icon/windows-linux/hicolor/scalable/apps"); os.makedirs(sc, exist_ok=True)
    shutil.copy(os.path.join(BRAND, "app-icon/windows-linux/onemk-appicon-master.svg"), os.path.join(sc, "one-mk.svg"))

def web_icons():
    """Favicons and PWA icons (the manifest and <head> snippet come from export_variants.py)."""
    W = os.path.join(BRAND, "web"); os.makedirs(W, exist_ok=True)
    big = drawing(M); mid = drawing(S24); one = drawing(S16_ONE)
    write("web/src/favicon-16.svg", tile_svg(one, None, "square", 0.90, "one·mk"))
    write("web/src/favicon.svg", tile_svg(mid, None, "square", 0.84, "one·mk"))
    write("web/src/touch.svg", tile_svg(big, None, "bleed", 0.70, "one·mk"))
    write("web/src/maskable.svg", tile_svg(big, None, "bleed", 0.56, "one·mk"))
    shutil.copy(os.path.join(W, "src/favicon.svg"), os.path.join(W, "favicon.svg"))
    jobs = [("favicon-16", 16, "favicon-16"), ("favicon", 32, "favicon-32"), ("favicon", 48, "favicon-48"),
            ("touch", 180, "apple-touch-icon"), ("touch", 192, "icon-192"), ("touch", 512, "icon-512"), ("maskable", 512, "maskable-512")]
    for src, px, name in jobs:
        render(os.path.join(W, f"src/{src}.svg"), os.path.join(W, f"{name}.png"), px)
    pack_ico([os.path.join(W, f"favicon-{p}.png") for p in (16, 32, 48)], os.path.join(W, "favicon.ico"))
    for junk in ("onemk-square.svg", "onemk-favicon.svg", "onemk-app-icon.svg"):
        p = os.path.join(W, junk)
        if os.path.exists(p): os.remove(p)

# ------------------------------------------------------------------ 4. tray icons
TRAY_STATES = ("off", "idle", "connected", "controlling")
def tray_body(state, gm, theme):
    """theme: dark (light ink on a dark bar), light (dark ink on a light bar), template (black + alpha, macOS)."""
    if theme == "dark": ink, sig = INK, AMBER
    elif theme == "light": ink, sig = "#1B1F24", AMBER
    else: ink, sig = BLACK, BLACK
    outline = ("#1B1F24", gm["gap"]*0.55) if theme == "light" else None
    lit_gm = dict(gm, litgap=gm.get("litgap", 0 if gm["t"] < 4 else gm["t"]*0.6))
    if state == "off":
        return build(gm, ink, sig, cursor_on=False, ink_opacity=0.45)
    if state == "idle":
        return build(gm, ink, ink, cursor_col=ink)
    if state == "connected":
        return build(gm, ink, sig, outline=outline)
    return build(lit_gm, ink, sig, lit=sig, outline=outline)
def tray():
    grids = {"s16": (S16, 16), "s22": (S22, 22), "s24": (S24, 24), "m": (M, 256)}
    pick = lambda px: "s16" if px <= 20 else ("s22" if px <= 23 else ("s24" if px < 32 else "m"))
    for theme in ("dark", "light", "template"):
        for st in TRAY_STATES:
            if theme == "template" and st in ("connected", "controlling"): continue
            for gk, (gm, vb) in grids.items():
                write(f"tray/svg/{theme}/tray-{st}-{gk}.svg", svg(vb, vb, tray_body(st, gm, theme), f"one·mk tray, {st}"))
            for size in (16, 22, 24, 32):
                for sc in (1, 2):
                    px = size*sc; gk = pick(px)
                    name = f"tray-{st}-{size}{'@2x' if sc == 2 else ''}.png"
                    if theme == "template": name = f"tray-{st}-{size}Template{'@2x' if sc == 2 else ''}.png"
                    render(os.path.join(BRAND, f"tray/svg/{theme}/tray-{st}-{gk}.svg"), os.path.join(BRAND, f"tray/{theme}/{name}"), px)

# ------------------------------------------------------------------ 5. pngs of the logo files
def logo_pngs():
    L = os.path.join(BRAND, "logo")
    for k in SCHEMES:
        render(f"{L}/onemk-symbol-{k}.svg", f"{L}/png/onemk-symbol-{k}-1024.png", 1024)
        render(f"{L}/onemk-symbol-{k}.svg", f"{L}/png/onemk-symbol-{k}-256.png", 256)
        render(f"{L}/onemk-symbol-small16-{k}.svg", f"{L}/png/onemk-symbol-{k}-16.png", 16)
        render(f"{L}/onemk-symbol-small24-{k}.svg", f"{L}/png/onemk-symbol-{k}-24.png", 24)
        render(f"{L}/onemk-symbol-{k}.svg", f"{L}/png/onemk-symbol-{k}-32.png", 32)
        for kind, (w, h) in (("wordmark", (1200, 300)), ("lockup-horizontal", (1600, 300)), ("lockup-stacked", (900, 640))):
            render(f"{L}/onemk-{kind}-{k}.svg", f"{L}/png/onemk-{kind}-{k}.png", w, h, pad=0.06)

if __name__ == "__main__":
    steps = sys.argv[1:] or ["logo", "app", "tray", "png"]
    if "logo" in steps: symbol_files(); wordmark_files(); lockup_files()
    if "app" in steps: app_icons()
    if "tray" in steps: tray()
    if "png" in steps: logo_pngs()
    if "web" in steps: web_icons()
    print("done")
