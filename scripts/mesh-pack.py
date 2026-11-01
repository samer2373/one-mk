#!/usr/bin/env python3
"""Pack a glTF model (.glb or .gltf) into the small mesh file the window draws
(crates/app/src/gear.rs, `Model`). Node transforms are applied, textures are
baked into vertex colours, the model is centred on its footprint, stood on
y = 0 and scaled so its longest side is SIZE millimetres.

    scripts/mesh-pack.py in.glb out.omkm --size 125 [--yaw 180] [--drop REGEX]
                         [--list]

--list prints each part (mesh and material name, triangles) and writes nothing;
--drop leaves out parts whose "mesh/material" name matches, e.g. a logo.
--regions takes a JSON list, applied after scaling, first match wins:
  [{"name": "left", "from": "Material.002", "box": [x0, x1, y0, y1, z0, z1],
    "front_of": [xa, za, xb, zb], "up": 0.3, "color": "#34373D",
    "gloss": 20, "spec": 0.1}, ...]
A triangle joins a region when its material matches "from", its centre is in
"box", in front of (smaller z than) the line "front_of", and its normal's y
is at least "up". "rest:MATERIAL" entries colour what is left of a material.

Format (little-endian): b"OMKM", u32 version 1, u32 parts; per part: u16 name
length, name (UTF-8), f32 gloss, f32 specular, u32 vertices, u32 indices,
then f32 x,y,z per vertex, f32 normal x,y,z per vertex, u8 r,g,b per vertex,
u32 indices.
"""
import argparse
import base64
import io
import json
import math
import re
import struct
import sys
from pathlib import Path

from PIL import Image

COMPONENTS = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}
FORMATS = {5120: "b", 5121: "B", 5122: "h", 5123: "H", 5125: "I", 5126: "f"}


def load(path):
    data = Path(path).read_bytes()
    if data[:4] == b"glTF":
        _, _, length = struct.unpack_from("<4sII", data, 0)
        offset, gltf, binary = 12, None, b""
        while offset < length:
            size, kind = struct.unpack_from("<II", data, offset)
            chunk = data[offset + 8: offset + 8 + size]
            if kind == 0x4E4F534A:
                gltf = json.loads(chunk)
            elif kind == 0x004E4942:
                binary = chunk
            offset += 8 + size
        buffers = [binary]
    else:
        gltf = json.loads(data)
        buffers = []
        for b in gltf.get("buffers", []):
            uri = b["uri"]
            if uri.startswith("data:"):
                buffers.append(base64.b64decode(uri.split(",", 1)[1]))
            else:
                buffers.append((Path(path).parent / uri).read_bytes())
    return gltf, buffers


def accessor(gltf, buffers, index):
    a = gltf["accessors"][index]
    view = gltf["bufferViews"][a["bufferView"]]
    buf = buffers[view.get("buffer", 0)]
    n = COMPONENTS[a["type"]]
    fmt = FORMATS[a["componentType"]]
    item = struct.calcsize("<" + fmt) * n
    stride = view.get("byteStride", item)
    start = view.get("byteOffset", 0) + a.get("byteOffset", 0)
    out = []
    for i in range(a["count"]):
        v = struct.unpack_from("<" + fmt * n, buf, start + i * stride)
        if a.get("normalized") and fmt != "f":
            top = float((1 << (8 * struct.calcsize(fmt) - (1 if fmt.islower() else 0))) - 1)
            v = tuple(max(x / top, -1.0) for x in v)
        out.append(v)
    return out


def matmul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def node_matrix(node):
    if "matrix" in node:
        m = node["matrix"]
        return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
    t = node.get("translation", [0, 0, 0])
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    s = node.get("scale", [1, 1, 1])
    r = [
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ]
    return [[r[i][0] * s[0], r[i][1] * s[1], r[i][2] * s[2], t[i]] for i in range(3)] + [[0, 0, 0, 1]]


