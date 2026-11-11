#!/usr/bin/env python3
"""Turns the 2x window screenshots (from app-shots.sh) into the page's images:
AVIF and WebP at three widths plus a PNG fallback, for the full windows
(assets/img/shots) and the detail crops (assets/img/crops), light and dark.
Needs Pillow, and ffmpeg with libaom for AVIF (skipped when missing).
    python3 -I site/tools/build-images.py SHOTS_DIR site/assets/img"""
import shutil, subprocess, sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from PIL import Image

SHOTS = ["desk", "arrange", "wallpaper", "files", "keys-caps", "health", "settings"]
# name: (source shot, box in 2880x1800 pixels)
CROPS = {
    "arrange-plan": ("arrange", (150, 100, 1850, 820)),
    "wallpaper-plan": ("wallpaper", (250, 300, 2630, 1100)),
    "mouse": ("mouse", (40, 740, 2060, 1480)),
    "health": ("health", (240, 150, 2640, 1250)),
    "files-desk": ("files", (260, 320, 2620, 1250)),
    "palette": ("palette", (780, 250, 2100, 880)),
    "pair": ("pair", (960, 600, 1920, 1200)),
    "keys-board": ("keys-caps", (60, 420, 2080, 1340)),
}
src, out = Path(sys.argv[1]), Path(sys.argv[2])
avif = shutil.which("ffmpeg") is not None
jobs = []

def emit(im, folder, stem, widths):
    folder.mkdir(parents=True, exist_ok=True)
    for w in widths:
        h = round(im.height * w / im.width)
        r = im if w == im.width else im.resize((w, h), Image.LANCZOS)
        r.save(folder / f"{stem}-{w}.webp", quality=84, method=6)
        if avif:
            tmp = folder / f"{stem}-{w}.tmp.png"
            r.save(tmp)
            jobs.append((tmp, folder / f"{stem}-{w}.avif"))
    fw = widths[0]
    im.resize((fw, round(im.height * fw / im.width)), Image.LANCZOS).quantize(256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE).save(folder / f"{stem}.png", optimize=True)

def enc(job):
    tmp, dst = job
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", str(tmp), "-c:v", "libaom-av1", "-still-picture", "1",
                    "-crf", "28", "-cpu-used", "5", "-pix_fmt", "yuv444p", str(dst)], check=True)
    tmp.unlink()

for theme in ("light", "dark"):
    for name in SHOTS:
        emit(Image.open(src / f"{name}-{theme}.png").convert("RGB"), out / "shots", f"{name}-{theme}", [960, 1440, 2880])
    for name, (shot, box) in CROPS.items():
        im = Image.open(src / f"{shot}-{theme}.png").convert("RGB").crop(box)
        full = im.width
        emit(im, out / "crops", f"{name}-{theme}", sorted({min(full, 800), full // 2 if full // 2 > 800 else min(full, 800), full}))
with ThreadPoolExecutor(6) as pool:
    list(pool.map(enc, jobs))
for name, (shot, box) in CROPS.items():
    print(f"{name}: {box[2]-box[0]}x{box[3]-box[1]}")
