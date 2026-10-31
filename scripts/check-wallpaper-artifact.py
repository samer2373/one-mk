#!/usr/bin/env python3
"""Fail-closed Rust dependency and isolated GStreamer closure gate; no publishing."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

PERMISSIVE = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016", "CC0-1.0", "BSL-1.0", "0BSD", "bzip2-1.0.6", "Apache-2.0 WITH LLVM-exception", "CDLA-Permissive-2.0"}
FORBIDDEN = re.compile(r"ffmpeg|libav(?:codec|format|filter|device|util)|libsw(?:scale|resample)|(?:^|[-_])(?:x264|x265)(?:$|[-_.])|libx26[45]|libmpv|gstlibav|gstx26[45]", re.I)


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


def runtime_components(root, helper):
    result = subprocess.run([str(helper.resolve()), "--runtime-inventory", str(root.absolute())], env={}, timeout=30, capture_output=True, text=True, check=False)
    manifest = json.loads(result.stdout)
    if result.returncode:
        raise ValueError(f"shared runtime policy: {manifest.get('error', 'validation failed')}")
    components = []
    for name, digest in sorted(manifest["files"].items()):
        license_expression = manifest["licenses"][name]
        components.append({"type": "file", "bom-ref": "runtime:" + name, "name": name, "version": "1.28.2" if "gst" in name else "inventory-sha256", "licenses": [{"expression": license_expression}], "hashes": [{"alg": "SHA-256", "content": digest}]})
    return components


def daemon_features(features):
    for feature in re.findall(r'image feature "([^"]+)"', features):
        if feature != "png":
            raise ValueError(f"unexpected daemon image decoder feature: {feature}")
    # Only the 6D-M1 laboratory authority (feature wallpaper-m1-lab) links it.
    if re.search(r'\bone-mk-wall-sync v', features):
        raise ValueError("6D-M1 laboratory presentation authority cannot ship in the daemon")


def renderer_features(features):
    # The release renderer is built with gnome-bridge only; the 6D-M1 laboratory
    # playback (feature m1-lab) links the importer's sandbox launcher.
    if re.search(r'\bone-mk-wall-import v', features) or re.search(r'one-mk-wall feature "m1-lab"', features):
        raise ValueError("6D-M1 laboratory playback cannot ship in the renderer")


def self_test():
    renderer_features('one-mk-wall v0.1.15 (crates/wall-presenter)\n├── zbus feature "tokio"')
    for injected in ('├── one-mk-wall feature "m1-lab"', '│   └── one-mk-wall-import v0.1.15 (crates/wall-import)'):
        try:
            renderer_features(injected)
        except ValueError as error:
            assert "laboratory playback" in str(error), error
        else:
            raise AssertionError("renderer laboratory playback injection passed")
    daemon_features('image feature "png"')
    try:
        daemon_features('├── one-mk-wall-sync feature "default"\n│   └── one-mk-wall-sync v0.1.15 (crates/wall)')
    except ValueError as error:
        assert "laboratory" in str(error), error
    else:
        raise AssertionError("daemon laboratory authority injection passed")
    for feature in ("gif", "webp", "avif", "jpeg", "tiff", "exr", "default", "bmp"):
        try:
            daemon_features(f'image feature "{feature}"')
        except ValueError as error:
            assert str(error) == f"unexpected daemon image decoder feature: {feature}", error
        else:
            raise AssertionError(f"daemon decoder injection passed: {feature}")
    for expression in ("GPL-3.0-only", "AGPL-3.0-only", "GPL-2.0-only AND MIT", "LicenseRef-Unknown", "", "MIT OR", "LGPL-2.1-or-later"):
        assert not allowed(expression, PERMISSIVE), expression
    for expression in ("MIT OR Apache-2.0", "BSD-3-Clause AND MIT", "(MIT OR GPL-3.0-only) AND Apache-2.0"):
        assert allowed(expression, PERMISSIVE), expression
    for name in ("ffmpeg-sys-next", "libavcodec.so.62", "libgstlibav.so", "libgstx264.so", "libx265.so.1", "libmpv.so"):
        assert FORBIDDEN.search(name), name
    bad = {"packages": [{"id": "bad", "name": "ffmpeg-sys-next", "version": "1", "license": "MIT"}], "resolve": {"nodes": [{"id": "bad", "dependencies": []}]}}
    try:
        cargo_components(bad, "ffmpeg-sys-next")
    except ValueError as error:
        assert str(error) == "forbidden/unknown Cargo dependency ffmpeg-sys-next: MIT", error
    else:
        raise AssertionError("negative dependency injection passed")
    print("PASS: injected FFmpeg Cargo dependency rejected for intended reason; GPL/invalid SPDX checks passed. Runtime policy regressions live in the Rust helper tests.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--metadata", type=Path)
    parser.add_argument("--package", default="one-mk-wall-import")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--sbom", type=Path)
    parser.add_argument("--helper", type=Path)
    parser.add_argument("--daemon-features", type=Path)
    parser.add_argument("--renderer-features", type=Path,
                        help="cargo tree --locked -p one-mk-wall --features gnome-bridge -e features")
    arguments = parser.parse_args()
    if arguments.self_test:
        self_test()
        return
    if not arguments.metadata:
        parser.error("--metadata required (cargo metadata --locked --format-version 1)")
    components = cargo_components(json.loads(arguments.metadata.read_text()), arguments.package)
    if not arguments.daemon_features:
        parser.error("--daemon-features required (cargo tree --locked -p one-mk -e features)")
    daemon_features(arguments.daemon_features.read_text())
    if arguments.renderer_features:
        renderer_features(arguments.renderer_features.read_text())
    if arguments.helper:
        info = json.loads(subprocess.check_output([str(arguments.helper.resolve()), "--build-info"], env={}, timeout=5))
        if info.get("schema") != 1 or info.get("test_faults") is not False:
            raise ValueError("test-faults/unknown helper build cannot ship")
        components.append({"type": "file", "bom-ref": "helper", "name": arguments.helper.name, "hashes": [{"alg": "SHA-256", "content": hashlib.sha256(arguments.helper.read_bytes()).hexdigest()}]})
    if arguments.runtime:
        if not arguments.helper:
            parser.error("--runtime requires --helper for shared runtime validation")
        components.extend(runtime_components(arguments.runtime, arguments.helper))
    if arguments.sbom:
        arguments.sbom.write_text(json.dumps({"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1, "metadata": {"component": {"type": "application", "name": arguments.package}, "properties": [{"name": "one-mk:publication", "value": "dry-run; not a qualification or licence/source audit"}]}, "components": sorted(components, key=lambda item: item["bom-ref"])}, indent=2) + "\n")
    print(f"PASS: {len(components)} dependency/closure records; dry-run only, no artifact published")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"wallpaper artifact gate FAILED: {error}", file=sys.stderr)
        sys.exit(1)
