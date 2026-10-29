#!/bin/sh
# Builds the Linux packages from an existing release build into a scratch
# folder under /tmp and checks them without installing anything:
#   - the .deb: contents, control, runs `one-mk --version` from an unpacked copy,
#     builds twice, and under two other time zones, and compares checksums, validates the desktop files, the
#     AppStream file and the systemd user unit;
#   - the AppDir: `AppRun --version`.
# Optional tools (lintian, desktop-file-validate, appstreamcli, systemd-analyze)
# are used when present and reported as skipped when not.
#   cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
#   sh scripts/check-packaging.sh
. "$(dirname "$0")/package-common.sh"

scratch=$(mktemp -d /tmp/one-mk-pkg-check.XXXXXX)
trap 'rm -rf "$scratch"' EXIT
fail=0
ok()   { echo "ok      $*"; }
bad()  { echo "FAILED  $*"; fail=1; }
skip() { echo "skipped $*"; }
check() { desc=$1; shift; if "$@" >/dev/null 2>&1; then ok "$desc"; else bad "$desc"; fi; }

OUT_DIR=$scratch/a sh "$ROOT/scripts/package-deb.sh" >/dev/null
OUT_DIR=$scratch/b sh "$ROOT/scripts/package-deb.sh" >/dev/null
deb=$(ls "$scratch"/a/*.deb)
[ "$(sha256sum < "$deb")" = "$(sha256sum < "$(ls "$scratch"/b/*.deb)")" ] && ok "the .deb is byte for byte reproducible" || bad "the .deb differs between two builds"

# The builder's time zone must not matter (touch -t reads its stamp in local time).
for zone in Asia/Amman Pacific/Honolulu; do
    TZ=$zone OUT_DIR=$scratch/tz sh "$ROOT/scripts/package-deb.sh" >/dev/null
    [ "$(sha256sum < "$deb")" = "$(sha256sum < "$(ls "$scratch"/tz/*.deb)")" ] && ok "the .deb is the same built under TZ=$zone" || bad "the .deb built under TZ=$zone differs from the one built under the default time zone"
    rm -rf "$scratch/tz"
done

root=$scratch/root
mkdir -p "$root/DEBIAN"
dpkg-deb -x "$deb" "$root"
dpkg-deb -e "$deb" "$root/DEBIAN"
for f in usr/bin/one-mk usr/bin/one-mk-tray usr/bin/one-mk-app usr/lib/systemd/user/one-mk.service \
         etc/xdg/autostart/one-mk-tray.desktop usr/share/applications/one-mk.desktop \
         usr/share/applications/io.github.onemk.OneMk.desktop usr/share/icons/hicolor/256x256/apps/one-mk.png \
         usr/share/icons/hicolor/scalable/apps/one-mk.svg usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml \
         usr/share/doc/one-mk/copyright; do
    [ -e "$root/$f" ] && ok "contains /$f" || bad "missing /$f"
done
# Files in the package must not be writable by others, and nothing sets uid.
[ -z "$(find "$root" -not -path "$root/DEBIAN*" \( -perm -002 -o -perm -4000 \) -not -type l)" ] && ok "no world-writable or setuid files" || bad "world-writable or setuid files"
grep -q "^Version: $(echo "$VERSION" | sed 's/-/~/')-1$" "$root/DEBIAN/control" && ok "control version matches Cargo.toml ($VERSION)" || bad "control version"
# The maintainer scripts must never start or enable a service.
if grep -E 'systemctl|service |start|enable' "$root/DEBIAN/postinst" "$root/DEBIAN/postrm" | grep -v '^\S*:#' >/dev/null; then bad "a maintainer script starts or enables something"; else ok "maintainer scripts start nothing"; fi
got=$("$root/usr/bin/one-mk" --version) && [ "$got" = "one-mk $VERSION" ] && ok "unpacked one-mk --version: $got" || bad "one-mk --version said '$got'"
check "unpacked one-mk-app starts (prints its usage and refuses an unknown option)" sh -c "! '$root/usr/bin/one-mk-app' --help"
check "unpacked one-mk-tray starts (prints its usage and refuses an unknown option)" sh -c "! '$root/usr/bin/one-mk-tray' --help"

if command -v desktop-file-validate >/dev/null; then
    check "desktop files validate" desktop-file-validate "$root/usr/share/applications/one-mk.desktop" \
        "$root/usr/share/applications/io.github.onemk.OneMk.desktop" "$root/etc/xdg/autostart/one-mk-tray.desktop"
else skip "desktop-file-validate not installed"; fi
if command -v appstreamcli >/dev/null; then
    check "AppStream metainfo validates" appstreamcli validate --no-net "$root/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml"
else skip "appstreamcli not installed"; fi
if command -v systemd-analyze >/dev/null; then
    # The unit names /usr/bin/one-mk; point a copy at the unpacked one so verify can see the program.
    sed "s#\"/usr/bin/#\"$root/usr/bin/#" "$root/usr/lib/systemd/user/one-mk.service" > "$scratch/one-mk.service"
    if (cd "$scratch" && systemd-analyze --user verify "$scratch/one-mk.service" 2>&1 | grep -v "spice-vdagent" | grep -q "one-mk.service"); then
        bad "systemd-analyze --user verify complains about the unit"
    else ok "systemd user unit verifies"; fi
else skip "systemd-analyze not installed"; fi
if command -v lintian >/dev/null; then
    lintian --no-tag-display-limit "$deb" && ok "lintian" || bad "lintian"
else skip "lintian not installed"; fi

OUT_DIR=$scratch/c APPIMAGETOOL= sh "$ROOT/scripts/package-appimage.sh" >/dev/null 2>&1
got=$("$scratch/c/one-mk.AppDir/AppRun" --version) && [ "$got" = "one-mk $VERSION" ] && ok "AppDir: AppRun --version: $got" || bad "AppDir AppRun --version"
[ -f "$scratch/c/one-mk.AppDir/one-mk.desktop" ] && [ -f "$scratch/c/one-mk.AppDir/one-mk.png" ] && [ -L "$scratch/c/one-mk.AppDir/.DirIcon" ] && ok "AppDir has desktop file and icons at the top" || bad "AppDir top level"

[ "$fail" = 0 ] && echo "all checks passed" || { echo "some checks failed"; exit 1; }
