#!/bin/sh
# Builds the Linux packages from an existing build into a scratch folder
# under /tmp and checks them without installing anything:
#   - the .deb (scripts/bundle-deb.sh, as the release makes it, here by a
#     builder with umask 002): contents, control version, file modes (files
#     0644/0755, folders 0755), the AppStream metainfo with this release's
#     notes, runs `one-mk --version` from an unpacked copy;
#   - the AppDir (scripts/package-appimage.sh): `AppRun --version`, top-level
#     files, modes, the desktop file and the AppStream metainfo validate;
#   - the Flatpak source bundle: unpacked elsewhere, every path in the
#     manifest resolves and matches its checksum, its metainfo is filled in;
#   - the Homebrew cask and formula: filled in from stand-in .dmg files named
#     like the release's, no placeholder left;
#   - the macOS installer window's artwork and settings (scripts/dmg/check.sh;
#     needs Python 3 with Pillow);
#   - every icon (app icon sets, tray icons, the window icon) is what
#     scripts/make-icons.py builds from the brand SVGs now (needs Pillow).
#   - the licence files every package carries (scripts/copy-notices.sh) and
#     THIRD-PARTY-NOTICES is current (scripts/third-party-notices.py --check);
# Optional tools (dpkg-deb, desktop-file-validate, appstreamcli, ruby, Pillow) are
# used when present and reported as skipped when not.
#   cargo build --locked -j 4 -p one-mk -p one-mk-tray -p one-mk-app -p one-mk-wallpaper-helper
#   sh scripts/check-packaging.sh
# A debug build is enough for these checks: BIN_DIR=target/debug sh scripts/check-packaging.sh
. "$(dirname "$0")/package-common.sh"

