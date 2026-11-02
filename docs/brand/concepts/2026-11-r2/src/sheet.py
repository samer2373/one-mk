#!/usr/bin/env python3
"""Colour concept sheet for round 2: large colour marks, app icons on a dark desktop, sidebar header on dark glass.
Writes <OUT>/concepts-r2.html (kept for reference) and <OUT>/concepts-r2.png."""
import os, subprocess, sys

OUT = os.environ["OUT"]
R = "/home/samer/.claude/skills/logo-design/scripts/render_png.py"
C = [
    ("d1-lens", "D1 · Lens", "Glass object · app icon is the concept",
     "A glass lens sits on the seam between two screens; seen through it, the two are one picture."),
    ("d2-pointer", "D2 · Pointer", "The pointer, reinvented",
     "A pointer cut by the screen edge: its tip has already crossed and lights up on the next computer."),
    ("d3-converge", "D3 · Converge", "Abstract symbol",
     "Three trails of colour, one per computer, run into one solid point."),
]
REC = "d2-pointer"


def card(k, title, kind, idea):
    rec = '<div class="rec">recommended</div>' if k == REC else ""
    return f'''<div class="card{' r' if k == REC else ''}">
  <div class="hero"><img src="{k}-symbol-color.svg" width="300" height="300"></div>
  <div class="row"><img class="icon" src="{k}-appicon-1024.png" width="200" height="200">
    <div class="col">
      <div class="chip w"><img src="{k}-symbol.svg" width="56" height="56"></div>
      <div class="chip d"><img src="{k}-symbol-white.svg" width="56" height="56"></div>
      <div class="chip w"><img src="{k}-symbol-color-light.svg" width="56" height="56"></div>
    </div>
    <div class="col tiny">
      <div class="bar d"><img src="{k}-small16.svg" width="16" height="16" style="filter:invert(1)"><img src="{k}-small22.svg" width="22" height="22" style="filter:invert(1)"></div>
      <div class="bar l"><img src="{k}-small16.svg" width="16" height="16"><img src="{k}-small22.svg" width="22" height="22"></div>
      <div class="cap">menu bar · 16 / 22 px, true size</div>
    </div>
  </div>
  <div class="lock"><img src="{k}-lockup-color.svg" height="70"></div>
  <div class="lockl"><img src="{k}-lockup-color-light.svg" height="56"><img src="{k}-lockup.svg" height="40" style="margin-left:36px"></div>
  <h2>{title}</h2>{rec}<div class="kind">{kind}</div><p>{idea}</p>
</div>'''


dock = "".join(f'<img src="{k}-appicon-1024.png" width="104" height="104">' for k, *_ in C)
fillers = "".join(f'<div class="ph" style="background:{c}"></div>' for c in
                  ("linear-gradient(#5a6170,#2e333c)", "linear-gradient(#3d6b5c,#1d3a31)", "linear-gradient(#6b5a3d,#3a2f1d)"))
tray = "".join(f'<img src="{k}-small16.svg" width="16" height="16" style="filter:invert(1)">' for k, *_ in C)
side = "".join(f'''<div class="side"><div class="lights"><i></i><i></i><i></i></div>
  <div class="hdr"><img src="{k}-lockup-color.svg" height="22"></div>
  <div class="item on"></div><div class="item"></div><div class="item"></div><div class="item"></div>
  <div class="zoom"><img src="{k}-lockup-color.svg" height="44"><span>2x</span></div></div>''' for k, *_ in C)
desk_icons = "".join(f'<div class="dicon"><img src="{k}-appicon-1024.png" width="128" height="128"><span>oneMK</span></div>'
                     for k, *_ in C)

