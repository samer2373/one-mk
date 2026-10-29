#!/bin/sh
# Writes dist/homebrew/one-mk.rb (cask) and dist/homebrew/one-mk-source.rb
# (formula) for this version, using the checksums of the release's two .dmg
# files in dist/ (one-mk-aarch64-apple-darwin.dmg and
# one-mk-x86_64-apple-darwin.dmg, made by scripts/bundle-mac.sh) and of the
# source tarball (SOURCE_SHA256, or `git archive` of HEAD). UNTESTED.
. "$(dirname "$0")/package-common.sh"

sum() { sha256sum "$1" | cut -d' ' -f1; }
arm=$OUT_DIR/one-mk-aarch64-apple-darwin.dmg
x64=$OUT_DIR/one-mk-x86_64-apple-darwin.dmg
[ -f "$arm" ] && [ -f "$x64" ] || die "need $arm and $x64 (the release's two .dmg files)"
src=${SOURCE_SHA256:-$(git -C "$ROOT" archive --format=tar.gz --prefix="one-mk-$VERSION/" HEAD | sha256sum | cut -d' ' -f1)}

mkdir -p "$OUT_DIR/homebrew"
sed -e "s/@VERSION@/$VERSION/" -e "s/@SHA256_ARM64@/$(sum "$arm")/" -e "s/@SHA256_X64@/$(sum "$x64")/" \
    "$ROOT/packaging/homebrew/one-mk.rb.in" > "$OUT_DIR/homebrew/one-mk.rb"
sed -e "s/@VERSION@/$VERSION/" -e "s/@SHA256_SOURCE@/$src/" \
    "$ROOT/packaging/homebrew/one-mk-source.rb.in" > "$OUT_DIR/homebrew/one-mk-source.rb"
echo "$OUT_DIR/homebrew/one-mk.rb"
echo "$OUT_DIR/homebrew/one-mk-source.rb"
