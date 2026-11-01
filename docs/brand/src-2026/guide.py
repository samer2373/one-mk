#!/usr/bin/env python3
"""oneMK guideline sheet, misuse examples, the refinement before/after, palette.json and the
presentation board. Run after kit.py:

    python3 docs/brand/src-2026/guide.py

Needs the logo-design skill's render_png.py and presentation_board.py (headless Chrome)."""
import json, math, os, pathlib, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import mark as M
from kit import svg, write, BRAND, RENDER, mark_body

SKILL = os.path.expanduser("~/.claude/skills/logo-design/scripts")
GL = os.path.join(BRAND, "guideline")
FONTS = os.path.normpath(os.path.join(BRAND, "..", "..", "crates", "app", "fonts"))


def uri(rel):
    return pathlib.Path(os.path.join(BRAND, rel)).as_uri()


def lum(h):
    c = [int(h[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    c = [x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]


def cr(a, b):
    la, lb = sorted((lum(a), lum(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


PALETTE = [
    # token, hex, role, measured against
    ("accent", M.ACCENT, "Volt. The lit tip; switches that are on, the primary button, focus ring, the pointer on the desk", M.GROUND),
    ("accent-lit", M.ACCENT_LIT, "Hover and pressed-in state of anything in the accent", M.GROUND),
    ("accent-ink", M.ACCENT_INK, "Text and icons on the accent", M.ACCENT),
    ("accent-deep", M.ACCENT_DEEP, "The accent as a graphic on white and light trays", "#FFFFFF"),
    ("ok", M.OK, "Connected / ok status. Never the accent, so 'on' and 'connected' never look alike", M.GROUND),
    ("ground", M.GROUND, "Window ground; ink of the mark on light grounds", "#FFFFFF"),
    ("ink", M.INK, "Ink of the mark on dark grounds", M.GROUND),
    ("tile-top", M.TILE_TOP, "App-icon tile, top", None),
    ("tile-bottom", M.TILE_BOTTOM, "App-icon tile, bottom", None),
]


def palette_json():
    out = {}
    for tok, hx, role, against in PALETTE:
        e = {"value": hx, "role": role}
        if against:
            e["contrast"] = {"against": against, "ratio": round(cr(hx, against), 2)}
        e["on_ground"] = round(cr(hx, M.GROUND), 2)
        e["on_glass_solid"] = round(cr(hx, "#161A20"), 2)
        out[tok] = e
    p = os.path.join(HERE, "palette.json")
    json.dump(out, open(p, "w"), indent=2)
    return out


# ---------------------------------------------------------------- misuse art (256 grid)

def misuse():
    d = os.path.join(GL, "misuse")
    for f in os.listdir(d):
        os.remove(os.path.join(d, f))
    ptr = M.pointer(M.MASTER)
    tip, body = M.paths(ptr)
    # 1. recoloured tip
    write("guideline/misuse/recolour.svg", svg(256, 256, f'  <path fill="{M.INK}" d="{body}"/>\n  <path fill="#3B82F6" d="{tip}"/>\n',
                                                "Misuse: recoloured tip"))
    # 2. detached tip: pulled away along the pointer's axis
    write("guideline/misuse/detached-tip.svg", svg(256, 256,
          f'  <path fill="{M.INK}" d="{body}"/>\n  <g transform="translate(-26 -26)"><path fill="{M.ACCENT}" d="{tip}"/></g>\n',
          "Misuse: detached tip"))
    # 3. rotated
    write("guideline/misuse/rotated.svg", svg(256, 256,
          f'  <g transform="rotate(-38 128 128)"><path fill="{M.INK}" d="{body}"/><path fill="{M.ACCENT}" d="{tip}"/></g>\n',
          "Misuse: rotated"))
    # 4. effects: glow + gradient body
    defs = ('    <filter id="glow" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="10"/></filter>\n'
            '    <linearGradient id="gr" x1="0" y1="40" x2="0" y2="220" gradientUnits="userSpaceOnUse">'
            '<stop offset="0" stop-color="#FF5FD2"/><stop offset="1" stop-color="#5B8CFF"/></linearGradient>\n')
    write("guideline/misuse/effects.svg", svg(256, 256,
          f'  <path fill="{M.ACCENT}" filter="url(#glow)" d="{tip} {body}"/>\n  <path fill="url(#gr)" d="{body}"/>\n  <path fill="{M.ACCENT}" d="{tip}"/>\n',
          "Misuse: effects", defs))


# ---------------------------------------------------------------- before / after

def before_after():
    b_tip, b_body = M.paths(M.pointer(M.CONCEPT_R2))
    a_tip, a_body = M.paths(M.pointer(M.MASTER))
    write("src-2026/.build/before.svg", svg(256, 256, f'  <path fill="{M.INK}" d="{b_body}"/>\n  <path fill="{M.ACCENT}" d="{b_tip}"/>\n', "before"))
    write("src-2026/.build/after.svg", svg(256, 256, f'  <path fill="{M.INK}" d="{a_body}"/>\n  <path fill="{M.ACCENT}" d="{a_tip}"/>\n', "after"))
    write("src-2026/.build/before-k.svg", svg(256, 256, f'  <path fill="#000" d="{b_body} {b_tip}"/>\n', "before"))
    s16 = uri("logo/onemk-symbol-small16-reversed.svg")
    html = f'''<!doctype html><meta charset="utf-8"><style>
@font-face{{font-family:G;src:url("{pathlib.Path(FONTS, 'Geist-Regular.ttf').as_uri()}")}}
@font-face{{font-family:G;font-weight:600;src:url("{pathlib.Path(FONTS, 'Geist-SemiBold.ttf').as_uri()}")}}
body{{margin:0;width:1400px;background:#0F1216;color:#EDEFF2;font-family:G}}
.r{{display:flex;gap:40px;padding:36px 48px;align-items:center}} h2{{font-weight:600;font-size:26px;margin:0 0 6px}}
p{{color:#9DA6B1;font-size:17px;margin:0;max-width:430px;line-height:1.45}}
.c{{background:#161A20;border-radius:20px;padding:20px;display:flex;gap:26px;align-items:center}}
.k{{background:#fff;border-radius:20px;padding:20px;display:flex;gap:26px;align-items:center}}
img{{display:block}} .px{{image-rendering:pixelated}}
</style>
<div class="r"><div><h2>Before (concept round 2)</h2><p>14-unit gap and rounded cut corners: the lit tip read as a separate triangle.</p></div>
<div class="c"><img src="{uri('src-2026/.build/before.svg')}" width="220"><img src="{uri('src-2026/.build/before.svg')}" width="32"><img src="{uri('src-2026/.build/before.svg')}" width="16"></div>
<div class="k"><img src="{uri('src-2026/.build/before-k.svg')}" width="120"></div></div>
<div class="r"><div><h2>After (final)</h2><p>A 6.5-unit cut with square ends: the edges run straight through, so it is one pointer crossing an edge. Below 32 px the pixel-fitted drawings keep a whole-pixel cut.</p></div>
<div class="c"><img src="{uri('src-2026/.build/after.svg')}" width="220"><img src="{uri('src-2026/.build/after.svg')}" width="32"><img src="{s16}" width="16"></div>
<div class="k"><img src="{uri('logo/onemk-symbol-black.svg')}" width="120"></div></div>'''
    hp = os.path.join(HERE, ".build", "before-after.html")
    open(hp, "w").write(html)
    subprocess.run(["python3", RENDER, hp, "-o", os.path.join(GL, "refinement-before-after.png"), "--width", "1400", "--height", "680"],
                   check=True, stdout=subprocess.DEVNULL)


# ---------------------------------------------------------------- guideline sheet

def guideline(pal):
    sw = "".join(f'''<div class="sw"><i style="background:{hx}"></i><b>{tok}</b><code>{hx}</code>
      <span>{pal[tok].get('contrast', {}).get('ratio', '')}{':1 on ' + ('ground' if pal[tok].get('contrast', {}).get('against') == M.GROUND else ('white' if pal[tok].get('contrast', {}).get('against') == '#FFFFFF' else 'accent')) if pal[tok].get('contrast') else ''}</span></div>'''
                 for tok, hx, role, _ in PALETTE if not tok.startswith("tile"))
    states = ""
    for st, label in (("off", "Off: hollow, dim"), ("idle", "Idle: tip unlit"), ("connected", "Connected: tip lit"),
                      ("controlling", "Controlling: the pointer has crossed")):
        states += f'''<div class="st"><div class="bar d"><img src="{uri(f'tray/dark/tray-{st}-16.png')}" width="16"><img src="{uri(f'tray/dark/tray-{st}-22.png')}" width="22"><img class="px" src="{uri(f'tray/dark/tray-{st}-16.png')}" width="64"></div>
        <div class="bar l"><img src="{uri(f'tray/light/tray-{st}-16.png')}" width="16"><img src="{uri(f'tray/light/tray-{st}-22.png')}" width="22"><img class="px" src="{uri(f'tray/light/tray-{st}-16.png')}" width="64"></div><p>{label}</p></div>'''
    mis = "".join(f'<div class="mi"><img src="{uri("guideline/misuse/" + f)}" width="150"><p>{t}</p></div>' for f, t in
                  (("recolour.svg", "Don't recolour the tip"), ("detached-tip.svg", "Don't pull the tip away"),
                   ("rotated.svg", "Don't rotate or tilt"), ("effects.svg", "No glows or gradients on the flat mark")))
    html = f'''<!doctype html><html><head><meta charset="utf-8"><style>
@font-face{{font-family:G;src:url("{pathlib.Path(FONTS, 'Geist-Regular.ttf').as_uri()}")}}
@font-face{{font-family:G;font-weight:600;src:url("{pathlib.Path(FONTS, 'Geist-SemiBold.ttf').as_uri()}")}}
@font-face{{font-family:GM;src:url("{pathlib.Path(FONTS, 'GeistMono-Regular.ttf').as_uri()}")}}
*{{box-sizing:border-box;margin:0}}
body{{width:2400px;background:#0F1216;color:#EDEFF2;font-family:G;padding:72px 88px}}
h1{{font-weight:600;font-size:44px;letter-spacing:-.02em}} h2{{font-family:GM;font-weight:400;font-size:15px;letter-spacing:.1em;text-transform:uppercase;color:{M.ACCENT};margin-bottom:18px}}
p{{color:#C9CDD3;font-size:19px;line-height:1.5}} code{{font-family:GM;font-size:15px;color:#9DA6B1}}
.mast{{display:flex;justify-content:space-between;align-items:flex-end;border-bottom:1px solid #252B34;padding-bottom:28px;margin-bottom:36px}}
.grid{{display:grid;grid-template-columns:1.2fr 1fr 1fr;gap:28px;margin-bottom:28px}}
.card{{background:#13171C;border:1px solid #1F242B;border-radius:24px;padding:30px}}
.art{{display:flex;align-items:center;justify-content:center;background:#0F1216;border-radius:16px;height:320px;margin:6px 0 18px;gap:40px}}
.wh{{background:#fff}} .row{{display:flex;gap:18px;align-items:center}}
.sws{{display:grid;grid-template-columns:1fr 1fr;gap:12px}} .sw{{display:grid;grid-template-columns:48px 1fr;grid-template-rows:auto auto auto;column-gap:14px;align-items:center}}
.sw i{{grid-row:span 3;width:48px;height:48px;border-radius:12px;border:1px solid rgba(255,255,255,.12)}} .sw b{{font-weight:600;font-size:17px}} .sw span{{color:#9DA6B1;font-size:14px}}
.st{{display:flex;flex-direction:column;gap:8px}} .bar{{display:flex;gap:16px;align-items:center;padding:8px 12px;border-radius:8px;width:180px}}
.bar.d{{background:#1e1f24}} .bar.l{{background:#ececee}} .px{{image-rendering:pixelated}} .st p{{font-size:15px;color:#9DA6B1}}
.sts{{display:flex;gap:24px}} .mis{{display:flex;gap:22px}} .mi{{background:#0F1216;border-radius:16px;padding:14px;text-align:center;position:relative}}
.mi:after{{content:"";position:absolute;inset:14px;background:linear-gradient(45deg,transparent 49%,#FF6B5E 49.5%,#FF6B5E 50.5%,transparent 51%);opacity:.8}}
.mi p{{font-size:14px;color:#9DA6B1;width:150px}}
.icons{{display:flex;gap:22px;align-items:flex-end}}
.clear{{position:relative;display:inline-block;padding:34px;border:1px dashed #3a414c;border-radius:6px}}
</style></head><body>
<div class="mast"><div><h1>oneMK brand guideline</h1><p style="color:#9DA6B1">The Pointer · November 2026 · CLI <code>one-mk</code></p></div>
<img src="{uri('logo/onemk-lockup-horizontal-reversed.svg')}" height="70"></div>
<div class="grid">
 <div class="card"><h2>The mark</h2><div class="art"><img src="{uri('logo/onemk-symbol-reversed.svg')}" width="290"></div>
  <p>A pointer cut by the edge of a screen. Its tip has already crossed to the next computer and is lit: one mouse and keyboard, on every screen on the desk.</p>
  <p style="margin-top:10px"><code>edges 15° and 60° · notch 165° and 90° · cut at 135°, 6.5/256 wide · Geist wordmark</code></p></div>
 <div class="card"><h2>Colour versions</h2>
  <div class="row" style="margin-bottom:14px"><div class="art" style="flex:1;height:150px"><img src="{uri('logo/onemk-symbol-reversed.svg')}" width="110"></div>
   <div class="art wh" style="flex:1;height:150px"><img src="{uri('logo/onemk-symbol-color.svg')}" width="110"></div></div>
  <div class="row"><div class="art wh" style="flex:1;height:150px"><img src="{uri('logo/onemk-symbol-black.svg')}" width="110"></div>
   <div class="art" style="flex:1;height:150px;background:{M.ACCENT}"><img src="{uri('logo/onemk-symbol-black.svg')}" width="110"></div></div>
  <p>Dark grounds: reversed. Light grounds: colour (the tip keeps an ink edge). On the accent and on photos: one colour.</p></div>
 <div class="card"><h2>Small sizes</h2><div class="art" style="height:230px;flex-direction:column;gap:18px">
   <div class="row"><img src="{uri('logo/onemk-symbol-small16-reversed.svg')}" width="16"><img src="{uri('logo/onemk-symbol-small22-reversed.svg')}" width="22"><img src="{uri('logo/onemk-symbol-small24-reversed.svg')}" width="24"><img src="{uri('logo/onemk-symbol-reversed.svg')}" width="32"></div>
   <div class="row"><img class="px" src="{uri('logo/png/onemk-symbol-reversed-16.png')}" width="96"><img class="px" src="{uri('logo/png/onemk-symbol-reversed-24.png')}" width="96"></div></div>
  <p>Below 32 px use the pixel-fitted drawings: 16 px (16–20), 22 px (21–23), 24 px (24–31). They keep a whole-pixel cut so the tip still reads.</p></div>
</div>
<div class="grid">
 <div class="card"><h2>Clear space and minimum size</h2><div class="art" style="height:250px"><span class="clear"><img src="{uri('logo/onemk-lockup-horizontal-reversed.svg')}" height="80"></span></div>
  <p>Clear space on every side: the cap height of “MK” (in the symbol alone, a third of its height). Minimum: symbol 16 px; horizontal lockup 18 px tall (90 px wide, 20 mm in print).</p></div>
 <div class="card"><h2>Colour</h2><div class="sws">{sw}</div></div>
 <div class="card"><h2>Wordmark</h2><div class="art" style="height:150px"><img src="{uri('logo/onemk-wordmark-reversed.svg')}" height="62"></div>
  <p>Geist (SIL OFL), drawn as outlines: “one” at weight 600, “MK” as small capitals at 88 % height and weight 660 so the stems match. Always “oneMK”; the CLI is <code>one-mk</code>.</p></div>
</div>
<div class="grid" style="grid-template-columns:1.2fr 2fr">
 <div class="card"><h2>App icon</h2><div class="icons"><img src="{uri('app-icon/macos/png/onemk-1024.png')}" width="220"><img src="{uri('app-icon/windows-linux/hicolor/128x128/apps/one-mk.png')}" width="96">
  <img src="{uri('app-icon/macos/onemk.iconset/icon_32x32.png')}" width="32"><img src="{uri('app-icon/macos/onemk.iconset/icon_16x16.png')}" width="16"></div>
  <p style="margin-top:12px">Deep green-black glass tile; frosted white body; the lit tip. 16–32 px use a flat, pixel-fitted drawing.</p></div>
 <div class="card"><h2>Tray states (16 / 22 px, true size and 16 px ×4)</h2><div class="sts">{states}</div>
  <p style="margin-top:12px">Distinct in one colour: off is hollow, idle has a dim tip, connected lights the tip, controlling leaves only the tip solid.</p></div>
</div>
<div class="card"><h2>Don't</h2><div class="mis">{mis}</div></div>
</body></html>'''
    hp = os.path.join(GL, "guideline.html")
    open(hp, "w").write(html)
    subprocess.run(["python3", RENDER, hp, "-o", os.path.join(GL, "onemk-brand-guideline.png"), "--width", "2400",
                    "--height", sys.argv[1] if len(sys.argv) > 1 else "2330"], check=True, stdout=subprocess.DEVNULL)


# ---------------------------------------------------------------- presentation board (skill tool)

def board():
    spec = {
        "brand": "oneMK",
        "tagline": "One mouse and keyboard for every computer on your desk",
        "brief": "A fast, very light app that moves one mouse, keyboard, clipboard and files between the macOS, Windows "
                 "and Linux computers on a desk, as if all the screens were one. Premium, precise, calm.",
        "adjectives": ["precise", "effortless", "calm", "fast", "premium"],
        "criteria": "Ownable twist on the pointer, readable at 16 px in one colour, a glass app icon in the macOS 26 style.",
        "industry": "software",
        "mockups": ["app-icon", "readme", "terminal", "favicon-tab", "website", "sticker"],
        "nav": ["Download", "How it works", "GitHub"],
        "brand_color": M.ACCENT,
        "tile_color": M.TILE_BOTTOM,
        "greyscale": False, "final": True, "designer": "oneMK", "round": 5,
        "concepts": [{
            "name": "Pointer",
            "symbol": "../logo/onemk-symbol-color.svg",
            "lockup": "../logo/onemk-lockup-horizontal-color.svg",
            "stacked": "../logo/onemk-lockup-stacked-color.svg",
            "avatar": "../logo/onemk-symbol-small24-color.svg",
            "symbol_on_tile": "../logo/onemk-symbol-reversed.svg",
            "stacked_on_tile": "../logo/onemk-lockup-stacked-reversed.svg",
            "symbol_on_dark": "../logo/onemk-symbol-reversed.svg",
            "lockup_on_dark": "../logo/onemk-lockup-horizontal-reversed.svg",
            "stacked_on_dark": "../logo/onemk-lockup-stacked-reversed.svg",
            "idea": "A pointer cut by the edge of a screen: its tip has already crossed to the next computer and is lit.",
            "rationale": ["The pointer is what the app moves; the cut is the screen edge it crosses",
                          "One shape, two colours: body and lit tip, readable from 16 px to a sign",
                          "Volt on deep green-black: no one in the category (all blues) owns it"]}],
        "recommendation": "Pointer: final artwork, app icons, tray states and web icons are in docs/brand/.",
        "next_steps": ["Use the proposed palette in crates/app/src/style.rs, then run scripts/brand-tokens.py",
                       "Run scripts/brand-assets.py to embed the new tray and window art",
                       "Update the tray tests that look for amber"]}
    sp = os.path.join(GL, "board-spec.json")
    json.dump(spec, open(sp, "w"), indent=2)
    bd = os.path.join(GL, "board")
    for f in os.listdir(bd):
        if f.startswith("slide-"):
            os.remove(os.path.join(bd, f))
    subprocess.run(["python3", os.path.join(SKILL, "presentation_board.py"), sp, "-o", os.path.join(GL, "board.html"),
                    "--png-dir", bd], check=True, stdout=subprocess.DEVNULL)
    slides = sorted(f for f in os.listdir(bd) if f.startswith("slide-"))
    # the board image = the slides stacked
    from PIL import Image
    ims = [Image.open(os.path.join(bd, f)).convert("RGB") for f in slides]
    W = max(i.width for i in ims); H = sum(i.height for i in ims)
    out = Image.new("RGB", (W, H), "#0F1216"); y = 0
    for i in ims:
        out.paste(i, (0, y)); y += i.height
    out.save(os.path.join(GL, "onemk-presentation-board.png"))
    return slides


if __name__ == "__main__":
    pal = palette_json()
    misuse()
    before_after()
    guideline(pal)
    print(board())