html = f'''<!doctype html><html><head><meta charset="utf-8"><style>
@font-face{{font-family:G;src:url("file:///home/samer/.t3/projects/one-mk/.claude/worktrees/polish/crates/app/fonts/Geist-Regular.ttf")}}
@font-face{{font-family:G;font-weight:600;src:url("file:///home/samer/.t3/projects/one-mk/.claude/worktrees/polish/crates/app/fonts/Geist-SemiBold.ttf")}}
body{{margin:0;background:#0c0d11;color:#e9ebf0;font-family:G,sans-serif;width:2400px}}
header{{padding:48px 64px 8px}} header h1{{margin:0;font-size:44px;font-weight:600;letter-spacing:-.5px}}
header p{{margin:8px 0 0;color:#9aa3af;font-size:20px}}
.cards{{display:flex;gap:32px;padding:32px 64px}}
.card{{flex:1;background:linear-gradient(#171a21,#111318);border:1px solid rgba(255,255,255,.08);border-radius:28px;padding:28px;position:relative}}
.card.r{{border-color:rgba(200,255,74,.45);box-shadow:0 0 0 1px rgba(200,255,74,.15) inset}}
.hero{{height:340px;display:flex;align-items:center;justify-content:center;border-radius:20px;
  background:radial-gradient(circle at 50% 30%,#232733,#14161c 70%)}}
.row{{display:flex;gap:22px;align-items:center;margin-top:22px}}
.icon{{filter:drop-shadow(0 10px 20px rgba(0,0,0,.5))}}
.col{{display:flex;flex-direction:column;gap:10px}}
.chip{{width:72px;height:72px;border-radius:14px;display:flex;align-items:center;justify-content:center}}
.chip.w{{background:#fff}} .chip.d{{background:#161A20;border:1px solid rgba(255,255,255,.1)}}
.bar{{height:28px;border-radius:6px;display:flex;gap:14px;align-items:center;padding:0 12px;width:120px}}
.bar.d{{background:#1e1f24}} .bar.l{{background:#ececee}}
.cap{{color:#7d8592;font-size:13px}}
.lock{{margin-top:24px;height:110px;display:flex;align-items:center;padding:0 22px;border-radius:18px;
  background:linear-gradient(rgba(255,255,255,.06),rgba(255,255,255,.02));border:1px solid rgba(255,255,255,.08)}}
.lockl{{margin-top:14px;height:96px;display:flex;align-items:center;padding:0 22px;border-radius:18px;background:#fff}}
h2{{margin:22px 0 2px;font-size:30px;font-weight:600}}
.kind{{color:#9aa3af;font-size:16px;text-transform:uppercase;letter-spacing:1.5px}}
.rec{{position:absolute;top:22px;right:24px;background:#c8ff4a;color:#111;font-weight:600;font-size:14px;padding:6px 12px;border-radius:99px;letter-spacing:1px;text-transform:uppercase}}
p{{font-size:20px;line-height:1.4;color:#c7ccd4;margin:10px 0 0}}
section{{padding:24px 64px 48px}} section h3{{font-size:24px;font-weight:600;margin:0 0 16px;color:#cfd4dc}}
.desk{{height:560px;border-radius:24px;position:relative;overflow:hidden;
  background:radial-gradient(1200px 500px at 20% 10%,#3b2a5a,transparent 60%),radial-gradient(900px 500px at 85% 80%,#173f4a,transparent 60%),linear-gradient(#141821,#0a0b10)}}
.menubar{{height:30px;background:rgba(20,22,28,.55);backdrop-filter:blur(20px);display:flex;align-items:center;justify-content:flex-end;gap:18px;padding:0 18px;font-size:14px;color:#e8e8ea}}
.dock{{position:absolute;bottom:22px;left:50%;transform:translateX(-50%);display:flex;gap:14px;padding:12px 16px;border-radius:30px;
  background:rgba(255,255,255,.12);border:1px solid rgba(255,255,255,.22);box-shadow:0 20px 50px rgba(0,0,0,.45)}}
.ph{{width:104px;height:104px;border-radius:24px}}
.dicons{{position:absolute;top:70px;right:60px;display:flex;flex-direction:column;gap:20px}}
.dicon{{display:flex;flex-direction:column;align-items:center;gap:6px;font-size:14px}}
.dicons2{{position:absolute;top:80px;left:70px;display:flex;gap:56px}}
.sides{{display:flex;gap:32px}}
.side{{width:300px;height:420px;border-radius:22px;padding:16px;position:relative;
  background:linear-gradient(rgba(54,56,61,.55),rgba(31,33,38,.55)),radial-gradient(circle at 70% 120%,#2b3f66,#141821);
  border:1px solid rgba(255,255,255,.14);box-shadow:inset 0 1px 0 rgba(255,255,255,.18)}}
.lights{{display:flex;gap:8px}} .lights i{{width:12px;height:12px;border-radius:50%;background:#ff5f57}} .lights i+i{{background:#febc2e}} .lights i+i+i{{background:#28c840}}
.hdr{{margin:22px 4px 18px}}
.item{{height:30px;border-radius:9px;background:rgba(255,255,255,.05);margin:8px 0}} .item.on{{background:rgba(255,255,255,.12)}}
.zoom{{position:absolute;left:16px;right:16px;bottom:16px;height:90px;border-radius:14px;background:rgba(0,0,0,.25);display:flex;align-items:center;justify-content:center;gap:12px}}
.zoom span{{color:#7d8592;font-size:13px}}
.note{{color:#7d8592;font-size:15px;margin-top:10px}}
</style></head><body>
<header><h1>oneMK — round 2 concepts</h1>
<p>Three directions, one concept each, shown in their proposed colour. Wordmark: Geist (OFL), “one” at weight 600, “MK” drawn as small capitals at 88 % height and weight 660, so their stems match.</p></header>
<div class="cards">{"".join(card(*c) for c in C)}</div>
<section><h3>As app icons on a dark desktop</h3>
<div class="desk"><div class="menubar">{tray}<span>Sun 1 Nov 23:40</span></div>
<div class="dicons2">{desk_icons}</div>
<div class="dock">{fillers}{dock}{fillers}</div></div>
<div class="note">Menu bar: the three 16 px cuts in white at true size, left to right D1, D2, D3. Dock: icons at 104 px, between neutral placeholders.</div></section>
<section><h3>In the window's dark glass sidebar header (22 px tall, 1x; 2x below)</h3><div class="sides">{side}</div></section>
</body></html>'''
hp = os.path.join(OUT, "concepts-r2.html")
open(hp, "w").write(html)
subprocess.run(["python3", R, hp, "-o", os.path.join(OUT, "concepts-r2.png"), "--width", "2400", "--height",
                sys.argv[1] if len(sys.argv) > 1 else "2300"], check=True, stdout=subprocess.DEVNULL)
print(os.path.join(OUT, "concepts-r2.png"))
