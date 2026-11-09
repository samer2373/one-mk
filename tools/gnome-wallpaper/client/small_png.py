#!/usr/bin/env python3
"""Spike helper: shrink a screenshot to a small palette PNG. small_png.py IN OUT [WIDTH]"""
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
w = int(sys.argv[3]) if len(sys.argv) > 3 else 640
im = im.resize((w, round(im.height * w / im.width)), Image.LANCZOS)
im.quantize(colors=96, method=Image.Quantize.MEDIANCUT).save(sys.argv[2], optimize=True)
