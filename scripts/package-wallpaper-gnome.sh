#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${BIN_DIR:-$root/target/release}/one-mk-wall
output=${1:-$root/target/wallpaper-gnome-dry-run}
test -f "$binary"
test ! -e "$output" || { echo "refusing to overwrite $output" >&2; exit 1; }
mkdir -p "$output/libexec" "$output/share/gnome-shell/extensions"
cp "$binary" "$output/libexec/one-mk-wall"
cp -R "$root/extensions/gnome/one-mk-wallpaper@one-mk.org" "$output/share/gnome-shell/extensions/"
cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$output/"
printf '%s\n' 'NON-PUBLISHING DRY RUN. Default OFF. No automatic install or enable.' \
    'Production playback blocked: owner-authenticated daemon plan and media integration are not delivered.' \
    'Install only explicitly with: one-mk wallpaper extension install --renderer /absolute/path/to/libexec/one-mk-wall' \
    'Enabling remains a separate user action. See docs/spikes/gnome/production-component.md.' > "$output/README.txt"
(cd "$output" && find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort | xargs sha256sum > SHA256SUMS)
(cd "$output" && sha256sum -c SHA256SUMS)
