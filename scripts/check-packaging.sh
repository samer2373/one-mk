#!/bin/sh
# Builds the Linux packages from an existing build into a scratch folder
# under /tmp and checks them without installing anything:
#   - the .deb (scripts/bundle-deb.sh, as the release makes it): contents,
#     control version, runs `one-mk --version` from an unpacked copy;
#   - the AppDir (scripts/package-appimage.sh): `AppRun --version`, top-level
#     files, the desktop file and the AppStream metainfo validate;
#   - the Flatpak source bundle: unpacked elsewhere, every path in the
#     manifest resolves and matches its checksum;
#   - the Homebrew cask and formula: filled in from stand-in .dmg files named
#     like the release's, no placeholder left.
# Optional tools (dpkg-deb, desktop-file-validate, appstreamcli, ruby) are
# used when present and reported as skipped when not.
#   cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
#   sh scripts/check-packaging.sh
# A debug build is enough for these checks: BIN_DIR=target/debug sh scripts/check-packaging.sh
. "$(dirname "$0")/package-common.sh"

need_binaries one-mk one-mk-tray one-mk-app
BIN_DIR=$(cd "$BIN_DIR" && pwd)
export BIN_DIR   # the package scripts run below use the same build
scratch=$(mktemp -d /tmp/one-mk-pkg-check.XXXXXX)
trap 'rm -rf "$scratch"' EXIT
fail=0
ok()   { echo "ok      $*"; }
bad()  { echo "FAILED  $*"; fail=1; }
skip() { echo "skipped $*"; }
check() { desc=$1; shift; if "$@" >/dev/null 2>&1; then ok "$desc"; else bad "$desc"; fi; }

# The .deb, made the way the release makes it (bundle-deb.sh reads Cargo.toml
# and the icon relative to the checkout, so it runs from there).
if command -v dpkg-deb >/dev/null; then
    deb=$scratch/one-mk-x86_64-unknown-linux-gnu.deb
    (cd "$ROOT" && sh scripts/bundle-deb.sh "$BIN_DIR" "$deb") >/dev/null
    root=$scratch/root
    mkdir -p "$root/DEBIAN"
    dpkg-deb -x "$deb" "$root"
    dpkg-deb -e "$deb" "$root/DEBIAN"
    for f in usr/bin/one-mk usr/bin/one-mk-tray usr/bin/one-mk-app usr/share/applications/one-mk.desktop \
             usr/share/applications/io.github.onemk.OneMk.desktop usr/share/icons/hicolor/256x256/apps/one-mk.png; do
        [ -e "$root/$f" ] && ok ".deb contains /$f" || bad ".deb is missing /$f"
    done
    [ -z "$(find "$root" -not -path "$root/DEBIAN*" \( -perm -002 -o -perm -4000 \) -not -type l)" ] && ok ".deb has no world-writable or setuid files" || bad ".deb has world-writable or setuid files"
    grep -q "^Version: $VERSION$" "$root/DEBIAN/control" && ok ".deb control version matches Cargo.toml ($VERSION)" || bad ".deb control version"
    got=$("$root/usr/bin/one-mk" --version) && [ "$got" = "one-mk $VERSION" ] && ok "unpacked one-mk --version: $got" || bad "one-mk --version said '$got'"
    check "unpacked one-mk-app starts (prints its usage and refuses an unknown option)" sh -c "! '$root/usr/bin/one-mk-app' --help"
    check "unpacked one-mk-tray starts (prints its usage and refuses an unknown option)" sh -c "! '$root/usr/bin/one-mk-tray' --help"
    # The hidden app-id entry the GlobalShortcuts portal needs, as `one-mk
    # install` writes it (the packaging-files example prints the same text).
    hidden=$root/usr/share/applications/io.github.onemk.OneMk.desktop
    grep -qx 'NoDisplay=true' "$hidden" && grep -qx 'Exec="/usr/bin/one-mk" "run"' "$hidden" \
        && ok ".deb app-id entry is hidden and runs /usr/bin/one-mk" || bad ".deb app-id entry"
    if command -v desktop-file-validate >/dev/null; then
        check ".deb desktop file validates" desktop-file-validate "$root/usr/share/applications/one-mk.desktop"
        check ".deb app-id desktop file validates" desktop-file-validate "$hidden"
    else skip "desktop-file-validate not installed"; fi
else skip ".deb checks: dpkg-deb not installed"; fi

# The AppDir (appimagetool is not needed for this part).
OUT_DIR=$scratch/c APPIMAGETOOL= sh "$ROOT/scripts/package-appimage.sh" >/dev/null 2>&1
appdir=$scratch/c/one-mk.AppDir
got=$("$appdir/AppRun" --version) && [ "$got" = "one-mk $VERSION" ] && ok "AppDir: AppRun --version: $got" || bad "AppDir AppRun --version"
[ -f "$appdir/one-mk.desktop" ] && [ -f "$appdir/one-mk.png" ] && [ -L "$appdir/.DirIcon" ] && ok "AppDir has desktop file and icons at the top" || bad "AppDir top level"
if command -v desktop-file-validate >/dev/null; then
    check "AppDir desktop file validates" desktop-file-validate "$appdir/one-mk.desktop"
else skip "desktop-file-validate not installed"; fi
if command -v appstreamcli >/dev/null; then
    check "AppStream metainfo validates" appstreamcli validate --no-net "$appdir/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml"
else skip "appstreamcli not installed"; fi

