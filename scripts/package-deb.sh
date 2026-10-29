#!/bin/sh
# Builds dist/one-mk_<version>-1_<arch>.deb from an existing release build.
#   cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
#   sh scripts/package-deb.sh
# Needs dpkg-deb; uses dpkg-shlibdeps for Depends when it is installed.
# Never installs or starts anything: the package only copies files, and its
# maintainer scripts only refresh the icon and desktop caches.
. "$(dirname "$0")/package-common.sh"

need_binaries one-mk one-mk-tray one-mk-app
command -v dpkg-deb >/dev/null || die "dpkg-deb not found (this needs a Debian-based system)"

case "${DEB_ARCH:-$(uname -m)}" in
    x86_64|amd64) arch=amd64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) die "no Debian architecture for $(uname -m); set DEB_ARCH" ;;
esac
# 0.2.0-rc1 sorts before 0.2.0 in Debian as 0.2.0~rc1.
debver=$(printf '%s' "$VERSION" | sed 's/-/~/')-1
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
pkg=$work/pkg
mkdir -p "$pkg/DEBIAN"

linux_tree "$pkg" /usr/bin

# Debian wants a changelog; the real one is the GitHub release notes.
mkdir -p "$pkg/usr/share/doc/one-mk"
{
    printf 'one-mk (%s) unstable; urgency=medium\n\n  * Release %s. See https://github.com/samer2373/one-mk/releases\n\n' "$debver" "$VERSION"
    printf ' -- Samer Abu-Sleem <samer2373@gmail.com>  %s\n' "$(date -u -d "@$SOURCE_DATE_EPOCH" '+%a, %d %b %Y %H:%M:%S +0000')"
} | gzip -9n > "$pkg/usr/share/doc/one-mk/changelog.Debian.gz"

# What /etc files the user may edit (the tray autostart entry).
(cd "$pkg" && find etc -type f | sed 's|^|/|') > "$pkg/DEBIAN/conffiles"

# Libraries the programs need. The graphics and keyboard libraries the
# window opens at run time are only recommended, so a headless machine
# can install the command-line part.
depends="libc6"
if command -v dpkg-shlibdeps >/dev/null; then
    mkdir -p "$work/shl/debian"
    printf 'Source: one-mk\n\nPackage: one-mk\nArchitecture: any\n' > "$work/shl/debian/control"
    depends=$(cd "$work/shl" && dpkg-shlibdeps -O -e "$pkg/usr/bin/one-mk" -e "$pkg/usr/bin/one-mk-tray" -e "$pkg/usr/bin/one-mk-app" 2>/dev/null \
        | sed -n 's/^shlibs:Depends=//p')
    [ -n "$depends" ] || depends="libc6"
fi

install -m 0755 "$ROOT/packaging/linux/postinst" "$pkg/DEBIAN/postinst"
install -m 0755 "$ROOT/packaging/linux/postrm" "$pkg/DEBIAN/postrm"
size=$(du -sk --exclude=DEBIAN "$pkg" | cut -f1)
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: one-mk
Version: $debver
Architecture: $arch
Maintainer: Samer Abu-Sleem <samer2373@gmail.com>
Installed-Size: $size
Depends: $depends
Recommends: libegl1 | libgl1, libxkbcommon0, libwayland-client0
Suggests: gnome-shell-extension-appindicator
Section: utils
Priority: optional
Homepage: https://github.com/samer2373/one-mk
Description: share one mouse and keyboard between your computers
 one-mk moves the mouse and keyboard to the next computer when the pointer
 crosses the edge of the screen, and shares the clipboard and files. It is
 encrypted, peer to peer and small.
 .
 Installing starts nothing. To start it when you log in, run "one-mk install"
 as your own user (or "systemctl --user enable --now one-mk").
CONTROL

clamp_times "$pkg"
out=$OUT_DIR/one-mk_${debver}_$arch.deb
dpkg-deb --root-owner-group -Zxz -b "$pkg" "$out" >/dev/null
echo "$out"
