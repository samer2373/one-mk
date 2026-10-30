#!/bin/sh
# Writes the Flatpak manifest and everything it names, packs them as one
# self-contained source bundle, and builds the Flatpak if flatpak-builder is
# installed. UNTESTED end to end (no flatpak-builder here).
#   sh scripts/package-flatpak.sh [URL_OR_FILE_OF_THE_TARBALL [URL]]
# Without an argument it uses dist/one-mk-x86_64-unknown-linux-gnu.tar.gz
# (the plain Linux archive the release workflow's "Package" step makes and
# publishes, the one install.sh downloads). With a URL, the manifest downloads the
# tarball from there (and its checksum is taken from a download made here);
# with a file, the tarball is copied into the bundle. With a file and a URL
# (the release workflow), the manifest downloads the tarball from the URL
# with the file's checksum: the file is what will be published there, so
# the bundle can be made before anything is public.
#
# Writes dist/flatpak/ (io.github.onemk.OneMk.yml and sources/) and
# dist/one-mk-flatpak-sources.tar.gz: that folder, packed with
# relative paths only and a sha256 for every source, so it builds anywhere.
# The release publishes the bundle, never the bare manifest.
. "$(dirname "$0")/package-common.sh"

arg=${1:-$OUT_DIR/one-mk-x86_64-unknown-linux-gnu.tar.gz}
url=${2:-}
dir=$OUT_DIR/flatpak
rm -rf "$dir"
mkdir -p "$dir/sources"

case "$arg" in
    http://*|https://*)
        # The checksum of what is behind the URL: download it once to hash it.
        tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
        curl -fsSL "$arg" -o "$tmp/t.tar.gz"
        key=url ref=$arg
        sha=$(sha256sum "$tmp/t.tar.gz" | cut -d' ' -f1) ;;
    *)
        [ -f "$arg" ] || die "$arg not found; build the release tarball first (see docs/PACKAGING.md)"
        case "$url" in
            http://*|https://*) key=url ref=$url ;;
            '') cp "$arg" "$dir/sources/$(basename "$arg")"
                key=path ref=sources/$(basename "$arg") ;;
            *) die "$url is not a URL" ;;
        esac
        sha=$(sha256sum "$arg" | cut -d' ' -f1) ;;
esac

hash_of() { sha256sum "$1" | cut -d' ' -f1; }
icons=$ROOT/docs/brand/app-icon/windows-linux/hicolor
cp "$ROOT/packaging/linux/one-mk.desktop" "$dir/sources/one-mk.desktop"
cp "$ROOT/packaging/linux/io.github.onemk.OneMk.metainfo.xml" "$dir/sources/io.github.onemk.OneMk.metainfo.xml"
cp "$icons/scalable/apps/one-mk.svg" "$dir/sources/icon.svg"
cp "$icons/256x256/apps/one-mk.png" "$dir/sources/icon-256.png"

sed -e "s|@ARCHIVE_KEY@|$key|" -e "s|@ARCHIVE_REF@|$ref|" -e "s|@SHA256@|$sha|" \
    -e "s|@SHA256_DESKTOP@|$(hash_of "$dir/sources/one-mk.desktop")|" \
    -e "s|@SHA256_METAINFO@|$(hash_of "$dir/sources/io.github.onemk.OneMk.metainfo.xml")|" \
    -e "s|@SHA256_ICON_SVG@|$(hash_of "$dir/sources/icon.svg")|" \
    -e "s|@SHA256_ICON_PNG@|$(hash_of "$dir/sources/icon-256.png")|" \
    "$ROOT/packaging/flatpak/io.github.onemk.OneMk.yml.in" > "$dir/io.github.onemk.OneMk.yml"
echo "$dir/io.github.onemk.OneMk.yml"
if grep -q '@[A-Z0-9_]*@' "$dir/io.github.onemk.OneMk.yml"; then die "a placeholder is left in the manifest"; fi

cat > "$dir/README.txt" <<README
one-mk $VERSION, Flatpak source bundle: the manifest and every file it names.
Build with:
  flatpak-builder --force-clean --user --install-deps-from=flathub --repo=repo build io.github.onemk.OneMk.yml
Every source in the manifest has a sha256; paths are relative to the manifest.
README

# The bundle: a folder named for the version, files in sorted order, the same bytes every time.
bundle=$OUT_DIR/one-mk-flatpak-sources.tar.gz
work=$(mktemp -d)
trap 'rm -rf "$work" "${tmp:-}"' EXIT
mkdir "$work/one-mk-flatpak-$VERSION"
cp -R "$dir/." "$work/one-mk-flatpak-$VERSION/"
clamp_times "$work/one-mk-flatpak-$VERSION"
if tar --version 2>/dev/null | grep -q GNU; then
    set -- --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH"
else
    set --
fi
(cd "$work" && tar "$@" -cf - "one-mk-flatpak-$VERSION" | gzip -9n > "$bundle")
echo "$bundle"

if command -v flatpak-builder >/dev/null; then
    (cd "$dir" && flatpak-builder --force-clean --user --install-deps-from=flathub \
        --repo="$dir/repo" "$dir/build" "$dir/io.github.onemk.OneMk.yml")
    flatpak build-bundle "$dir/repo" "$OUT_DIR/one-mk-x86_64-unknown-linux-gnu.flatpak" io.github.onemk.OneMk
    echo "$OUT_DIR/one-mk-x86_64-unknown-linux-gnu.flatpak"
else
    echo "flatpak-builder not found: wrote the manifest and the source bundle only." >&2
fi