# The Flatpak source bundle: built here from a tarball laid out like the
# release's (the "Package" step of release.yml), unpacked somewhere else, and
# every path in the manifest must resolve there and match its checksum.
mkdir -p "$scratch/t/one-mk-x86_64-unknown-linux-gnu"
cp "$BIN_DIR/one-mk" "$BIN_DIR/one-mk-tray" "$BIN_DIR/one-mk-app" "$scratch/t/one-mk-x86_64-unknown-linux-gnu/"
(cd "$scratch/t" && tar czf one-mk-x86_64-unknown-linux-gnu.tar.gz one-mk-x86_64-unknown-linux-gnu)
OUT_DIR=$scratch/f sh "$ROOT/scripts/package-flatpak.sh" "$scratch/t/one-mk-x86_64-unknown-linux-gnu.tar.gz" >/dev/null 2>&1
bundle=$(ls "$scratch"/f/one-mk-flatpak-sources.tar.gz 2>/dev/null) || bundle=
if [ -n "$bundle" ]; then
    mkdir -p "$scratch/elsewhere"
    tar -xzf "$bundle" -C "$scratch/elsewhere"
    rm -rf "$scratch/f"   # what the manifest points at must be in the bundle, not left behind
    manifest=$(ls "$scratch"/elsewhere/*/io.github.onemk.OneMk.yml)
    mdir=$(dirname "$manifest")
    refs=$(sed -n 's/^ *path: //p' "$manifest")
    [ -n "$refs" ] && ok "flatpak manifest names $(echo "$refs" | wc -l | tr -d ' ') local sources" || bad "flatpak manifest names no sources"
    for ref in $refs; do
        want=$(awk -v r="$ref" '$1=="path:" && $2==r {f=1; next} f && $1=="sha256:" {print $2; exit}' "$manifest")
        case "$ref" in /*|*..*) bad "flatpak source $ref is not a plain relative path"; continue ;; esac
        if [ -f "$mdir/$ref" ] && [ "$(sha256sum < "$mdir/$ref" | cut -d' ' -f1)" = "$want" ]; then ok "flatpak source $ref resolves and matches its sha256"
        else bad "flatpak source $ref is missing from the bundle or its sha256 is wrong"; fi
    done
    grep -q "$ROOT\|$scratch\|@[A-Z0-9_]*@" "$manifest" && bad "flatpak manifest holds a checkout path or a placeholder" || ok "flatpak manifest holds no checkout path or placeholder"
else bad "no flatpak source bundle was written"; fi
# As the release workflow makes it, before the release is public: the
# tarball's checksum from the local file, the manifest naming its URL.
url=https://github.com/samer2373/one-mk/releases/download/v$VERSION/one-mk-x86_64-unknown-linux-gnu.tar.gz
if OUT_DIR=$scratch/u sh "$ROOT/scripts/package-flatpak.sh" "$scratch/t/one-mk-x86_64-unknown-linux-gnu.tar.gz" "$url" >/dev/null 2>&1; then
    m=$scratch/u/flatpak/io.github.onemk.OneMk.yml
    want=$(sha256sum < "$scratch/t/one-mk-x86_64-unknown-linux-gnu.tar.gz" | cut -d' ' -f1)
    awk -v u="$url" -v w="$want" '$1=="url:" && $2==u {f=1; next} f && $1=="sha256:" {exit !($2==w)} END {if (!f) exit 1}' "$m" \
        && ok "flatpak manifest made from a local tarball names its release URL with its sha256" || bad "flatpak manifest from a local tarball and a URL"
    [ ! -e "$scratch/u/flatpak/sources/one-mk-x86_64-unknown-linux-gnu.tar.gz" ] && ok "a tarball named by URL is not copied into the bundle" || bad "a tarball named by URL was copied into the bundle"
else bad "package-flatpak.sh with a local tarball and its URL"; fi

# The Homebrew files, from stand-ins named like the release's .dmg files.
mkdir -p "$scratch/h"
echo arm > "$scratch/h/one-mk-aarch64-apple-darwin.dmg"
echo x64 > "$scratch/h/one-mk-x86_64-apple-darwin.dmg"
if OUT_DIR=$scratch/h SOURCE_SHA256=0000000000000000000000000000000000000000000000000000000000000000 \
        sh "$ROOT/scripts/package-homebrew.sh" >/dev/null 2>&1; then
    cask=$scratch/h/homebrew/one-mk.rb formula=$scratch/h/homebrew/one-mk-source.rb
    grep -q '@[A-Z0-9_]*@' "$cask" "$formula" && bad "a placeholder is left in the Homebrew files" || ok "Homebrew cask and formula hold no placeholder"
    grep -q "\"$(sha256sum < "$scratch/h/one-mk-aarch64-apple-darwin.dmg" | cut -d' ' -f1)\"" "$cask" && ok "cask has the Apple silicon .dmg's checksum" || bad "cask checksum"
    grep -q 'one-mk-#{arch}-apple-darwin.dmg' "$cask" && ok "cask downloads the .dmg names the release publishes" || bad "cask .dmg URL"
    if command -v ruby >/dev/null; then
        check "Homebrew files are valid Ruby" sh -c "ruby -c '$cask' && ruby -c '$formula'"
    else skip "ruby not installed"; fi
else bad "package-homebrew.sh failed"; fi

[ "$fail" = 0 ] && echo "all checks passed" || { echo "some checks failed"; exit 1; }
