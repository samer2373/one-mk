#!/bin/sh
# Builds dist/one-mk-<arch>-unknown-linux-gnu.AppImage from an existing
# release build (named like the release's other Linux files):
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
# An AppImage is not installed: no service file, autostart, man pages or
# hidden app-id entry. (`one-mk install` writes those into the home folder,
# pointing at the AppImage file.)
mkdir -p "$appdir/usr/bin"
for b in one-mk one-mk-tray one-mk-app; do
    install -m 0755 "$BIN_DIR/$b" "$appdir/usr/bin/$b"
done
install -Dm 0644 "$ROOT/packaging/linux/one-mk.desktop" "$appdir/usr/share/applications/one-mk.desktop"
mkdir -p "$appdir/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/" -e "s/@DATE@/$(release_date)/" "$ROOT/packaging/linux/io.github.onemk.OneMk.metainfo.xml" \
    > "$appdir/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml"
icons=$ROOT/docs/brand/app-icon/windows-linux/hicolor
(cd "$icons" && find . -type f \( -name '*.png' -o -name '*.svg' \)) | while read -r f; do
    install -Dm 0644 "$icons/$f" "$appdir/usr/share/icons/hicolor/$f"
done
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

out=$OUT_DIR/one-mk-$arch-unknown-linux-gnu.AppImage
tool=${APPIMAGETOOL-$(command -v appimagetool || true)}
if [ -z "$tool" ]; then
    echo "appimagetool not found: stopped after the AppDir. Get it from https://github.com/AppImage/appimagetool/releases" >&2
    echo "and run:  ARCH=$arch appimagetool --no-appstream $appdir $out" >&2
    exit 0
fi
# --appimage-extract-and-run lets appimagetool itself run where FUSE is missing (CI containers).
ARCH=$arch "$tool" --appimage-extract-and-run --no-appstream "$appdir" "$out" >/dev/null 2>&1 || ARCH=$arch "$tool" --no-appstream "$appdir" "$out"
echo "$out"
