#!/bin/sh
# Builds dist/one-mk.app and dist/one-mk-<version>-<arch>.dmg from an
# existing release build. macOS only for the .dmg (hdiutil); the .app is
# plain files and can be laid out anywhere with APP_ONLY=1 (used to check
# the layout on Linux). UNTESTED on a Mac.
#   TARGET=aarch64-apple-darwin cargo build --release --locked --target $TARGET -p one-mk -p one-mk-tray -p one-mk-app
#   TARGET=aarch64-apple-darwin sh scripts/package-macos.sh
# The window is the bundle's main program; `one-mk` (the command line and
# the service) and `one-mk-tray` sit next to it in Contents/MacOS, so
# `one-mk install` registers LaunchAgents that point inside the app.
# Signing: ad-hoc by default; set MACOS_SIGN_IDENTITY="Developer ID Application: ..."
# to sign for real (notarising is not done here).
. "$(dirname "$0")/package-common.sh"

need_binaries one-mk one-mk-tray one-mk-app
case "${TARGET:-$(uname -m)}" in
    aarch64-apple-darwin|arm64|aarch64) arch=arm64 ;;
    x86_64-apple-darwin|x86_64) arch=x64 ;;
    *) die "no macOS architecture for ${TARGET:-$(uname -m)}" ;;
esac

app=$OUT_DIR/one-mk.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for b in one-mk one-mk-tray one-mk-app; do
    install -m 0755 "$BIN_DIR/$b" "$app/Contents/MacOS/$b"
done
sed "s/@VERSION@/$VERSION/" "$ROOT/packaging/macos/Info.plist.in" > "$app/Contents/Info.plist"
install -m 0644 "$ROOT/docs/brand/app-icon/macos/onemk.icns" "$app/Contents/Resources/onemk.icns"
printf 'APPL????' > "$app/Contents/PkgInfo"

if [ "${APP_ONLY:-}" = 1 ]; then
    echo "$app"
    exit 0
fi
[ "$(uname -s)" = Darwin ] || die "building the .dmg needs macOS (set APP_ONLY=1 to lay out just the .app)"

codesign --force --deep --sign "${MACOS_SIGN_IDENTITY:--}" "$app"
codesign --verify --deep --strict "$app"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
out=$OUT_DIR/one-mk-$VERSION-$arch.dmg
rm -f "$out"
hdiutil create -quiet -volname "one-mk $VERSION" -srcfolder "$stage" -ov -format UDZO "$out"
echo "$out"
