#!/usr/bin/env python3
"""Fail-closed Rust dependency and isolated GStreamer closure gate; no publishing."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

PERMISSIVE = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016", "CC0-1.0", "BSL-1.0", "0BSD", "bzip2-1.0.6", "Apache-2.0 WITH LLVM-exception", "CDLA-Permissive-2.0"}
DYNAMIC = PERMISSIVE | {"LGPL-2.0-or-later", "LGPL-2.1-or-later", "GPL-3.0-or-later WITH GCC-exception-3.1"}
FORBIDDEN = re.compile(r"ffmpeg|libav(?:codec|format|filter|device|util)|libsw(?:scale|resample)|(?:^|[-_])(?:x264|x265)(?:$|[-_.])|libx26[45]|libmpv|gstlibav|gstx26[45]", re.I)
PLUGINS = {f"libgst{name}.so" for name in ("coreelements", "matroska", "vpx", "aom", "videoconvertscale")}


def normalized(expression):
    return {"MIT/Apache-2.0": "MIT OR Apache-2.0", "Apache-2.0/MIT": "Apache-2.0 OR MIT"}.get(expression, expression)


def allowed(expression, choices):
    expression = {"MIT/Apache-2.0": "MIT OR Apache-2.0", "Apache-2.0/MIT": "Apache-2.0 OR MIT"}.get(expression, expression)
    tokens = re.findall(r"\(|\)|\bAND\b|\bOR\b|\bWITH\b|[A-Za-z0-9.+-]+", expression or "")
    if "".join(tokens) != re.sub(r"\s", "", expression or ""):
        return False
    position = 0

    def atom():
        nonlocal position
        if position >= len(tokens):
            raise ValueError("missing SPDX operand")
        token = tokens[position]
        position += 1
        if token == "(":
            result = disjunction()
            if position >= len(tokens) or tokens[position] != ")":
                raise ValueError("unclosed SPDX expression")
            position += 1
            return result
        if token in {"AND", "OR", "WITH", ")"}:
            raise ValueError("invalid SPDX operand")
        if position < len(tokens) and tokens[position] == "WITH":
            position += 1
            if position >= len(tokens):
                raise ValueError("missing SPDX exception")
            token += " WITH " + tokens[position]
            position += 1
        return token in choices

    def conjunction():
        nonlocal position
        result = atom()
        while position < len(tokens) and tokens[position] == "AND":
            position += 1
            other = atom()
            result = result and other
        return result

    def disjunction():
        nonlocal position
        result = conjunction()
        while position < len(tokens) and tokens[position] == "OR":
            position += 1
            other = conjunction()
            result = result or other
        return result

    try:
        result = disjunction()
        return result and position == len(tokens)
    except ValueError:
        return False


def cargo_components(metadata, package):
    packages = {item["id"]: item for item in metadata["packages"]}
    roots = [item["id"] for item in packages.values() if item["name"] == package]
    if len(roots) != 1 or not metadata.get("resolve"):
        raise ValueError("exact package and resolved Cargo graph required")
    nodes = {item["id"]: item for item in metadata["resolve"]["nodes"]}
    pending, seen, components = roots[:], set(), []
    while pending:
        identity = pending.pop()
        if identity in seen:
            continue
        seen.add(identity)
        item = packages[identity]
        if FORBIDDEN.search(item["name"]) or not allowed(item.get("license"), PERMISSIVE):
            raise ValueError(f"forbidden/unknown Cargo dependency {item['name']}: {item.get('license')}")
        components.append({"type": "library", "bom-ref": identity, "name": item["name"], "version": item["version"], "licenses": [{"expression": normalized(item["license"])}], "purl": f"pkg:cargo/{item['name']}@{item['version']}"})
        pending.extend(nodes[identity]["dependencies"])
    return components


def runtime_components(root):
    manifest = json.loads((root / "runtime.json").read_text())
    if manifest.get("schema") != 1 or manifest.get("gstreamer") != "1.28.2":
        raise ValueError("runtime pin mismatch")
    actual = {}
    for path in root.rglob("*"):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError(f"symlink/device forbidden: {path}")
        if path.stat().st_mode & 0o022:
            raise ValueError(f"writable runtime member: {path}")
        if path.is_file() and path != root / "runtime.json":
            actual[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    if not actual or actual != manifest["files"] or set(actual) != set(manifest["licenses"]):
        raise ValueError("unlisted, modified or unlicensed runtime member")
    plugins = {Path(name).name for name in actual if name.startswith("plugins/")}
    if plugins != PLUGINS:
        raise ValueError(f"plugin closure must exactly match the explicit allowlist, got {sorted(plugins)}")
    names = {Path(name).name for name in actual}
    components = []
    for name, digest in sorted(actual.items()):
        license_expression = manifest["licenses"][name]
        if FORBIDDEN.search(name) or not allowed(license_expression, DYNAMIC):
            raise ValueError(f"forbidden/unknown runtime library: {name}: {license_expression}")
        path = root / name
        if path.read_bytes()[:4] == b"\x7fELF":
            dynamic = subprocess.check_output(["readelf", "-d", str(path)], text=True, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"})
            for dependency in re.findall(r"\(NEEDED\).*?\[(.*?)\]", dynamic):
                if FORBIDDEN.search(dependency) or dependency not in names:
                    raise ValueError(f"forbidden/unbundled dynamic edge: {name} -> {dependency}")
            if "RPATH" in dynamic or "RUNPATH" in dynamic:
                raise ValueError(f"unreviewed ELF loader search path: {name}")
        components.append({"type": "file", "bom-ref": "runtime:" + name, "name": name, "version": "1.28.2" if "gst" in name else "inventory-sha256", "licenses": [{"expression": license_expression}], "hashes": [{"alg": "SHA-256", "content": digest}]})
    return components


def self_test():
    for expression in ("GPL-3.0-only", "AGPL-3.0-only", "GPL-2.0-only AND MIT", "LicenseRef-Unknown", "", "MIT OR", "LGPL-2.1-or-later"):
        assert not allowed(expression, PERMISSIVE), expression
    for expression in ("MIT OR Apache-2.0", "BSD-3-Clause AND MIT", "(MIT OR GPL-3.0-only) AND Apache-2.0"):
        assert allowed(expression, PERMISSIVE), expression
    for name in ("ffmpeg-sys-next", "libavcodec.so.62", "libgstlibav.so", "libgstx264.so", "libx265.so.1", "libmpv.so"):
        assert FORBIDDEN.search(name), name
    bad = {"packages": [{"id": "bad", "name": "ffmpeg-sys-next", "version": "1", "license": "MIT"}], "resolve": {"nodes": [{"id": "bad", "dependencies": []}]}}
    try:
        cargo_components(bad, "ffmpeg-sys-next")
    except ValueError:
        pass
    else:
        raise AssertionError("negative dependency injection passed")
    with tempfile.TemporaryDirectory(prefix="wallpaper-gate-") as directory:
        root = Path(directory)
        files = {"plugins/" + name: b"fixture" for name in PLUGINS}
        files["lib/libavcodec.so.62"] = b"forbidden negative artifact"
        for name, content in files.items():
            path = root / name
            path.parent.mkdir(exist_ok=True)
            path.parent.chmod(0o700)
            path.write_bytes(content)
            path.chmod(0o600)
        (root / "runtime.json").write_text(json.dumps({"schema": 1, "gstreamer": "1.28.2", "files": {name: hashlib.sha256(content).hexdigest() for name, content in files.items()}, "licenses": {name: "MIT" for name in files}}))
        (root / "runtime.json").chmod(0o600)
        try:
            runtime_components(root)
        except ValueError as error:
            assert "forbidden/unknown runtime library" in str(error), error
        else:
            raise AssertionError("negative FFmpeg artifact injection passed")
    print("PASS: injected FFmpeg Cargo dependency and on-disk runtime artifact rejected; GPL/invalid SPDX/plugin checks passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--metadata", type=Path)
    parser.add_argument("--package", default="one-mk-wall-import")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--sbom", type=Path)
    parser.add_argument("--helper", type=Path)
    arguments = parser.parse_args()
    if arguments.self_test:
        self_test()
        return
    if not arguments.metadata:
        parser.error("--metadata required (cargo metadata --locked --format-version 1)")
    components = cargo_components(json.loads(arguments.metadata.read_text()), arguments.package)
    if arguments.helper:
        info = json.loads(subprocess.check_output([str(arguments.helper.resolve()), "--build-info"], env={}, timeout=5))
        if info.get("schema") != 1 or info.get("test_faults") is not False:
            raise ValueError("test-faults/unknown helper build cannot ship")
        components.append({"type": "file", "bom-ref": "helper", "name": arguments.helper.name, "hashes": [{"alg": "SHA-256", "content": hashlib.sha256(arguments.helper.read_bytes()).hexdigest()}]})
    if arguments.runtime:
        components.extend(runtime_components(arguments.runtime))
    if arguments.sbom:
        arguments.sbom.write_text(json.dumps({"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1, "metadata": {"component": {"type": "application", "name": arguments.package}, "properties": [{"name": "one-mk:publication", "value": "dry-run; not a qualification or licence/source audit"}]}, "components": sorted(components, key=lambda item: item["bom-ref"])}, indent=2) + "\n")
    print(f"PASS: {len(components)} dependency/closure records; dry-run only, no artifact published")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"wallpaper artifact gate FAILED: {error}", file=sys.stderr)
        sys.exit(1)
