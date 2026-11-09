#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${BIN_DIR:-$root/target/release}/one-mk-wallpaper
output=${1:-$root/target/wallpaper-gnome-dry-run}
test -f "$binary"
test ! -e "$output" || { echo "refusing to overwrite $output" >&2; exit 1; }
mkdir -p "$output/libexec" "$output/share/gnome-shell/extensions"
cp "$binary" "$output/libexec/one-mk-wallpaper"
cp -R "$root/extensions/gnome/one-mk-wallpaper@one-mk.org" "$output/share/gnome-shell/extensions/"
cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$output/"
printf '%s\n' 'OPTIONAL GNOME COMPONENT. Default OFF. No automatic install or enable.' \
    'GNOME 50 (experimental): GStreamer video runs in the separate oneMK wallpaper helper, never in Shell.' \
    'The Wallpaper page offers separate, consented Install and Enable actions. No download is required.' \
    'Log out and back in once after installation, then Enable and choose a video. Off restores the desktop.' \
    'See docs/reviews/GNOME-WAYLAND-VIDEO.md for qualification and safety details.' > "$output/README.txt"
(cd "$output" && find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort | xargs sha256sum > SHA256SUMS)
(cd "$output" && sha256sum -c SHA256SUMS)