def normal_matrix(m):
    """The inverse transpose of `m`'s 3x3 part: normals stay at right angles
    to the surface under uneven scale or shear."""
    a = [row[:3] for row in m[:3]]
    det = (a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
           - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
           + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]))
    if abs(det) < 1e-12:
        sys.exit("a node flattens its mesh (its transform can't be inverted)")
    cof = [[(a[(i + 1) % 3][(j + 1) % 3] * a[(i + 2) % 3][(j + 2) % 3] - a[(i + 1) % 3][(j + 2) % 3] * a[(i + 2) % 3][(j + 1) % 3]) for j in range(3)] for i in range(3)]
    # inverse = adjugate / det = cofactor^T / det; its transpose is cofactor / det.
    return [[cof[i][j] / det for j in range(3)] for i in range(3)]


def apply(m, p, w=1.0):
    return tuple(m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3] * w for i in range(3))


def image_of(gltf, buffers, path, tex_index, cache):
    if tex_index in cache:
        return cache[tex_index]
    src = gltf["textures"][tex_index]["source"]
    img = gltf["images"][src]
    if "bufferView" in img:
        view = gltf["bufferViews"][img["bufferView"]]
        raw = buffers[view.get("buffer", 0)][view.get("byteOffset", 0): view.get("byteOffset", 0) + view["byteLength"]]
    elif img["uri"].startswith("data:"):
        raw = base64.b64decode(img["uri"].split(",", 1)[1])
    else:
        raw = (Path(path).parent / img["uri"]).read_bytes()
    im = Image.open(io.BytesIO(raw)).convert("RGB")
    cache[tex_index] = im
    return im


def smoothed(part, degrees):
    """Gives every triangle corner the average normal of the faces round
    that point which turn less than `degrees` from its own face."""
    name, gloss, spec, pos, nm, cols, idx = part
    limit = math.cos(math.radians(degrees))
    faces = []
    for t in range(0, len(idx) - 2, 3):
        a, b, c = (pos[idx[t + k]] for k in range(3))
        u = [b[i] - a[i] for i in range(3)]
        v = [c[i] - a[i] for i in range(3)]
        n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
        # Keep the model's facing: flip to agree with its own normals.
        if sum(n[i] * sum(nm[idx[t + k]][i] for k in range(3)) for i in range(3)) < 0:
            n = [-x for x in n]
        faces.append(n)
    unit = [[x / (math.sqrt(sum(y * y for y in n)) or 1.0) for x in n] for n in faces]
    key = lambda p: (round(p[0], 2), round(p[1], 2), round(p[2], 2))
    around = {}
    for f in range(len(faces)):
        for k in range(3):
            around.setdefault(key(pos[idx[f * 3 + k]]), []).append(f)
    P, N, C, I = [], [], [], []
    for f in range(len(faces)):
        for k in range(3):
            i = idx[f * 3 + k]
            acc = [0.0, 0.0, 0.0]
            for g in around[key(pos[i])]:
                if sum(unit[f][j] * unit[g][j] for j in range(3)) >= limit:
                    acc = [acc[j] + faces[g][j] for j in range(3)]
            ln = math.sqrt(sum(x * x for x in acc)) or 1.0
            P.append(pos[i])
            N.append(tuple(x / ln for x in acc))
            C.append(cols[i])
            I.append(len(P) - 1)
    return (name, gloss, spec, P, N, C, I)


