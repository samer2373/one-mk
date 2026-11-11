#!/usr/bin/env python3
"""Pack the app's 3D mouse (crates/app/assets/gear/mouse.omkm) into a small
classic script for the page: site/js/mouse-mesh.js sets window.OMK_MOUSE.

The model is "Logitech G502 x lightspeed" by Okopchi, CC BY 4.0, changed for
oneMK (see crates/app/assets/gear/README.md); the page credits it in its footer.

Vertices are welded, positions quantised to 16 bits over the bounding box,
normals to 8 bits, and each vertex keeps its part (0 shell, 1 buttons,
2 wheel, 3 trim) so the page colours it like the app does, per theme.
    python3 -I site/tools/pack-mouse.py crates/app/assets/gear/mouse.omkm site/js/mouse-mesh.js
"""
import base64, struct, sys

GROUP = {"left": 1, "right": 1, "wheel": 2, "Material.001": 3, "forward": 3, "back": 3}

def main(src, out):
    d = open(src, "rb").read()
    magic, _ver, parts = struct.unpack_from("<4sII", d, 0)
    assert magic == b"OMKM"
    o, verts = 12, []
    for _ in range(parts):
        (n,) = struct.unpack_from("<H", d, o); o += 2
        name = d[o:o + n].decode(); o += n
        _g, _s, nv, ni = struct.unpack_from("<ffII", d, o); o += 16
        pos = struct.unpack_from(f"<{nv*3}f", d, o); o += nv * 12
        nor = struct.unpack_from(f"<{nv*3}f", d, o); o += nv * 12
        o += nv * 3
        idx = struct.unpack_from(f"<{ni}I", d, o); o += ni * 4
        g = GROUP.get(name, 0)
        for i in idx:
            verts.append((pos[i*3:i*3+3], nor[i*3:i*3+3], g))
    lo = [min(v[0][k] for v in verts) for k in range(3)]
    hi = [max(v[0][k] for v in verts) for k in range(3)]
    q = lambda x, k: round((x - lo[k]) / (hi[k] - lo[k]) * 65535)
    weld, P, N, G, I = {}, [], [], [], []
    for p, n, g in verts:
        key = (tuple(q(p[k], k) for k in range(3)), tuple(max(-127, min(127, round(c * 127))) for c in n), g)
        if key not in weld:
            weld[key] = len(G); P += key[0]; N += key[1]; G.append(g)
        I.append(weld[key])
    nv = len(G)
    blob = struct.pack(f"<{len(P)}H", *P) + struct.pack(f"<{len(N)}b", *N) + bytes(G)
    blob += b"\0" * (-len(blob) % 2) + struct.pack(f"<{len(I)}H", *I)
    js = ("/* The 3D mouse: \"Logitech G502 x lightspeed\" by Okopchi, CC BY 4.0\n"
          "   (https://sketchfab.com/3d-models/logitech-g502-x-lightspeed-cda7107cc1444b4788d747f0361f7d40),\n"
          "   changed for oneMK: logo and feet left out, normals smoothed, shell split into its buttons.\n"
          "   Packed by site/tools/pack-mouse.py. */\n"
          f"window.OMK_MOUSE={{v:{nv},i:{len(I)},lo:{[round(x,3) for x in lo]},hi:{[round(x,3) for x in hi]},"
          f"b:\"{base64.b64encode(blob).decode()}\"}};\n")
    open(out, "w").write(js)
    print(f"{out}: {nv} vertices, {len(I)//3} triangles, {len(js)} bytes")

main(*sys.argv[1:])
