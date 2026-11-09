#!/bin/sh
# Builds a .deb from the built programs, on Linux (needs dpkg-deb, and cargo
# for the hidden desktop entry below), from the checkout:
#   sh scripts/bundle-deb.sh <release dir, e.g. target/release> <out.deb>
set -eu
# The modes in the package must not depend on who builds it: a builder with
# umask 002 (Ubuntu's default) made group-writable files and folders.
# Fixed explicitly again before packing, below.
umask 022

dir=$1
out=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

root="$tmp/pkg"
install -d "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/applications" "$root/usr/share/icons/hicolor/256x256/apps"
install -m 755 "$dir/one-mk" "$dir/one-mk-tray" "$dir/one-mk-app" "$dir/one-mk-wallpaper" "$root/usr/bin/"
# Outside GNOME's discovery folders: only an explicit user Install action
# copies this bridge into ~/.local/share/gnome-shell/extensions.
extension=one-mk-wallpaper@one-mk.org
install -d "$root/usr/share/one-mk/gnome-extension/$extension"
for file in metadata.json extension.js bridge.xml; do
    install -m 644 "extensions/gnome/$extension/$file" "$root/usr/share/one-mk/gnome-extension/$extension/"
done
install -m 644 LICENSE-MIT LICENSE-APACHE "$root/usr/share/one-mk/gnome-extension/$extension/"
install -m 644 docs/brand/app-icon/windows-linux/hicolor/256x256/apps/one-mk.png "$root/usr/share/icons/hicolor/256x256/apps/one-mk.png"
# The polkit action of the update's one root step (docs/UPDATES.md): its own
# message, and the password asked every time.
install -d "$root/usr/share/polkit-1/actions"
install -m 644 packaging/linux/io.github.onemk.OneMk.update.policy "$root/usr/share/polkit-1/actions/"
# The AppStream metainfo (software centres, `appstreamcli`), with this
# release's notes: the same text the AppImage and the Flatpak carry.
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}
release_date=$(date -u -d "@$epoch" +%Y-%m-%d 2>/dev/null || date -u -r "$epoch" +%Y-%m-%d)
install -d "$root/usr/share/metainfo"
python3 -I scripts/metainfo.py "$version" "$release_date" > "$root/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml"
if command -v appstreamcli >/dev/null; then
    appstreamcli validate --no-net "$root/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml" >/dev/null \
        || { echo "bundle-deb.sh: the AppStream metainfo does not validate" >&2; exit 1; }
fi
# Licences: oneMK's, every statically linked crate's, the Geist fonts'.
sh scripts/copy-notices.sh "$root/usr/share/doc/one-mk"
install -m 644 packaging/linux/copyright "$root/usr/share/doc/one-mk/copyright"

cat > "$root/usr/share/applications/one-mk.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=oneMK
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
cargo run --quiet --locked -j 2 -p one-mk-ctl --example packaging-files -- "$tmp/packaged" /usr/bin >/dev/null
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
Recommends: pkexec | policykit-1
Description: Share one mouse and keyboard across your computers
 Move the pointer off the edge of one screen and it appears on the next.
 The clipboard and files follow. Run one-mk-app to open the window.
CONTROL
# Purging the package removes the update's root-only replay record.
cat > "$root/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if [ "$1" = purge ]; then rm -rf /var/lib/one-mk; fi
POSTRM
chmod 755 "$root/DEBIAN/postrm"

# Modes, whatever the builder's umask: folders 0755, programs 0755, every
# other file 0644 (and DEBIAN's postrm 0755, as dpkg requires).
find "$root" -type d -exec chmod 755 {} +
find "$root" -type f -exec chmod 644 {} +
chmod 755 "$root"/usr/bin/* "$root/DEBIAN/postrm"

dpkg-deb --root-owner-group --build "$root" "$out" >/dev/null
listing=$(dpkg-deb -c "$out")
bad=$(echo "$listing" | awk '$1 ~ /^d/ && $1 != "drwxr-xr-x" {print; next} $1 ~ /^-/ && $1 != "-rw-r--r--" && $1 != "-rwxr-xr-x" {print}')
[ -z "$bad" ] || { echo "bundle-deb.sh: $out has files or folders with unexpected modes:" >&2; echo "$bad" >&2; exit 1; }
for f in usr/bin/one-mk usr/share/applications/one-mk.desktop "$app_entry" usr/bin/one-mk-wallpaper \
    usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml \
    usr/share/polkit-1/actions/io.github.onemk.OneMk.update.policy usr/share/doc/one-mk/copyright \
    usr/share/doc/one-mk/LICENSE-MIT usr/share/doc/one-mk/LICENSE-APACHE usr/share/doc/one-mk/THIRD-PARTY-NOTICES \
    usr/share/doc/one-mk/OFL-Geist.txt \
    "usr/share/one-mk/gnome-extension/$extension/extension.js" "usr/share/one-mk/gnome-extension/$extension/bridge.xml" \
    "usr/share/one-mk/gnome-extension/$extension/metadata.json"; do
    echo "$listing" | grep -q " \./$f\$" || { echo "bundle-deb.sh: $out is missing /$f" >&2; exit 1; }
done
hidden=$(dpkg-deb --fsys-tarfile "$out" | tar -xOf - "./$app_entry")
echo "$hidden" | grep -qx 'NoDisplay=true' || { echo "bundle-deb.sh: /$app_entry is not hidden" >&2; exit 1; }
echo "Wrote $out"
