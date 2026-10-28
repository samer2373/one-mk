"""Render the one-page brand guideline sheet: guideline/onemk-brand-guideline.png"""
import os, sys, subprocess
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
from mark import *
from kit import svg, write, BRAND, RENDER, MB
FONTS = os.path.normpath(os.path.join(BRAND, "../../crates/app/fonts"))
U = lambda p: "file://" + os.path.join(BRAND, p)
# misuse artwork
mis = {
    "recolour": build(M, INK, "#4C8DFF"),
    "amber-devices": build(M, AMBER, INK),
    "no-clearance": build(dict(M, gap=0), INK, AMBER),
    "outline-pointer": build(M, INK, GROUND, outline=(AMBER, 3)),
}
for k, b in mis.items(): write(f"guideline/misuse/{k}.svg", svg(256, 256, b, "misuse"))
sw = [("ground", "#0F1216", "Ground", "16.3:1 with ink"), ("panel", "#13171C", "Panel", "15.6:1 with ink"),
      ("raised", "#171B21", "Raised", ""), ("line", "#252B34", "Line", "hairlines only"),
      ("ink", "#EDEFF2", "Ink", "16.3:1 on ground"), ("ink2", "#9DA6B1", "Ink 2", "7.6:1 on ground"),
      ("signal", "#FFB547", "Signal", "10.7:1 on ground"), ("signal-ink", "#1E1505", "Signal ink", "10.3:1 on signal")]
