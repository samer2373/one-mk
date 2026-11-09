#!/usr/bin/env bash
# Writes ASSET_DIRECTORY/update-manifest.json, the list the updater verifies.
# Optional, for a bad release (docs/UPDATES.md):
#   ONE_MK_REVOKED="0.1.27 0.1.28"          versions no updater may install
#   ONE_MK_MINIMUM_SUPPORTED_VERSION=0.1.20  older installations must reinstall by hand
set -euo pipefail
if [ "$#" -ne 2 ]; then
  echo 'Usage: bash scripts/make-update-manifest.sh ASSET_DIRECTORY POSITIVE_SEQUENCE' >&2
  exit 2
fi
version=$(sh scripts/version.sh)
release_time=${RELEASE_TIME:-$(git show -s --format=%ct HEAD)}
python3 - "$1" "$2" "$version" "$release_time" "${ONE_MK_REVOKED:-}" "${ONE_MK_MINIMUM_SUPPORTED_VERSION:-0.1.0}" <<'PY'
import datetime
import hashlib
import json
import pathlib
import re
import sys

directory, sequence, version, timestamp, revoked_text, minimum = sys.argv[1:]
semver = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
def parts(text):
    match = semver.fullmatch(text)
    if not match:
        sys.exit(f"Not a release version (MAJOR.MINOR.PATCH): {text!r}")
    return tuple(map(int, match.groups()))
revoked = sorted(set(revoked_text.replace(",", " ").split()), key=parts)
if version in revoked:
    sys.exit(f"Refusing to revoke the version being released ({version})")
if parts(minimum) > parts(version):
    sys.exit(f"ONE_MK_MINIMUM_SUPPORTED_VERSION {minimum} is newer than the release {version}")
if len(revoked) > 128:
    sys.exit("At most 128 revoked versions")
if not sequence.isascii() or not sequence.isdecimal() or not 0 < int(sequence) <= 2**64 - 1:
    sys.exit("Release sequence must be a positive u64")
assets = []
for path in sorted(pathlib.Path(directory).iterdir()):
    if path.name in {"SHA256SUMS", "update-manifest.json", "update-manifest.json.sig"}:
        continue
    if path.is_symlink() or not path.is_file() or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,159}", path.name) or ".." in path.name:
        sys.exit("Unsafe staged asset: " + path.name)
    name = path.name
    if "unknown-linux" in name:
        system = "linux"
    elif "apple-darwin" in name:
        system = "macos"
    elif "pc-windows" in name:
        system = "windows"
    else:
        system = "any"
    arch = "aarch64" if "aarch64" in name else "x86_64" if "x86_64" in name else "any"
    kind = "metadata"
    if name.endswith(".AppImage"):
        kind = "appimage"
    elif name.endswith("-setup.exe"):
        kind = "installer"
    elif name.endswith((".dmg", ".app.zip")):
        kind = "app"
    elif name.endswith(".deb"):
        kind = "deb"
    elif name.endswith((".tar.gz", ".zip")):
        kind = "archive"
    elif name.endswith((".bin", ".exe")):
        kind = "binary"
    size = path.stat().st_size
    if not 0 < size <= 512 * 1024 * 1024:
        sys.exit("Asset outside size limits: " + name)
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    assets.append(dict(name=name, size=size, sha256=digest.hexdigest(), os=system, arch=arch, kind=kind))
if not 0 < len(assets) <= 128:
    sys.exit("Expected between 1 and 128 staged assets")
manifest = dict(version=version, release_time=datetime.datetime.fromtimestamp(int(timestamp), datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), sequence=int(sequence), minimum_supported_version=minimum, assets=assets, revoked=revoked)
data = (json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n").encode()
if len(data) > 256 * 1024:
    sys.exit("Manifest exceeds 256 KiB")
pathlib.Path(directory, "update-manifest.json").write_bytes(data)
PY
