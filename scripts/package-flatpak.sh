#!/bin/sh
# Writes dist/flatpak/io.github.onemk.OneMk.yml for a release tarball, and
# builds the Flatpak if flatpak-builder is installed. UNTESTED end to end.
#   sh scripts/package-flatpak.sh [URL_OF_TARBALL]
# Without an argument it points at dist/one-mk-x86_64-unknown-linux-gnu.tar.gz
# (what the release workflow builds) by file path.
. "$(dirname "$0")/package-common.sh"

tarball=$OUT_DIR/one-mk-x86_64-unknown-linux-gnu.tar.gz
if [ $# -ge 1 ]; then
    url=$1
    # The checksum of what is behind the URL: download it once to hash it.
    tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
    curl -fsSL "$url" -o "$tmp/t.tar.gz"
    sha=$(sha256sum "$tmp/t.tar.gz" | cut -d' ' -f1)
else
    [ -f "$tarball" ] || die "$tarball not found; build the release tarball first (see docs/PACKAGING.md)"
    url=file://$tarball
    sha=$(sha256sum "$tarball" | cut -d' ' -f1)
fi

dir=$OUT_DIR/flatpak
mkdir -p "$dir"
sed -e "s|@URL@|$url|" -e "s|@SHA256@|$sha|" -e "s|@VERSION@|$VERSION|" -e "s|@ROOT@|$ROOT|" \
    "$ROOT/packaging/flatpak/io.github.onemk.OneMk.yml.in" > "$dir/io.github.onemk.OneMk.yml"
echo "$dir/io.github.onemk.OneMk.yml"

if command -v flatpak-builder >/dev/null; then
    (cd "$dir" && flatpak-builder --force-clean --user --install-deps-from=flathub \
        --repo="$dir/repo" "$dir/build" "$dir/io.github.onemk.OneMk.yml")
    flatpak build-bundle "$dir/repo" "$OUT_DIR/one-mk-$VERSION.flatpak" io.github.onemk.OneMk
    echo "$OUT_DIR/one-mk-$VERSION.flatpak"
else
    echo "flatpak-builder not found: wrote the manifest only." >&2
fi
