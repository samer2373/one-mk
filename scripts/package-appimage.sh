#!/bin/sh
# Builds dist/one-mk-<version>-<arch>.AppImage from an existing release build.
#   cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
#   sh scripts/package-appimage.sh
# Needs appimagetool (set APPIMAGETOOL=/path, or have it on PATH). Without it
# the script stops after laying out dist/one-mk.AppDir, which is checked by
# running `AppRun --version`, and says so.
. "$(dirname "$0")/package-common.sh"

need_binaries one-mk one-mk-tray one-mk-app
case "$(uname -m)" in
    x86_64) arch=x86_64 ;;
    aarch64) arch=aarch64 ;;
    *) die "no AppImage architecture for $(uname -m)" ;;
esac

appdir=$OUT_DIR/one-mk.AppDir
rm -rf "$appdir"
linux_tree "$appdir" /usr/bin
# An AppImage is not installed: no service file, autostart or man pages.
# (`one-mk install` writes those into the home folder, pointing at the
# AppImage file.)
rm -rf "$appdir/usr/lib" "$appdir/etc" "$appdir/usr/share/man" "$appdir/usr/share/doc"
install -Dm 0644 "$ROOT/packaging/linux/copyright" "$appdir/usr/share/doc/one-mk/copyright"
install -m 0755 "$ROOT/packaging/linux/AppRun" "$appdir/AppRun"
# The files appimagetool wants at the top.
cp "$appdir/usr/share/applications/one-mk.desktop" "$appdir/one-mk.desktop"
cp "$appdir/usr/share/icons/hicolor/256x256/apps/one-mk.png" "$appdir/one-mk.png"
ln -sf one-mk.png "$appdir/.DirIcon"
clamp_times "$appdir"

# The cheapest real test of the layout: it runs.
got=$("$appdir/AppRun" --version) || die "AppRun --version failed"
[ "$got" = "one-mk $VERSION" ] || die "AppRun --version said '$got', expected 'one-mk $VERSION'"
echo "AppDir ok: $appdir ($got)"

tool=${APPIMAGETOOL-$(command -v appimagetool || true)}
if [ -z "$tool" ]; then
    echo "appimagetool not found: stopped after the AppDir. Get it from https://github.com/AppImage/appimagetool/releases" >&2
    echo "and run:  ARCH=$arch appimagetool $appdir $OUT_DIR/one-mk-$VERSION-$arch.AppImage" >&2
    exit 0
fi
out=$OUT_DIR/one-mk-$VERSION-$arch.AppImage
# --appimage-extract-and-run lets appimagetool itself run where FUSE is missing (CI containers).
ARCH=$arch "$tool" --appimage-extract-and-run --no-appstream "$appdir" "$out" >/dev/null 2>&1 || ARCH=$arch "$tool" --no-appstream "$appdir" "$out"
echo "$out"
