#!/bin/sh
# Wraps the built programs in one-mk.app and zips it, on macOS:
#   sh scripts/bundle-mac.sh <release dir, e.g. target/release> <out.zip>
# Also writes the matching .dmg next to the zip (drag one·mk onto Applications).
# The app is ad-hoc signed; a Developer ID would be needed to notarize it.
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

codesign --force --deep --sign - "$app"
rm -f "$out"
ditto -c -k --norsrc --noextattr --keepParent "$app" "$out"
dmg="${out%.app.zip}.dmg"
mkdir "$tmp/dmg"
cp -R "$app" "$tmp/dmg/"
ln -s /Applications "$tmp/dmg/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname "one·mk" -srcfolder "$tmp/dmg" -ov -format UDZO "$dmg"
echo "Wrote $out and $dmg"