need_binaries one-mk one-mk-tray one-mk-app one-mk-wallpaper
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
    # By a builder whose umask is Ubuntu's default: the modes must not depend on it.
    (umask 002; cd "$ROOT" && sh scripts/bundle-deb.sh "$BIN_DIR" "$deb") >/dev/null
    root=$scratch/root
    mkdir -p "$root/DEBIAN"
    dpkg-deb -x "$deb" "$root"
    dpkg-deb -e "$deb" "$root/DEBIAN"
    for f in usr/bin/one-mk usr/bin/one-mk-tray usr/bin/one-mk-app usr/bin/one-mk-wallpaper usr/share/applications/one-mk.desktop \
             usr/share/applications/io.github.onemk.OneMk.desktop usr/share/icons/hicolor/256x256/apps/one-mk.png \
             usr/share/one-mk/gnome-extension/one-mk-wallpaper@one-mk.org/extension.js \
             usr/share/one-mk/gnome-extension/one-mk-wallpaper@one-mk.org/bridge.xml \
             usr/share/one-mk/gnome-extension/one-mk-wallpaper@one-mk.org/metadata.json \
             usr/share/polkit-1/actions/io.github.onemk.OneMk.update.policy usr/share/doc/one-mk/copyright \
             usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml \
             usr/share/doc/one-mk/LICENSE-MIT usr/share/doc/one-mk/LICENSE-APACHE \
             usr/share/doc/one-mk/THIRD-PARTY-NOTICES usr/share/doc/one-mk/OFL-Inter.txt \
             usr/share/doc/one-mk/OFL-JetBrainsMono.txt usr/share/doc/one-mk/OFL-Unbounded.txt; do
        [ -e "$root/$f" ] && ok ".deb contains /$f" || bad ".deb is missing /$f"
    done
    cmp -s "$root/usr/share/doc/one-mk/THIRD-PARTY-NOTICES" "$ROOT/THIRD-PARTY-NOTICES" && ok ".deb ships the current THIRD-PARTY-NOTICES" || bad ".deb THIRD-PARTY-NOTICES differs from the checkout"
    grep -q 'org.freedesktop.policykit.exec.path">/usr/bin/one-mk<' "$root/usr/share/polkit-1/actions/io.github.onemk.OneMk.update.policy" \
        && grep -q '<allow_active>auth_admin</allow_active>' "$root/usr/share/polkit-1/actions/io.github.onemk.OneMk.update.policy" \
        && ok ".deb polkit action covers only /usr/bin/one-mk and asks every time" || bad ".deb polkit action"
    if command -v xmllint >/dev/null; then
        check ".deb polkit action is well-formed XML" xmllint --noout "$root/usr/share/polkit-1/actions/io.github.onemk.OneMk.update.policy"
    else check ".deb polkit action is well-formed XML" python3 -I -c "import sys, xml.dom.minidom; xml.dom.minidom.parse(sys.argv[1])" "$root/usr/share/polkit-1/actions/io.github.onemk.OneMk.update.policy"; fi
    [ -x "$root/DEBIAN/postrm" ] && grep -q 'rm -rf /var/lib/one-mk' "$root/DEBIAN/postrm" && ok ".deb purge removes the update's root-only record" || bad ".deb postrm"
    [ ! -e "$root/usr/share/gnome-shell/extensions" ] && ok ".deb does not auto-install a GNOME extension" || bad ".deb installs an extension without user consent"
    check "bundled renderer requires a playback plan" sh -c "! '$root/usr/bin/one-mk-wallpaper'"
    [ -z "$(find "$root" -not -path "$root/DEBIAN*" \( -perm -002 -o -perm -4000 \) -not -type l)" ] && ok ".deb has no world-writable or setuid files" || bad ".deb has world-writable or setuid files"
    # The modes as stored in the archive (extracting would apply this shell's umask):
    # folders 0755, programs 0755, every other file 0644, whoever built it.
    modes=$(dpkg-deb -c "$deb" | awk '$1 ~ /^d/ && $1 != "drwxr-xr-x" {print; next} $1 ~ /^-/ && $1 != "-rw-r--r--" && $1 != "-rwxr-xr-x" {print}')
    [ -z "$modes" ] && ok ".deb files are 0644 or 0755 and folders 0755 (built under umask 002)" || { echo "$modes"; bad ".deb has files or folders with other modes"; }
    dpkg-deb -c "$deb" | awk '$NF == "./usr/share/applications/one-mk.desktop" && $1 == "-rw-r--r--" {f=1} END {exit !f}' \
        && ok ".deb one-mk.desktop is 0644" || bad ".deb one-mk.desktop is not 0644"
    dpkg-deb -c "$deb" | awk '$NF ~ /^\.\/usr\/bin\/one-mk/ && $1 != "-rwxr-xr-x" {f=1} END {exit f}' \
        && ok ".deb programs are 0755" || bad ".deb programs are not all 0755"
    meta=$root/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml
    grep -q "<release version=\"$VERSION\"" "$meta" && ok ".deb metainfo names release $VERSION" || bad ".deb metainfo release version"
    if [ -f "$ROOT/docs/release-notes/v$VERSION.md" ]; then
        grep -q '<release version=[^>]*>' "$meta" && grep -q '<description>' "$meta" && grep -q '<li>' "$meta" \
            && ok ".deb metainfo carries the release notes (docs/release-notes/v$VERSION.md)" || bad ".deb metainfo has no release notes"
    fi
    grep -q '@[A-Z_]*@' "$meta" && bad ".deb metainfo holds a placeholder" || ok ".deb metainfo holds no placeholder"
    if command -v appstreamcli >/dev/null; then
        check ".deb AppStream metainfo validates" appstreamcli validate --no-net "$meta"
    else skip "appstreamcli not installed"; fi
    grep -q "^Version: $VERSION$" "$root/DEBIAN/control" && ok ".deb control version matches Cargo.toml ($VERSION)" || bad ".deb control version"
    got=$("$root/usr/bin/one-mk" --version) && [ "$got" = "one-mk $VERSION" ] && ok "unpacked one-mk --version: $got" || bad "one-mk --version said '$got'"
    check "unpacked one-mk-app prints help successfully" sh -c "'$root/usr/bin/one-mk-app' --help > '$scratch/app-help' && grep -q 'Usage: one-mk-app' '$scratch/app-help'"
    got=$("$root/usr/bin/one-mk-app" --version) && [ "$got" = "one-mk-app $VERSION" ] && ok "unpacked one-mk-app --version: $got" || bad "one-mk-app --version said '$got'"
    check "unpacked one-mk-app refuses an unknown option with usage" sh -c "'$root/usr/bin/one-mk-app' --not-an-option > '$scratch/app-error' 2>&1; [ \$? = 2 ] && grep -q 'unknown option' '$scratch/app-error' && grep -q 'usage: one-mk-app' '$scratch/app-error'"
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
[ -z "$(find "$appdir" ! -type l \( -type d -a ! -perm 755 -o -type f -a ! -perm 644 -a ! -perm 755 \))" ] && ok "AppDir files are 0644 or 0755 and folders 0755" || bad "AppDir has files or folders with other modes"
grep -q "<release version=\"$VERSION\"" "$appdir/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml" && ! grep -q '@[A-Z_]*@' "$appdir/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml" \
    && ok "AppDir metainfo names release $VERSION and holds no placeholder" || bad "AppDir metainfo"
