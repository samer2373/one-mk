#!/usr/bin/env python3
"""Build an isolated HOST SNAPSHOT for tests, NOT a redistributable runtime."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "HOME": "/nonexistent"}
PLUGINS = ("coreelements", "matroska", "vpx", "aom", "videoconvertscale")


def license_for(name):
    if name.startswith("libz.so"):
        return "Zlib"
    if name.startswith("libbz2"):
        return "bzip2-1.0.6"
    if name.startswith("libdrm"):
        return "MIT"
    if name.startswith(("libgcc_s", "libstdc++", "libatomic")):
        return "GPL-3.0-or-later WITH GCC-exception-3.1"
    if name.startswith(("libffi", "libunwind")):
        return "MIT"
    if name.startswith(("libvpx", "libpcre2")):
        return "BSD-3-Clause"
    if name.startswith(("libaom", "liborc")):
        return "BSD-2-Clause"
    if name.startswith(("libgst", "gst-launch")):
        return "LGPL-2.0-or-later"
    if name.startswith(("ld-linux", "libc.", "libm.", "libglib", "libgobject", "libgmodule", "libpthread", "libdl", "librt")):
        return "LGPL-2.1-or-later"
    raise ValueError(f"no lab licence classification for {name}; review before use")


def build(root, worker):
    if root.exists():
        raise ValueError("root must not exist; never overwrite a runtime")
    if "target" not in root.parts:
        raise ValueError("lab snapshots must stay under ignored target/")
    version = subprocess.check_output(["/usr/bin/gst-launch-1.0", "--version"], env=ENV, text=True)
    if "GStreamer 1.28.2" not in version:
        raise ValueError("this lab pin requires GStreamer 1.28.2")
    root.mkdir(mode=0o700, parents=True)
    for name in ("bin", "lib", "lib64", "plugins", "proc", "dev", "tmp"):
        (root / name).mkdir(mode=0o755)
    licenses = {}
    for name in ("worker", "input"):
        (root / name).write_bytes(b"")
        (root / name).chmod(0o644)
        licenses[name] = "MIT"

    def copy(source, destination):
        if destination.exists():
            if destination.read_bytes() != source.read_bytes():
                raise ValueError(f"conflicting SONAME {destination.name}")
            return
        shutil.copyfile(source, destination)
        destination.chmod(0o755)
        licenses[str(destination.relative_to(root))] = license_for(source.name)

    def dependencies(source):
        text = subprocess.check_output(["/usr/bin/ldd", str(source)], env=ENV, text=True)
        if "not found" in text:
            raise ValueError(text)
        for path in re.findall(r"(?:=>\s+)?(/[^\s]+)\s+\(", text):
            dependency = Path(path)
            destination = root / ("lib64" if path.startswith("/lib64/") else "lib") / dependency.name
            copy(dependency, destination)

    launch = Path("/usr/bin/gst-launch-1.0")
    copy(launch, root / "bin" / launch.name)
    dependencies(launch)
    dependencies(worker)
    plugin_directory = Path("/usr/lib/x86_64-linux-gnu/gstreamer-1.0")
    for plugin in PLUGINS:
        source = plugin_directory / f"libgst{plugin}.so"
        copy(source, root / "plugins" / source.name)
        dependencies(source)
    files = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(root.rglob("*")) if path.is_file()}
    (root / "runtime.json").write_text(json.dumps({"schema": 1, "gstreamer": "1.28.2", "files": files, "licenses": licenses}, sort_keys=True))
    (root / "runtime.json").chmod(0o644)
    print(f"LAB ONLY: {root}; {len(files)} pinned files; not a source/licence/signing audit")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--worker", type=Path, default=Path("target/debug/one-mk-wall-import"))
    arguments = parser.parse_args()
    root = arguments.root.absolute()
    existed = root.exists()
    try:
        build(root, arguments.worker.absolute())
    except Exception:
        if not existed and root.exists():
            shutil.rmtree(root)
        raise
