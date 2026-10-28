#!/usr/bin/env python3
"""Converts brand PNGs (docs/brand) into the raw RGBA files the app and
tray embed, so neither needs an image decoder at run time.

Each .rgba file is: width (u32 little-endian), height (u32 LE), then
width*height*4 bytes of straight (not premultiplied) RGBA.
Needs Pillow. Run from anywhere: python3 scripts/brand-assets.py"""
import os
import struct
from PIL import Image

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
BRAND = os.path.join(ROOT, "docs", "brand")


def write(img, out):
    img = img.convert("RGBA")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "wb") as f:
        f.write(struct.pack("<II", *img.size))
        f.write(img.tobytes())
    print(os.path.relpath(out, ROOT), img.size)


def brand(*p):
    return Image.open(os.path.join(BRAND, *p))


app = os.path.join(ROOT, "crates", "app", "assets")
# The top bar lockup, at 2x of its 22 px display height.
lock = brand("logo", "png", "onemk-lockup-horizontal-reversed.png")
h = 44
write(lock.resize((round(lock.width * h / lock.height), h), Image.LANCZOS), os.path.join(app, "lockup.rgba"))
# The window icon.
write(brand("app-icon", "windows-linux", "hicolor", "256x256", "apps", os.listdir(os.path.join(BRAND, "app-icon", "windows-linux", "hicolor", "256x256", "apps"))[0]), os.path.join(app, "icon-256.rgba"))

tray = os.path.join(ROOT, "crates", "tray", "assets")
for state in ["off", "idle", "connected", "controlling"]:
    for size in ["22", "22@2x", "16@2x", "32@2x"]:
        name = f"tray-{state}-{size}.png"
        path = os.path.join(BRAND, "tray", "dark", name)
        if os.path.exists(path):
            write(Image.open(path), os.path.join(tray, f"{state}-{size.replace('@2x', 'x2')}.rgba"))
    for size in ["16@2x"]:
        path = os.path.join(BRAND, "tray", "template", f"tray-{state}-{size.replace('@2x', '')}Template@2x.png")
        if os.path.exists(path):
            write(Image.open(path), os.path.join(tray, f"{state}-template-32.rgba"))
