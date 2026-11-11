#!/usr/bin/env python3
"""Expands the image markers in index.html in place (safe to run again):
  <!--img shots/desk "alt" sizes="..."--> ... <!--/img-->       light and dark
  <!--img-fixed crops/x light "alt" sizes="..."--> ... <!--/img-->  one theme
into <picture> elements with AVIF, WebP and PNG sources from assets/img.
    python3 -I site/tools/expand-images.py site/index.html"""
import re, sys
from pathlib import Path
from PIL import Image

page = Path(sys.argv[1]); root = page.parent
MARK = re.compile(r'<!--(img|img-fixed) (\S+)(?: (light|dark))? "([^"]*)" sizes="([^"]*)"-->.*?<!--/img-->', re.S)

def picture(path, theme, alt, sizes, cls):
    folder, name = path.split("/")
    d = root / "assets/img" / folder
    stem = f"{name}-{theme}"
    widths = sorted(int(p.stem.rsplit("-", 1)[1]) for p in d.glob(f"{stem}-*.webp"))
    big = Image.open(d / f"{stem}-{widths[-1]}.webp")
    w, h = big.size
    rel = f"assets/img/{folder}/{stem}"
    srcs = lambda ext: ", ".join(f"{rel}-{x}.{ext} {x}w" for x in widths if (d / f"{stem}-{x}.{ext}").exists())
    out = f'<picture{cls}>'
    if (d / f"{stem}-{widths[0]}.avif").exists():
        out += f'<source type="image/avif" srcset="{srcs("avif")}" sizes="{sizes}">'
    out += f'<source type="image/webp" srcset="{srcs("webp")}" sizes="{sizes}">'
    out += f'<img src="{rel}.png" width="{w}" height="{h}" alt="{alt}" loading="lazy" decoding="async"></picture>'
    return out

def expand(m):
    kind, path, theme, alt, sizes = m.groups()
    head = m.group(0).split("-->", 1)[0] + "-->"
    if kind == "img-fixed":
        body = picture(path, theme, alt, sizes, "")
    else:
        body = picture(path, "light", alt, sizes, ' class="t-light"') + picture(path, "dark", alt, sizes, ' class="t-dark"')
    return head + body + "<!--/img-->"

text = page.read_text()
text, n = MARK.subn(expand, text)
page.write_text(text)
print(f"{n} images expanded")
