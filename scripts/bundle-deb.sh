#!/bin/sh
# Builds a .deb from the built programs, on Linux (needs dpkg-deb, and cargo
# for the hidden desktop entry below), from the checkout:
#   sh scripts/bundle-deb.sh <release dir, e.g. target/release> <out.deb>
set -eu

dir=$1
out=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

root="$tmp/pkg"
install -d "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/applications" "$root/usr/share/icons/hicolor/256x256/apps"
install -m 755 "$dir/one-mk" "$dir/one-mk-tray" "$dir/one-mk-app" "$root/usr/bin/"
install -m 644 docs/brand/app-icon/windows-linux/ico-png/onemk-256.png "$root/usr/share/icons/hicolor/256x256/apps/one-mk.png"

cat > "$root/usr/share/applications/one-mk.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=one·mk
Comment=Share one mouse and keyboard across your computers
Exec=one-mk-app
Icon=one-mk
Categories=Utility;
DESKTOP

# The hidden entry named after the app id (io.github.onemk.OneMk): GNOME's
# GlobalShortcuts portal refuses a session to an app id with no installed
# desktop entry. Written by the code `one-mk install` writes it with
# (one_mk_ctl::install::packaged_files), so the two can't drift apart. Only
# that entry: the service and the tray stay something each user turns on.
cargo run --quiet --locked -p one-mk-ctl --example packaging-files -- "$tmp/packaged" /usr/bin >/dev/null
set -- "$tmp"/packaged/usr/share/applications/*.desktop
[ $# -eq 1 ] && [ -f "$1" ] || { echo "bundle-deb.sh: expected one packaged desktop entry, got: $*" >&2; exit 1; }
app_entry=usr/share/applications/$(basename "$1")
install -m 644 "$1" "$root/$app_entry"

cat > "$root/DEBIAN/control" <<CONTROL
Package: one-mk
Version: $version
Architecture: amd64
Maintainer: Samer Abu-Sleem <samer2373@users.noreply.github.com>
Section: utils
Priority: optional
Homepage: https://github.com/samer2373/one-mk
Description: Share one mouse and keyboard across your computers
 Move the pointer off the edge of one screen and it appears on the next.
 The clipboard and files follow. Run one-mk-app to open the window.
CONTROL

dpkg-deb --root-owner-group --build "$root" "$out" >/dev/null
listing=$(dpkg-deb -c "$out")
for f in usr/bin/one-mk usr/share/applications/one-mk.desktop "$app_entry"; do
    echo "$listing" | grep -q " \./$f\$" || { echo "bundle-deb.sh: $out is missing /$f" >&2; exit 1; }
done
hidden=$(dpkg-deb --fsys-tarfile "$out" | tar -xOf - "./$app_entry")
echo "$hidden" | grep -qx 'NoDisplay=true' || { echo "bundle-deb.sh: /$app_entry is not hidden" >&2; exit 1; }
echo "Wrote $out"
