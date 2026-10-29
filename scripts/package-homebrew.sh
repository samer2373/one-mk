#!/bin/sh
# Writes dist/homebrew/one-mk.rb (cask) and dist/homebrew/one-mk-source.rb
# (formula) for this version, using the checksums of the .dmg files and the
# source tarball in dist/ (or SOURCE_SHA256 for the latter). UNTESTED.
. "$(dirname "$0")/package-common.sh"

sum() { sha256sum "$1" | cut -d' ' -f1; }
arm=$OUT_DIR/one-mk-$VERSION-arm64.dmg
x64=$OUT_DIR/one-mk-$VERSION-x64.dmg
[ -f "$arm" ] && [ -f "$x64" ] || die "need $arm and $x64 (the release's two .dmg files)"
src=${SOURCE_SHA256:-$(git -C "$ROOT" archive --format=tar.gz --prefix="one-mk-$VERSION/" HEAD | sha256sum | cut -d' ' -f1)}

mkdir -p "$OUT_DIR/homebrew"
sed -e "s/@VERSION@/$VERSION/" -e "s/@SHA256_ARM64@/$(sum "$arm")/" -e "s/@SHA256_X64@/$(sum "$x64")/" \
    "$ROOT/packaging/homebrew/one-mk.rb.in" > "$OUT_DIR/homebrew/one-mk.rb"
sed -e "s/@VERSION@/$VERSION/" -e "s/@SHA256_SOURCE@/$src/" \
    "$ROOT/packaging/homebrew/one-mk-source.rb.in" > "$OUT_DIR/homebrew/one-mk-source.rb"
echo "$OUT_DIR/homebrew/one-mk.rb"
echo "$OUT_DIR/homebrew/one-mk-source.rb"
