#!/bin/sh
# Wraps the built programs in one-mk.app and zips it, on macOS:
#   sh scripts/bundle-mac.sh <release dir, e.g. target/release> <out.zip>
# Also writes the matching .dmg next to the zip: with dmgbuild a designed
# installer window (scripts/dmg/), otherwise a plain image with the app and an
# Applications link (see "Disk image" below and docs/PACKAGING.md).
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
  <key>CFBundleName</key><string>oneMK</string>
  <key>CFBundleDisplayName</key><string>oneMK</string>
  <key>CFBundleIdentifier</key><string>com.one-mk.app</string>
  <key>CFBundleExecutable</key><string>one-mk-app</string>
  <key>CFBundleIconFile</key><string>onemk</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocationUsageDescription</key><string>oneMK reads your Wi-Fi network name to switch layout profiles automatically, including while its window is closed. It does not track your location.</string>
  <key>NSLocationWhenInUseUsageDescription</key><string>oneMK reads your Wi-Fi network name to switch layout profiles automatically. It does not track your location.</string>
  <key>NSLocationAlwaysAndWhenInUseUsageDescription</key><string>oneMK reads your Wi-Fi network name to switch layout profiles automatically, even while its window is closed. It does not track your location.</string>
  <key>NSLocationAlwaysUsageDescription</key><string>oneMK reads your Wi-Fi network name to switch layout profiles automatically while running in the background. It does not track your location.</string>
  <key>LSHasLocalizedDisplayName</key><true/>
</dict></plist>
PLIST
# Finder, the Dock and Launchpad show the bundle's file name unless it has
# a localized display name: they show "oneMK" while the file stays
# one-mk.app (updates and login items find it by that name).
mkdir -p "$app/Contents/Resources/en.lproj"
printf '"CFBundleDisplayName" = "oneMK";\n"CFBundleName" = "oneMK";\n' > "$app/Contents/Resources/en.lproj/InfoPlist.strings"

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

# ---- Disk image ------------------------------------------------------------
# The designed window needs dmgbuild (pip install dmgbuild; CI installs it) and
# two macOS tools it calls, SetFile (Xcode command line tools) and tiffutil.
# Without them this falls back to the plain image, so a local build still
# works. DMGBUILD=/path/to/dmgbuild picks the program; REQUIRE_DMGBUILD=1 (the
# release workflow sets it) makes a missing one an error instead, so a release
# never ships the plain window by accident; ONE_MK_PLAIN_DMG=1 forces the plain
# image. A dmgbuild that is present but fails is always an error.
root=$(pwd)
dmgbuild_cmd=${DMGBUILD:-$(command -v dmgbuild || true)}
use_dmgbuild=1
why=
if [ -n "${ONE_MK_PLAIN_DMG:-}" ]; then
  use_dmgbuild=0; why="ONE_MK_PLAIN_DMG is set"
elif [ -z "$dmgbuild_cmd" ] || [ ! -x "$dmgbuild_cmd" ]; then
  use_dmgbuild=0; why="dmgbuild is not installed (pip install dmgbuild)"
elif [ ! -x /usr/bin/SetFile ] || [ ! -x /usr/bin/tiffutil ]; then
  use_dmgbuild=0; why="SetFile or tiffutil is missing (install the Xcode command line tools: xcode-select --install)"
fi
if [ "$use_dmgbuild" = 0 ] && [ -n "${REQUIRE_DMGBUILD:-}" ] && [ -z "${ONE_MK_PLAIN_DMG:-}" ]; then
  echo "error: REQUIRE_DMGBUILD is set but $why" >&2
  exit 1
fi

rm -f "$dmg"
if [ "$use_dmgbuild" = 1 ]; then
  # settings.py puts the app and the Applications link where the artwork has its
  # glows, and the background pair next to it (background.png, background@2x.png).
  "$dmgbuild_cmd" -s "$root/scripts/dmg/settings.py" \
    -D "app=$app" \
    -D "icon=$root/docs/brand/app-icon/macos/onemk.icns" \
    -D "background=$root/scripts/dmg/background.png" \
    "oneMK" "$dmg"
else
  echo "note: writing the plain .dmg without the designed window: $why" >&2
  mkdir "$tmp/dmg"
  cp -R "$app" "$tmp/dmg/"
  ln -s /Applications "$tmp/dmg/Applications"
  hdiutil create -quiet -volname "oneMK" -srcfolder "$tmp/dmg" -ov -format UDZO "$dmg"
fi

# Open what was built before it is signed: the app must be in it, intact, next to
# the Applications link (dmgbuild ignores a failed copy), and the designed
# window's layout and background must be there.
mnt="$tmp/mnt"
mkdir "$mnt"
hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$mnt" "$dmg" >/dev/null
rc=0
[ -d "$mnt/one-mk.app/Contents/MacOS" ] && [ -L "$mnt/Applications" ] || rc=1
codesign --verify --deep --strict "$mnt/one-mk.app" || rc=1
if [ "$use_dmgbuild" = 1 ]; then
  [ -f "$mnt/.DS_Store" ] && [ -n "$(ls "$mnt"/.background.* 2>/dev/null)" ] || rc=1
fi
hdiutil detach -quiet "$mnt" || hdiutil detach -force -quiet "$mnt" || true
if [ "$rc" != 0 ]; then
  echo "error: $dmg is missing the app, the Applications link or the window layout" >&2
  exit 1
fi

# The image is signed last: anything that changed it afterwards would break it.
[ -z "${SIGN_IDENTITY:-}" ] || codesign --force --timestamp --sign "$SIGN_IDENTITY" "$dmg"
echo "Wrote $out and $dmg"
