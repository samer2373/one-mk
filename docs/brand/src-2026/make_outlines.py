#!/usr/bin/env python3
"""Regenerate wordmark-outlines.json from Geist's variable font (SIL OFL 1.1).

    npm pack geist@1.9.0 && tar -xzf geist-1.9.0.tgz
    python3 make_outlines.py package/dist/fonts/geist-sans/Geist-Variable.woff2

Needs fontTools and brotli (pip install fonttools brotli)."""
import json, os, sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.boundsPen import BoundsPen

VF = sys.argv[1]
out = {"source": "Geist (SIL OFL 1.1, Vercel), npm package geist 1.9.0, dist/fonts/geist-sans/Geist-Variable.woff2",
       "units_per_em": 1000, "x_height": 534, "cap_height": 710,
       "note": "y is up (font units); instanced at the weights given", "glyphs": {}}
for w, chars in ((600, "one"), (660, "MK")):
    f = instancer.instantiateVariableFont(TTFont(VF), {"wght": w})
    gs, cm = f.getGlyphSet(), f.getBestCmap()
    for ch in chars:
        g = cm[ord(ch)]
        pen = SVGPathPen(gs, ntos=lambda v: ("%.1f" % v).rstrip("0").rstrip("."))
        gs[g].draw(pen)
        bp = BoundsPen(gs); gs[g].draw(bp)
        adv, lsb = f["hmtx"][g]
        out["glyphs"][ch] = {"weight": w, "advance": adv, "lsb": lsb, "bounds": [round(v, 1) for v in bp.bounds],
                             "d": pen.getCommands()}
json.dump(out, open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "wordmark-outlines.json"), "w"), indent=1)