swh = "".join(f'<div class="sw"><div class="chip" style="background:{h};{"border:1px solid #2a3039" if n in ("ground","panel","raised") else ""}"></div><b>{t}</b><code>{h}</code><span>{c}</span></div>' for n, h, t, c in sw)
mis_cells = [
    ("Don't stretch or squash", f'<img src="{U("logo/onemk-symbol-reversed.svg")}" style="transform:scaleX(1.45)">'),
    ("Don't recolour the pointer", f'<img src="{U("guideline/misuse/recolour.svg")}">'),
    ("Don't make the devices amber", f'<img src="{U("guideline/misuse/amber-devices.svg")}">'),
    ("Don't remove the clearance", f'<img src="{U("guideline/misuse/no-clearance.svg")}">'),
    ("Don't rotate or tilt", f'<img src="{U("logo/onemk-symbol-reversed.svg")}" style="transform:rotate(-14deg)">'),
    ("No glows or shadows", f'<img src="{U("logo/onemk-symbol-reversed.svg")}" style="filter:drop-shadow(0 0 14px #FFB547)">'),
    ("No busy or mid-grey grounds", f'<div style="background:#8C939C;width:100%;height:100%;display:flex;align-items:center;justify-content:center"><img src="{U("logo/onemk-symbol-reversed.svg")}"></div>'),
    ("Don't outline the pointer", f'<img src="{U("guideline/misuse/outline-pointer.svg")}">'),
]
mish = "".join(f'<div class="mis"><div class="mbox">{a}<i>×</i></div><span>{t}</span></div>' for t, a in mis_cells)
states = "".join(f'<div class="st"><div class="bar"><img src="{U(f"tray/dark/tray-{s}-22@2x.png")}" width=22><img src="{U(f"tray/dark/tray-{s}-16.png")}" width=16></div><div class="bar lt"><img src="{U(f"tray/light/tray-{s}-22@2x.png")}" width=22><img src="{U(f"tray/light/tray-{s}-16.png")}" width=16></div><b>{s.title()}</b></div>' for s in ("off", "idle", "connected", "controlling"))
mh = MB[3] - MB[1]
html = f'''<!doctype html><html><head><meta charset="utf-8"><style>
@font-face {{ font-family: Geist; src: url("file://{FONTS}/Geist-Regular.ttf"); font-weight: 400; }}
@font-face {{ font-family: Geist; src: url("file://{FONTS}/Geist-SemiBold.ttf"); font-weight: 600; }}
@font-face {{ font-family: GM; src: url("file://{FONTS}/GeistMono-Regular.ttf"); }}
* {{ box-sizing: border-box; margin: 0; }}
body {{ width: 2400px; background: #0F1216; color: #EDEFF2; font-family: Geist; padding: 80px 96px; }}
h1 {{ font-weight: 600; font-size: 44px; letter-spacing: -.02em; }}
h2 {{ font-family: GM; font-weight: 400; font-size: 15px; letter-spacing: .1em; text-transform: uppercase; color: #FFB547; margin-bottom: 18px; }}
p, li {{ color: #C9CDD3; font-size: 20px; line-height: 1.5; }}
code {{ font-family: GM; font-size: 15px; color: #9DA6B1; }}
.mast {{ display: flex; justify-content: space-between; align-items: flex-end; border-bottom: 1px solid #252B34; padding-bottom: 32px; margin-bottom: 40px; }}
.mast img {{ height: 70px; }}
.grid {{ display: grid; grid-template-columns: 1.25fr 1fr 1fr; gap: 32px; }}
.card {{ background: #13171C; border: 1px solid #1F242B; border-radius: 24px; padding: 32px; }}
.hero {{ display: flex; flex-direction: column; }}
.hero .art {{ flex: 1; display: flex; align-items: center; justify-content: center; background: #0F1216; border-radius: 16px; margin: 18px 0 16px; height: 300px; }}
.hero .art img {{ width: 78%; }}
.cs {{ position: relative; background: #0F1216; border-radius: 16px; height: 300px; display: flex; align-items: center; justify-content: center; }}
.cs .box {{ position: relative; outline: 2px dashed #FFB54766; padding: 50px 50px; }}
.cs .box img {{ height: 100px; display: block; }}
.cs .lbl {{ position: absolute; font-family: GM; font-size: 13px; color: #FFB547; }}
.sizes {{ display: flex; align-items: flex-end; gap: 26px; background: #0F1216; border-radius: 16px; padding: 28px; height: 300px; }}
.sizes div {{ display: flex; flex-direction: column; align-items: center; gap: 10px; font-family: GM; font-size: 12px; color: #9DA6B1; }}
.sws {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 16px; }}
.sw {{ display: flex; flex-direction: column; gap: 6px; font-size: 16px; }}
.sw .chip {{ height: 70px; border-radius: 12px; }}
.sw span {{ font-size: 13px; color: #9DA6B1; }}
.bgs {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 14px; }}
.bgs div {{ height: 120px; border-radius: 14px; display: flex; align-items: center; justify-content: center; position: relative; }}
.bgs img {{ width: 70%; }}
.bgs span {{ position: absolute; left: 10px; bottom: 6px; font-family: GM; font-size: 11px; opacity: .7; }}
.miss {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 16px; }}
.mis span {{ font-size: 15px; color: #C9CDD3; display: block; margin-top: 8px; }}
.mbox {{ position: relative; height: 120px; background: #0F1216; border-radius: 14px; overflow: hidden; display: flex; align-items: center; justify-content: center; }}
.mbox img {{ width: 62%; }}
.mbox i {{ position: absolute; right: 10px; top: 4px; font-style: normal; color: #FF6B5B; font-size: 26px; }}
.sts {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 14px; }}
.st b {{ font-weight: 600; font-size: 15px; display: block; margin-top: 8px; }}
.bar {{ background: #1D2128; height: 46px; border-radius: 10px; display: flex; align-items: center; justify-content: center; gap: 16px; margin-bottom: 6px; }}
.bar.lt {{ background: #E9E9E7; }}
.wide {{ grid-column: span 2; }}
</style></head><body>
<div class="mast"><div><img src="{U("logo/onemk-lockup-horizontal-reversed.svg")}"><h1 style="margin-top:28px">Brand guideline</h1></div>
<code style="text-align:right;line-height:1.8">ONE PAGE · v1.0<br>OCTOBER 2026</code></div>
<div class="grid">
 <div class="card hero"><h2>The idea</h2>
  <p>A laptop and a monitor, and one pointer standing on the seam between them. That is the whole product: one mouse and keyboard, crossing from one computer to the next. The pointer is the only amber on the page, and a clear gap separates it from the devices.</p>
  <div class="art"><img src="{U("logo/onemk-symbol-reversed.svg")}" style="width:auto;height:150%"></div>
  <p style="font-size:17px;color:#9DA6B1">Masters: <code>logo/onemk-symbol-*.svg</code>, <code>logo/onemk-lockup-*.svg</code>, <code>logo/onemk-wordmark-*.svg</code>. The clearance around the pointer is a real hole, so the mark works on any ground.</p>
 </div>
 <div class="card"><h2>Clear space</h2>
  <div class="cs"><div class="box"><img src="{U("logo/onemk-symbol-reversed.svg")}" style="height:180px;margin:-40px -10px"><span class="lbl" style="top:14px;left:12px">½ h</span><span class="lbl" style="bottom:14px;right:12px">½ h</span></div></div>
  <p style="margin-top:16px">Keep half the mark's height (h) clear on every side. In lockups, h is the height of the mark.</p>
 </div>
 <div class="card"><h2>Minimum size</h2>
  <div class="sizes">
   <div><img src="{U("logo/onemk-lockup-horizontal-reversed.svg")}" style="width:150px">lockup 120 px</div>
   <div><img src="{U("logo/onemk-symbol-reversed.svg")}" style="width:48px">32 px</div>
   <div><img src="{U("logo/png/onemk-symbol-reversed-24.png")}" style="width:24px">24 px</div>
   <div><img src="{U("logo/png/onemk-symbol-reversed-16.png")}" style="width:16px">16 px</div>
  </div>
  <p style="margin-top:16px">Below 32 px use the pixel-fitted 24, 22 and 16 px drawings, never the master scaled down.</p>
 </div>
 <div class="card wide"><h2>Colour</h2><div class="sws">{swh}</div>
  <p style="margin-top:18px">Amber is the only signal colour. Use it for the pointer, live state and the one primary action. Never use it for decoration or for large fills. Type: <b>Geist</b> 400/600 at 15 / 13 / 12 px, and <b>Geist Mono</b> for numbers and keys.</p></div>
 <div class="card"><h2>Approved grounds</h2><div class="bgs">
   <div style="background:#0F1216"><img src="{U("logo/onemk-symbol-reversed.svg")}"><span style="color:#9DA6B1">ground</span></div>
   <div style="background:#171B21"><img src="{U("logo/onemk-symbol-reversed.svg")}"><span style="color:#9DA6B1">raised</span></div>
   <div style="background:#FFFFFF"><img src="{U("logo/onemk-symbol-color.svg")}"><span style="color:#555">white</span></div>
   <div style="background:#F2F1EE"><img src="{U("logo/onemk-symbol-color.svg")}"><span style="color:#555">paper</span></div>
   <div style="background:#FFB547"><img src="{U("logo/onemk-symbol-black.svg")}"><span style="color:#1E1505">amber: black only</span></div>
   <div style="background:#000"><img src="{U("logo/onemk-symbol-white.svg")}"><span style="color:#9DA6B1">one-colour white</span></div>
   <div style="background:#fff"><img src="{U("logo/onemk-symbol-black.svg")}"><span style="color:#555">one-colour black</span></div>
   <div style="background:linear-gradient(135deg,#1d3557,#3a2a4a)"><img src="{U("logo/onemk-symbol-white.svg")}"><span style="color:#ccc">photo: white</span></div>
 </div></div>
 <div class="card wide"><h2>Misuse</h2><div class="miss">{mish}</div></div>
 <div class="card"><h2>Tray states</h2><div class="sts">{states}</div>
  <p style="margin-top:16px;font-size:17px">Off is dimmed, Idle has an ink pointer, Connected has an amber pointer, and Controlling also lights the far screen. Geometry for <code>icon.rs</code> is in <code>tray-geometry.md</code>.</p></div>
</div></body></html>'''
write("guideline/guideline.html", html)
subprocess.run(["python3", RENDER, os.path.join(BRAND, "guideline/guideline.html"), "-o", os.path.join(BRAND, "guideline/onemk-brand-guideline.png"), "--width", "2400", "--height", "1880"])
