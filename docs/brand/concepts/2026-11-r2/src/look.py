#!/usr/bin/env python3
"""Fast contact sheet: one HTML page, one Chrome screenshot.
Each row: 200 px on white, 32/22/16 px true size, 22 and 16 px rasterised then enlarged x6 (pixelated),
then the same on a dark glass ground (#161A20) with the art inverted to light (for black masters) or as-is (--asis).
Usage: look.py out.png [--asis] a.svg b.svg ..."""
import os, subprocess, sys, pathlib

R = "/home/samer/.claude/skills/logo-design/scripts/render_png.py"
out = sys.argv[1]
args = sys.argv[2:]
asis = "--asis" in args
files = [a for a in args if a != "--asis"]
inv = "" if asis else "filter: invert(1) brightness(0.94);"
rows = []
for i, f in enumerate(files):
    u = pathlib.Path(os.path.abspath(f)).as_uri()
    rows.append(f'''<div class="row"><div class="w"><img src="{u}" width="200" height="200">
  <img src="{u}" width="32" height="32"><img src="{u}" width="22" height="22"><img src="{u}" width="16" height="16">
  <canvas data-src="{u}" data-s="22"></canvas><canvas data-src="{u}" data-s="16"></canvas></div>
  <div class="d"><img src="{u}" width="200" height="200" style="{inv}">
  <img src="{u}" width="32" height="32" style="{inv}"><img src="{u}" width="22" height="22" style="{inv}"><img src="{u}" width="16" height="16" style="{inv}"></div>
  <span>{os.path.basename(f)}</span></div>''')
html = f'''<!doctype html><html><head><style>
body{{margin:0;font:12px sans-serif;background:#fff}}
.row{{display:flex;align-items:center;gap:0;height:220px;position:relative}}
.w,.d{{display:flex;align-items:center;gap:14px;padding:10px;height:200px}}
.d{{background:#161A20}} img{{display:block}}
canvas{{image-rendering:pixelated}}
span{{position:absolute;left:230px;bottom:6px;color:#888}}
</style></head><body>{"".join(rows)}
<script>
document.querySelectorAll('canvas').forEach(c=>{{const s=+c.dataset.s;const im=new Image();
im.onload=()=>{{const t=document.createElement('canvas');t.width=s;t.height=s;const x=t.getContext('2d');
x.fillStyle='#fff';x.fillRect(0,0,s,s);x.drawImage(im,0,0,s,s);c.width=s*6;c.height=s*6;
c.style.width=s*6+'px';c.style.height=s*6+'px';const y=c.getContext('2d');y.imageSmoothingEnabled=false;y.drawImage(t,0,0,s*6,s*6);}};
im.src=c.dataset.src;}});
</script></body></html>'''
hp = out.replace(".png", ".html")
open(hp, "w").write(html)
subprocess.run(["python3", R, hp, "-o", out, "--width", "1250", "--height", str(220 * len(files))], check=True,
               stdout=subprocess.DEVNULL)
print(out)
