#!/bin/sh
# Copies the licence files every oneMK package ships into a folder:
#   sh scripts/copy-notices.sh <folder>
# LICENSE-MIT and LICENSE-APACHE (oneMK's own), THIRD-PARTY-NOTICES (every
# crate built in, with its licence texts: scripts/third-party-notices.py) and
# OFL-Inter.txt, OFL-JetBrainsMono.txt and OFL-Unbounded.txt (the licences of
# the fonts built into the window). Used by the
# .deb, AppImage, macOS app, Windows installer and the release archives, so
# they all carry the same set. Fails if one is missing.
set -eu
[ $# -eq 1 ] || { echo "usage: $0 <folder>" >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$1"
for file in LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES; do
    [ -s "$root/$file" ] || { echo "copy-notices.sh: $root/$file is missing; run python3 scripts/third-party-notices.py" >&2; exit 1; }
    cp "$root/$file" "$1/$file"
    chmod 644 "$1/$file"
done
for font in Inter JetBrainsMono Unbounded; do
    cp "$root/crates/app/fonts/OFL-$font.txt" "$1/OFL-$font.txt"
    chmod 644 "$1/OFL-$font.txt"
done
