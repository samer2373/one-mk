#!/usr/bin/env python3
"""Draws the page's own wallpaper picture (layered hills at dusk), used as the
fake playing video in the Wallpaper screenshots. Ours, no third-party art.
    python3 -I site/tools/make-wallpaper.py OUT.png [WIDTH HEIGHT]"""
import math, random, sys
from PIL import Image, ImageDraw, ImageFilter

out = sys.argv[1]
W, H = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (3840, 2160)
def lerp(a, b, t): return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))
sky = [(0.0, (22, 30, 78)), (0.45, (62, 84, 190)), (0.72, (196, 140, 170)), (1.0, (246, 196, 150))]
img = Image.new("RGB", (W, H))
px = img.load()
col = []
for y in range(H):
    t = y / (H - 1) / 0.78
    t = min(t, 1)
    for k in range(len(sky) - 1):
        if sky[k][0] <= t <= sky[k + 1][0]:
            c = lerp(sky[k][1], sky[k + 1][1], (t - sky[k][0]) / (sky[k + 1][0] - sky[k][0])); break
    col.append(c)
row = Image.new("RGB", (1, H)); row.putdata(col); img = row.resize((W, H))
# The sun's bloom, low and right of centre.
glow = Image.new("L", (W, H), 0); d = ImageDraw.Draw(glow)
cx, cy, r = int(W * 0.62), int(H * 0.66), int(H * 0.5)
for i in range(60, 0, -1):
    rr = r * i / 60; d.ellipse((cx - rr * 1.6, cy - rr, cx + rr * 1.6, cy + rr), fill=int(255 * (1 - i / 60) ** 2))
img = Image.composite(Image.new("RGB", (W, H), (255, 226, 190)), img, glow.filter(ImageFilter.GaussianBlur(H // 30)))
# Hills, back to front, each darker and bluer.
layers = [((122, 112, 170), 0.62, 0.035, 1.3, 0.2), ((78, 76, 146), 0.70, 0.05, 2.1, 1.7),
          ((44, 48, 110), 0.79, 0.06, 1.6, 3.1), ((22, 26, 64), 0.89, 0.05, 2.7, 4.4)]
for colr, base, amp, freq, ph in layers:
    m = Image.new("L", (W, H), 0); d = ImageDraw.Draw(m)
    pts = [(x, H * (base - amp * math.sin(x / W * math.pi * freq + ph) - amp * 0.4 * math.sin(x / W * math.pi * freq * 3.1 + ph * 2))) for x in range(0, W + 8, 8)]
    d.polygon(pts + [(W, H), (0, H)], fill=255)
    img = Image.composite(Image.new("RGB", (W, H), colr), img, m.filter(ImageFilter.GaussianBlur(3)))
# A little grain against banding.
noise = Image.effect_noise((W, H), 6).convert("RGB")
img = Image.blend(img, noise, 0.03)
img.save(out)
print(out)
