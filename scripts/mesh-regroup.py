#!/usr/bin/env python3
"""Re-split the mouse's buttons by the model's own pieces.

    python3 -I scripts/mesh-regroup.py crates/app/assets/gear/mouse.omkm

`mesh-pack.py --regions` cuts the buttons out of the shell with boxes and
normal limits, which grabbed a slab of the shell and only slivers of the real
buttons. The model already has each button as a piece of its own (a set of
triangles that touch only each other), so this finds those pieces and names
them: the wheel, the left and right buttons, and the two thumb buttons. It
merges every part of the same material back together first, so it can be run
again on its own output. Same file format as mesh-pack.py; stdlib only.

A piece is named by where its centre is, in the file's millimetres (x right,
z toward the back, y up). If one of the five is not found, nothing is written.
"""
import struct
import sys
from collections import defaultdict
from pathlib import Path

# name -> (material, centre box x0 x1 y0 y1 z0 z1, minimum triangles)
PIECES = {
    "wheel": ("Material.002", (0, 12, 10, 36, -46, -18), 300),
    "left": ("Material.002", (-26, 0, 20, 38, -64, 8), 20),
    "right": ("Material.002", (10, 36, 8, 36, -66, -2), 20),
    "forward": ("Material.001", (-33, -17, 18, 36, -35, 9), 10),
    "back": ("Material.001", (-30, -12, 18, 32, 5, 30), 10),
}
# Colours and finish of each name, as mouse-regions.json had them.
LOOK = {
    "wheel": ((0x66, 0x62, 0x5B), 14.0, 0.1),
    "left": ((0x40, 0x44, 0x4B), 22.0, 0.12),
    "right": ((0x40, 0x44, 0x4B), 22.0, 0.12),
    "forward": ((0x66, 0x62, 0x5B), 30.0, 0.2),
    "back": ((0x66, 0x62, 0x5B), 30.0, 0.2),
}
# What the parts of one material were before the buttons were cut out of it.
SHELLS = {"Material.002": ("Material.002", ("wheel", "left", "right")), "Material.001": ("Material.001", ("forward", "back"))}


def read(path):
    b = Path(path).read_bytes()
    at = 0

    def take(n):
        nonlocal at
        s = b[at : at + n]
        if len(s) != n:
            sys.exit("the file is cut short")
        at += n
        return s

    if take(4) != b"OMKM" or struct.unpack("<I", take(4))[0] != 1:
        sys.exit("not an OMKM version 1 file")
    parts = []
    for _ in range(struct.unpack("<I", take(4))[0]):
        name = take(struct.unpack("<H", take(2))[0]).decode()
        gloss, spec = struct.unpack("<ff", take(8))
        nv, ni = struct.unpack("<II", take(8))
        pos = [struct.unpack("<fff", take(12)) for _ in range(nv)]
        nrm = [struct.unpack("<fff", take(12)) for _ in range(nv)]
        col = [tuple(take(3)) for _ in range(nv)]
        idx = struct.unpack("<%dI" % ni, take(4 * ni))
        parts.append({"name": name, "gloss": gloss, "spec": spec, "pos": pos, "nrm": nrm, "col": col, "idx": idx})
    return parts


def write(path, parts):
    out = bytearray(b"OMKM" + struct.pack("<II", 1, len(parts)))
    for p in parts:
        name = p["name"].encode()
        out += struct.pack("<H", len(name)) + name + struct.pack("<ffII", p["gloss"], p["spec"], len(p["pos"]), len(p["idx"]))
        for v in p["pos"]:
            out += struct.pack("<fff", *v)
        for v in p["nrm"]:
            out += struct.pack("<fff", *v)
        for c in p["col"]:
            out += bytes(c)
        out += struct.pack("<%dI" % len(p["idx"]), *p["idx"])
    Path(path).write_bytes(out)


def main(path):
    parts = read(path)
    # Every triangle with its three vertices, under the material it came from.
    tris = defaultdict(list)
    base = {}
    for p in parts:
        mat = next((m for m, (_, cut) in SHELLS.items() if p["name"] == m or p["name"] in cut), None)
        if mat is None:
            sys.exit("unknown part " + p["name"])
        if p["name"] == mat:
            base[mat] = p
        for i in range(0, len(p["idx"]), 3):
            tris[mat].append([(p["pos"][k], p["nrm"][k], p["col"][k]) for k in p["idx"][i : i + 3]])
    if set(base) != set(SHELLS):
        sys.exit("a shell part is missing")

    def weld(v):
        return tuple(round(c, 2) for c in v)

    named = {}
    for mat in SHELLS:
        parent = {}

        def find(x):
            while parent.setdefault(x, x) != x:
                parent[x] = parent[parent[x]]
                x = parent[x]
            return x

        for t in tris[mat]:
            ks = [weld(v[0]) for v in t]
            for k in ks[1:]:
                parent[find(ks[0])] = find(k)
        islands = defaultdict(list)
        for t in tris[mat]:
            islands[find(weld(t[0][0]))].append(t)
        for name, (m, box, least) in PIECES.items():
            if m != mat:
                continue
            found = []
            for ts in islands.values():
                xs, ys, zs = ([v[0][a] for t in ts for v in t] for a in range(3))
                c = [(min(a) + max(a)) / 2 for a in (xs, ys, zs)]
                if len(ts) >= least and all(box[2 * i] <= c[i] <= box[2 * i + 1] for i in range(3)):
                    found.append(ts)
            if not found:
                sys.exit("no piece found for " + name)
            named[name] = [t for ts in found for t in ts]
            ids = {id(t) for t in named[name]}
            islands = {k: v for k, v in islands.items() if not any(id(t) in ids for t in v)}
        named["rest:" + mat] = [t for ts in islands.values() for t in ts]

    def part(name, ts):
        look = LOOK.get(name)
        old = base[name[5:]] if name.startswith("rest:") else None
        gloss, spec = (look[1], look[2]) if look else (old["gloss"], old["spec"])
        pos, nrm, col, idx = [], [], [], []
        for t in ts:
            for p, n, c in t:
                idx.append(len(pos))
                pos.append(p)
                nrm.append(n)
                col.append(look[0] if look else old["col"][0])
        return {"name": name[5:] if old else name, "gloss": gloss, "spec": spec, "pos": pos, "nrm": nrm, "col": col, "idx": idx}

    order = ["Material.002", "wheel", "left", "right", "Material.001", "forward", "back"]
    out = [part(("rest:" + n) if n in SHELLS else n, named[("rest:" + n) if n in SHELLS else n]) for n in order]
    write(path, out)
    for p in out:
        print("%-14s %4d triangles" % (p["name"], len(p["idx"]) // 3))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
