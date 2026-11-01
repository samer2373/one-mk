#!/usr/bin/env python3
"""Liquid-Glass-style app icon compositor (used by kit.py for the 128 px and larger app icons) (numpy + Pillow; SVG rasterised with the skill's render_png.py).
A presentation render, not a production asset pipeline: each layer is a flat SVG shape; the glass look
(frosted body, refraction at the rim, top light, rim light, soft shadow) is computed here."""
import os, subprocess, tempfile, hashlib
import numpy as np
from PIL import Image, ImageFilter

R = "/home/samer/.claude/skills/logo-design/scripts/render_png.py"
TMP = os.path.join(os.path.dirname(os.path.abspath(__file__)), ".build", "ras")
os.makedirs(TMP, exist_ok=True)
S = 1024


def raster(svg_body, defs="", size=S):
    """Render an SVG fragment (1024 user space) to float RGBA array 0..1."""
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" width="{size}" height="{size}">'
           f'<defs>{defs}</defs>{svg_body}</svg>')
    h = hashlib.md5((svg + str(size)).encode()).hexdigest()[:16]
    sp, pp = os.path.join(TMP, h + ".svg"), os.path.join(TMP, h + ".png")
    if not os.path.exists(pp):
        open(sp, "w").write(svg)
        subprocess.run(["python3", R, sp, "-o", pp, "--size", str(size)], check=True, stdout=subprocess.DEVNULL)
    return np.asarray(Image.open(pp).convert("RGBA"), dtype=np.float32) / 255.0


def _box1(a, r, axis):
    if r < 1:
        return a
    pad = [(0, 0)] * a.ndim
    pad[axis] = (r + 1, r)
    p = np.pad(a, pad, mode="edge")
    c = np.cumsum(p, axis=axis, dtype=np.float64)
    n_ = a.shape[axis]
    hi = np.take(c, np.arange(2 * r + 1, 2 * r + 1 + n_), axis=axis)
    lo = np.take(c, np.arange(0, n_), axis=axis)
    return ((hi - lo) / (2 * r + 1)).astype(np.float32)


def blur(a, sigma):
    """Approximate Gaussian blur (three box passes) for (H,W) or (H,W,C) float arrays."""
    if sigma <= 0:
        return a
    r = max(1, int(round(np.sqrt(12 * sigma * sigma / 3 + 1) / 2 - 0.5)))
    out = a
    for _ in range(3):
        out = _box1(out, r, 0)
        out = _box1(out, r, 1)
    return out


def shift(a, dx, dy):
    out = np.zeros_like(a)
    H, W = a.shape[:2]
    xs0, xs1 = max(0, -dx), min(W, W - dx)
    ys0, ys1 = max(0, -dy), min(H, H - dy)
    out[ys0 + dy:ys1 + dy, xs0 + dx:xs1 + dx] = a[ys0:ys1, xs0:xs1]
    return out


def sample(img, X, Y):
    """Bilinear sample img (H,W,C) at float coords X,Y."""
    H, W = img.shape[:2]
    X = np.clip(X, 0, W - 1.001); Y = np.clip(Y, 0, H - 1.001)
    x0 = np.floor(X).astype(int); y0 = np.floor(Y).astype(int)
    fx = (X - x0)[..., None]; fy = (Y - y0)[..., None]
    a = img[y0, x0]; b = img[y0, x0 + 1]; c = img[y0 + 1, x0]; d = img[y0 + 1, x0 + 1]
    return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy


YY, XX = np.mgrid[0:S, 0:S].astype(np.float32)


def glass_layer(C, mask, colour, tint=0.8, frost=10, refr=26, shadow=0.42, rim=0.9, spec=0.30,
                magnify=None, chroma=0.0, top_light=0.22, depth=0.18, behind_src=None, caustic=0.0, caustic_col=None):
    """Composite one glass layer onto C (H,W,3). mask: (H,W) 0..1. colour: (H,W,3) fill (gradients allowed)."""
    # 1. soft shadow under the layer
    sh = shift(blur(mask, 16), 0, 16)
    C = C * (1 - shadow * sh[..., None])
    # 2. what is behind, frosted, and refracted near the rim
    src_img = behind_src if behind_src is not None else C
    behind = blur(src_img, frost) if frost else src_img
    g = blur(mask, 18)
    gy, gx = np.gradient(g)
    X = XX + refr * gx * 18
    Y = YY + refr * gy * 18
    if magnify:
        cx, cy, k = magnify
        X = cx + (X - cx) * k
        Y = cy + (Y - cy) * k
    if chroma:
        bR = sample(behind, X * (1 + chroma) - chroma * XX, Y)[..., 0]
        bG = sample(behind, X, Y)[..., 1]
        bB = sample(behind, X * (1 - chroma) + chroma * XX, Y)[..., 2]
        refracted = np.stack([bR, bG, bB], -1)
    else:
        refracted = sample(behind, X, Y)
    body = refracted * (1 - tint) + colour * tint
    # 3. volume: lighter at the top, a little darker toward the bottom edge
    ys = np.where(mask > 0.01)
    if len(ys[0]):
        y0, y1 = ys[0].min(), ys[0].max()
        ramp = np.clip((YY - y0) / max(1, (y1 - y0)), 0, 1)
        body = body * (1 + top_light * (1 - ramp)[..., None] - depth * (ramp ** 2)[..., None])
    inner = blur(mask, 7)
    edge = np.clip((mask - inner) * 2.6, 0, 1)
    # inner shade along the edge (thickness), then rim light strongest at the top
    body = body * (1 - 0.22 * edge[..., None])
    top_edge = np.clip(mask - shift(mask, 0, 7), 0, 1)
    top_edge = blur(top_edge, 2.2)
    bot_edge = blur(np.clip(mask - shift(mask, 0, -6), 0, 1), 2.0)
    body = body + rim * top_edge[..., None] * 0.85 + 0.35 * rim * edge[..., None] * 0.35 + 0.25 * bot_edge[..., None]
    # 4. specular sheen: a soft band across the upper part of the shape
    if spec and len(ys[0]):
        band = np.exp(-((ramp - 0.12) ** 2) / 0.012)
        body = body + spec * (band * inner)[..., None]
    if caustic and len(ys[0]):
        # light focused through the glass: a bright tinted crescent inside the lower edge
        low = np.clip(mask - shift(mask, 0, -int(0.10 * (y1 - y0) + 4)), 0, 1)
        low = blur(low, 0.035 * (y1 - y0) + 3) * mask
        cc = np.array(caustic_col if caustic_col is not None else (1, 1, 1), dtype=np.float32)
        body = body + caustic * low[..., None] * cc
    body = np.clip(body, 0, 1)
    m = mask[..., None]
    return C * (1 - m) + body * m


def finish(C, tmask, out, shadow=True):
    # tile rim light and a soft outer shadow
    inner = blur(tmask, 3)
    rim = np.clip((tmask - inner) * 3, 0, 1)
    topw = np.clip(1 - (YY - 100) / 500, 0, 1)
    C = np.clip(C + (0.10 + 0.25 * topw)[..., None] * rim[..., None], 0, 1)
    sh = shift(blur(tmask, 22), 0, 14) * (0.45 if shadow else 0.0)
    alpha = np.clip(tmask + sh * (1 - tmask), 0, 1)
    rgb = np.where(tmask[..., None] > 0, C, 0)
    # premultiplied shadow colour = black
    out_a = alpha
    rgba = np.dstack([rgb * (tmask / np.maximum(out_a, 1e-6))[..., None], out_a])
    Image.fromarray((np.clip(rgba, 0, 1) * 255).astype(np.uint8), "RGBA").save(out)