def split(parts, regions):
    """Cuts parts into named regions by where each triangle is."""
    def hexcol(h):
        h = h.lstrip("#")
        return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))

    def inside(r, c, n):
        x0, x1, y0, y1, z0, z1 = r.get("box", [-1e9, 1e9, -1e9, 1e9, -1e9, 1e9])
        if not (x0 <= c[0] <= x1 and y0 <= c[1] <= y1 and z0 <= c[2] <= z1):
            return False
        if "front_of" in r:
            xa, za, xb, zb = r["front_of"]
            t = (c[0] - xa) / ((xb - xa) or 1e-9)
            if c[2] > za + (zb - za) * max(0.0, min(1.0, t)):
                return False
        return n[1] >= r.get("up", -2.0)

    buckets = {}
    order = []
    for name, gloss, spec, pos, nm, cols, idx in parts:
        material = name.split("/")[-1]
        for t in range(0, len(idx) - 2, 3):
            tri = idx[t:t + 3]
            c = [sum(pos[i][k] for i in tri) / 3 for k in range(3)]
            n = [sum(nm[i][k] for i in tri) for k in range(3)]
            ln = sum(v * v for v in n) ** 0.5 or 1.0
            n = [v / ln for v in n]
            hit = next((r for r in regions if "box" in r and r.get("from", material) == material and inside(r, c, n)), None)
            if hit is None:
                hit = next((r for r in regions if r.get("name") == "rest:" + material), {"name": name})
            key = hit["name"].removeprefix("rest:") if hit["name"].startswith("rest:") else hit["name"]
            if key not in buckets:
                buckets[key] = (hit, [], [], [], [], {})
                order.append(key)
            r, P, N, C, I, remap = buckets[key]
            for i in tri:
                if (id(pos), i) not in remap:
                    remap[(id(pos), i)] = len(P)
                    P.append(pos[i])
                    N.append(nm[i])
                    C.append(hexcol(r["color"]) if "color" in r else cols[i])
                I.append(remap[(id(pos), i)])
    out = []
    for key in order:
        r, P, N, C, I, _ = buckets[key]
        out.append((key, r.get("gloss", 24.0), r.get("spec", 0.18), P, N, C, I))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("model")
    ap.add_argument("out", nargs="?")
    ap.add_argument("--size", type=float, default=120.0, help="longest side in mm")
    ap.add_argument("--yaw", type=float, default=0.0, help="turn round the vertical, degrees")
    ap.add_argument("--pitch", type=float, default=0.0, help="turn round x first, degrees (for Z-up models use -90)")
    ap.add_argument("--drop", default=None, help="regex on 'mesh/material' names to leave out")
    ap.add_argument("--gloss", type=float, default=24.0)
    ap.add_argument("--spec", type=float, default=0.18)
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--smooth", type=float, default=0.0, help="smooth normals across edges sharper than this many degrees apart (0: keep the model's)")
    ap.add_argument("--regions", default=None, help="JSON: split parts into named regions and colour them")
    args = ap.parse_args()

    gltf, buffers = load(args.model)
    materials = gltf.get("materials", [])
    textures = {}
    parts = []

    def visit(index, parent):
        node = gltf["nodes"][index]
        m = matmul(parent, node_matrix(node))
        if "mesh" in node:
            mesh = gltf["meshes"][node["mesh"]]
            for prim in mesh["primitives"]:
                if prim.get("mode", 4) != 4:
                    continue
                mat = materials[prim["material"]] if "material" in prim else {}
                name = f"{mesh.get('name', 'mesh')}/{mat.get('name', 'material')}"
                parts.append((name, m, prim, mat))
        for c in node.get("children", []):
            visit(c, m)

    ident = [[1 if i == j else 0 for j in range(4)] for i in range(4)]
    scene = gltf["scenes"][gltf.get("scene", 0)]
    for n in scene["nodes"]:
        visit(n, ident)

    if args.list:
        for name, _, prim, _ in parts:
            count = gltf["accessors"][prim["indices"]]["count"] // 3 if "indices" in prim else gltf["accessors"][prim["attributes"]["POSITION"]]["count"] // 3
            print(f"{count:7d} triangles  {name}")
        return

    drop = re.compile(args.drop) if args.drop else None
    out_parts = []
    pr, yr = math.radians(args.pitch), math.radians(args.yaw)
    turn = matmul(
        [[math.cos(yr), 0, math.sin(yr), 0], [0, 1, 0, 0], [-math.sin(yr), 0, math.cos(yr), 0], [0, 0, 0, 1]],
        [[1, 0, 0, 0], [0, math.cos(pr), -math.sin(pr), 0], [0, math.sin(pr), math.cos(pr), 0], [0, 0, 0, 1]],
    )
    for name, m, prim, mat in parts:
        if drop and drop.search(name):
            continue
        m = matmul(turn, m)
        attrs = prim["attributes"]
        pos = [apply(m, p) for p in accessor(gltf, buffers, attrs["POSITION"])]
        idx = [i[0] for i in accessor(gltf, buffers, prim["indices"])] if "indices" in prim else list(range(len(pos)))
        # Normals: the model's own, else smooth ones from the faces.
        if "NORMAL" in attrs:
            nmat = normal_matrix(m)
            nm = [tuple(sum(nmat[i][j] * n[j] for j in range(3)) for i in range(3)) for n in accessor(gltf, buffers, attrs["NORMAL"])]
        else:
            nm = [[0.0, 0.0, 0.0] for _ in pos]
            for t in range(0, len(idx) - 2, 3):
                a, b, c = (pos[idx[t + k]] for k in range(3))
                u = [b[i] - a[i] for i in range(3)]
                v = [c[i] - a[i] for i in range(3)]
                n = (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0])
                for k in range(3):
                    for i in range(3):
                        nm[idx[t + k]][i] += n[i]
        nm = [tuple(x / (math.sqrt(sum(y * y for y in n)) or 1.0) for x in n) for n in nm]
        pbr = mat.get("pbrMetallicRoughness", {})
        sg = mat.get("extensions", {}).get("KHR_materials_pbrSpecularGlossiness", {})
        factor = pbr.get("baseColorFactor", sg.get("diffuseFactor", [1, 1, 1, 1]))
        lin = lambda c: c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
        enc = lambda c: 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055
        cols = []
        tex = pbr.get("baseColorTexture")
        uvs = accessor(gltf, buffers, attrs[f"TEXCOORD_{tex.get('texCoord', 0)}"]) if tex and f"TEXCOORD_{tex.get('texCoord', 0)}" in attrs else None
        img = image_of(gltf, buffers, args.model, tex["index"], textures) if uvs else None
        vcol = accessor(gltf, buffers, attrs["COLOR_0"]) if "COLOR_0" in attrs else None
        for i in range(len(pos)):
            c = list(factor[:3])
            if img is not None:
                u, v = uvs[i]
                px = img.getpixel((int((u % 1.0) * (img.width - 1)), int((v % 1.0) * (img.height - 1))))
                c = [c[k] * lin(px[k] / 255.0) for k in range(3)]
            if vcol is not None:
                c = [c[k] * vcol[i][k] for k in range(3)]
            cols.append(tuple(max(0, min(255, round(enc(max(0.0, x)) * 255))) for x in c))
        rough = pbr.get("roughnessFactor", 1.0)
        gloss = args.gloss * (1.5 - rough)
        out_parts.append((name, gloss, args.spec * (1.2 - 0.8 * rough), pos, nm, cols, idx))

    if not args.out:
        sys.exit("give an output file, or --list")
    xs = [p[0] for part in out_parts for p in part[3]]
    ys = [p[1] for part in out_parts for p in part[3]]
    zs = [p[2] for part in out_parts for p in part[3]]
    k = args.size / max(max(xs) - min(xs), max(ys) - min(ys), max(zs) - min(zs))
    cx, cz, y0 = (max(xs) + min(xs)) / 2, (max(zs) + min(zs)) / 2, min(ys)
    out_parts = [(n, g, sp, [((p[0] - cx) * k, (p[1] - y0) * k, (p[2] - cz) * k) for p in pos], nm, cols, idx) for n, g, sp, pos, nm, cols, idx in out_parts]
    if args.smooth > 0:
        out_parts = [smoothed(part, args.smooth) for part in out_parts]
    if args.regions:
        out_parts = split(out_parts, json.loads(Path(args.regions).read_text()))
    blob = bytearray(b"OMKM" + struct.pack("<II", 1, len(out_parts)))
    total = 0
    for name, gloss, spec, pos, nm, cols, idx in out_parts:
        nb = name.encode()[:200]
        blob += struct.pack("<H", len(nb)) + nb + struct.pack("<ffII", gloss, spec, len(pos), len(idx))
        for p in pos:
            blob += struct.pack("<fff", *p)
        for n in nm:
            blob += struct.pack("<fff", *n)
        for c in cols:
            blob += bytes(c)
        blob += struct.pack(f"<{len(idx)}I", *idx)
        total += len(idx) // 3
    Path(args.out).write_bytes(blob)
    print(f"{args.out}: {len(out_parts)} parts, {total} triangles, {len(blob)} bytes")


if __name__ == "__main__":
    main()