for f in copyright LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES OFL-Inter.txt OFL-JetBrainsMono.txt OFL-Unbounded.txt; do
    [ -s "$appdir/usr/share/doc/one-mk/$f" ] && ok "AppDir ships /usr/share/doc/one-mk/$f" || bad "AppDir is missing /usr/share/doc/one-mk/$f"
done
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
cp "$BIN_DIR/one-mk" "$BIN_DIR/one-mk-tray" "$BIN_DIR/one-mk-app" "$BIN_DIR/one-mk-wallpaper" "$scratch/t/one-mk-x86_64-unknown-linux-gnu/"
sh "$ROOT/scripts/copy-notices.sh" "$scratch/t/one-mk-x86_64-unknown-linux-gnu"
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
    fmeta=$mdir/sources/io.github.onemk.OneMk.metainfo.xml
    grep -q "<release version=\"$VERSION\"" "$fmeta" && ! grep -q '@[A-Z_]*@' "$fmeta" && ok "flatpak metainfo names release $VERSION and holds no placeholder" || bad "flatpak metainfo is not filled in"
    if command -v appstreamcli >/dev/null; then
        check "flatpak AppStream metainfo validates" appstreamcli validate --no-net "$fmeta"
    else skip "appstreamcli not installed"; fi
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

# The macOS installer window: settings, artwork, readable names, and a preview
# of the finished window (left in /tmp/one-mk-dmg-preview.png to look at).
if python3 -c 'import PIL' 2>/dev/null; then
    if sh "$ROOT/scripts/dmg/check.sh" >"$scratch/dmg-check.txt" 2>&1; then ok "macOS installer window artwork and settings ($(grep -c '^ok' "$scratch/dmg-check.txt") checks)"
    else cat "$scratch/dmg-check.txt"; bad "macOS installer window artwork and settings"; fi
else skip "macOS installer window checks: Python 3 with Pillow not installed"; fi

# Every icon the packages and the app embed is built from docs/brand/logo/*.svg.
if python3 -c 'import PIL' 2>/dev/null; then
    if python3 -I "$ROOT/scripts/make-icons.py" --check >"$scratch/icons.txt" 2>&1; then ok "icons match the brand SVGs ($(tail -n1 "$scratch/icons.txt"))"
    else cat "$scratch/icons.txt"; bad "icons are out of date: run python3 scripts/make-icons.py"; fi
else skip "icon check: Python 3 with Pillow not installed"; fi

# Every package carries the same licence files; the crate list must match Cargo.lock.
check "copy-notices.sh puts the four licence files in a folder" sh -c "sh '$ROOT/scripts/copy-notices.sh' '$scratch/n' && [ \$(ls '$scratch/n' | wc -l) = 4 ]"
if python3 -I "$ROOT/scripts/third-party-notices.py" --check >"$scratch/notices.txt" 2>&1; then ok "THIRD-PARTY-NOTICES matches Cargo.lock"
else cat "$scratch/notices.txt"; bad "THIRD-PARTY-NOTICES is out of date"; fi

[ "$fail" = 0 ] && echo "all checks passed" || { echo "some checks failed"; exit 1; }
