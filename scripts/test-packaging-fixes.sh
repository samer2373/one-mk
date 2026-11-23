#!/bin/bash
# Regression tests for the packaging scripts' edge cases. Nothing is built,
# installed or downloaded; everything runs in a scratch folder.
#   bash scripts/test-packaging-fixes.sh
# Needs python3. (scripts/check-packaging.sh builds the real packages; this
# only runs the scripts that need no build.)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/one-mk-pkg-fixes.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
fail=0
ok()  { echo "ok      $*"; }
bad() { echo "FAILED  $*"; fail=1; }

# The Homebrew formula's class name is the one Homebrew derives from the file
# name it is published under (one-mk-source.rb gives OneMkSource).
mkdir -p "$scratch/h"
echo arm > "$scratch/h/one-mk-aarch64-apple-darwin.dmg"
echo x64 > "$scratch/h/one-mk-x86_64-apple-darwin.dmg"
OUT_DIR=$scratch/h SOURCE_SHA256=0000000000000000000000000000000000000000000000000000000000000000 \
    sh "$root/scripts/package-homebrew.sh" >/dev/null
for f in "$scratch"/h/homebrew/one-mk-source.rb; do
    stem=$(basename "$f" .rb)
    class=$(echo "$stem" | awk -F- '{ for (i = 1; i <= NF; i++) printf "%s%s", toupper(substr($i, 1, 1)), substr($i, 2) }')
    grep -q "^class $class < Formula\$" "$f" && ok "the formula's class is $class, as Homebrew derives it from $stem.rb" || bad "formula class does not match $stem.rb (wants $class): $(grep '^class' "$f")"
done

# The Flatpak manifest only takes the x86-64 programs, and a local build is
# attempted only where it can work.
mkdir -p "$scratch/t" "$scratch/stubs"
echo not-really-a-tarball > "$scratch/t/one-mk-x86_64-unknown-linux-gnu.tar.gz"
printf '#!/bin/sh\necho "$@" >> "%s/builder-calls"\nexit 0\n' "$scratch" > "$scratch/stubs/flatpak-builder"
printf '#!/bin/sh\nexit 0\n' > "$scratch/stubs/flatpak"
chmod 755 "$scratch/stubs/"*
url=https://github.com/samer2373/one-mk/releases/download/v1/one-mk-x86_64-unknown-linux-gnu.tar.gz
PATH="$scratch/stubs:$PATH" OUT_DIR=$scratch/u sh "$root/scripts/package-flatpak.sh" "$scratch/t/one-mk-x86_64-unknown-linux-gnu.tar.gz" "$url" >/dev/null 2>"$scratch/err"
manifest=$scratch/u/flatpak/io.github.onemk.OneMk.yml
awk '/type: archive/ {f=1; next} f && /only-arches:/ {found = ($0 ~ /\[x86_64\]/); exit} END {exit !found}' "$manifest" \
    && ok "the Flatpak manifest restricts the prebuilt archive to x86_64" || bad "no only-arches: [x86_64] on the archive source"
if [ ! -e "$scratch/builder-calls" ] && grep -q "not published yet" "$scratch/err"; then
    ok "with a release URL that is not public yet, no local Flatpak build is started (the bundle is still made)"
else bad "flatpak-builder ran against an unpublished URL: $(cat "$scratch/builder-calls" 2>/dev/null) $(cat "$scratch/err")"; fi
[ -f "$scratch/u/one-mk-flatpak-sources.tar.gz" ] && ok "the source bundle is made without a local build" || bad "no source bundle"
if [ "$(uname -m)" = x86_64 ]; then
    PATH="$scratch/stubs:$PATH" OUT_DIR=$scratch/l sh "$root/scripts/package-flatpak.sh" "$scratch/t/one-mk-x86_64-unknown-linux-gnu.tar.gz" >/dev/null 2>&1
    [ -s "$scratch/builder-calls" ] && ok "a local tarball still gets the local build when flatpak-builder is there" || bad "the local build was not started"
fi

# The Mac bundler refuses an output name that would make the disk image replace the zip,
# before it builds anything (so this runs anywhere).
if (cd "$root" && sh scripts/bundle-mac.sh "$scratch" "$scratch/out.zip") > "$scratch/mac" 2>&1; then bad "bundle-mac.sh accepted out.zip"
elif grep -q "NAME.app.zip" "$scratch/mac" && [ ! -e "$scratch/out.zip" ] && [ ! -e "$scratch/out.dmg" ]; then ok "bundle-mac.sh refuses an output that is not NAME.app.zip, and writes nothing"
else bad "bundle-mac.sh out.zip: $(cat "$scratch/mac")"; fi

# The update list is not signed when a release version has a number the updater cannot read (a u64).
mkdir -p "$scratch/stage"
echo asset > "$scratch/stage/one-mk-x86_64-unknown-linux-gnu.bin"
for revoked in "99999999999999999999.0.0" "1.18446744073709551616.0" "0.0.18446744073709551616"; do
    if (cd "$root" && ONE_MK_REVOKED="$revoked" RELEASE_TIME=1793318400 bash scripts/make-update-manifest.sh "$scratch/stage" 5) > "$scratch/mm" 2>&1; then bad "a revoked version $revoked was listed"
    elif grep -q "too large for the updater" "$scratch/mm" && [ ! -e "$scratch/stage/update-manifest.json" ]; then ok "revoking $revoked is refused: the updater could not read the list"
    else bad "revoked $revoked: $(cat "$scratch/mm")"; fi
done
(cd "$root" && ONE_MK_REVOKED="0.0.18446744073709551615" RELEASE_TIME=1793318400 bash scripts/make-update-manifest.sh "$scratch/stage" 5) >/dev/null 2>&1 \
    && ok "the largest readable version number is still accepted" || bad "a u64::MAX component was refused"

exit $fail
