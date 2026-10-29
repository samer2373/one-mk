#!/bin/sh
# Wraps the built programs in one-mk.app and zips it, on macOS:
#   sh scripts/bundle-mac.sh <release dir, e.g. target/release> <out.zip>
# Also writes the matching .dmg next to the zip (drag one·mk onto Applications).
# With SIGN_IDENTITY set (a Developer ID Application certificate) the app is
# signed with it, and with NOTARY_APPLE_ID, NOTARY_PASSWORD and NOTARY_TEAM_ID
# also set it is notarized and stapled, so macOS keeps its Accessibility
# permission across updates. Without them it is ad-hoc signed.
set -eu

dir=$1
out=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

app="$tmp/one-mk.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$dir/one-mk" "$dir/one-mk-tray" "$dir/one-mk-app" "$app/Contents/MacOS/"
cp docs/brand/app-icon/macos/onemk.icns "$app/Contents/Resources/onemk.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>one·mk</string>
  <key>CFBundleDisplayName</key><string>one·mk</string>
  <key>CFBundleIdentifier</key><string>com.one-mk.app</string>
  <key>CFBundleExecutable</key><string>one-mk-app</string>
  <key>CFBundleIconFile</key><string>onemk</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST

if [ -n "${SIGN_IDENTITY:-}" ]; then
  codesign --force --deep --options runtime --timestamp --sign "$SIGN_IDENTITY" "$app"
  codesign --verify --deep --strict "$app"
else
  codesign --force --deep --sign - "$app"
fi
if [ -n "${SIGN_IDENTITY:-}" ] && [ -n "${NOTARY_APPLE_ID:-}" ]; then
  ditto -c -k --norsrc --noextattr --keepParent "$app" "$tmp/notarize.zip"
  xcrun notarytool submit "$tmp/notarize.zip" --apple-id "$NOTARY_APPLE_ID" --password "$NOTARY_PASSWORD" \
    --team-id "$NOTARY_TEAM_ID" --wait | tee "$tmp/notary.txt"
  grep -q "status: Accepted" "$tmp/notary.txt"
  xcrun stapler staple "$app"
fi
rm -f "$out"
ditto -c -k --norsrc --noextattr --keepParent "$app" "$out"
dmg="${out%.app.zip}.dmg"
mkdir "$tmp/dmg"
cp -R "$app" "$tmp/dmg/"
ln -s /Applications "$tmp/dmg/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname "one·mk" -srcfolder "$tmp/dmg" -ov -format UDZO "$dmg"
[ -z "${SIGN_IDENTITY:-}" ] || codesign --force --timestamp --sign "$SIGN_IDENTITY" "$dmg"
echo "Wrote $out and $dmg"
