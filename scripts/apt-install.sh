#!/bin/sh
# Installs packages in CI, surviving a mirror that does not answer. The Azure
# Ubuntu mirror ("azure.archive.ubuntu.com") sometimes fails to resolve for
# minutes on end and used to fail whole releases: retry a few times, then use
# the main Ubuntu archive instead.
# Usage: scripts/apt-install.sh PACKAGE...
set -u
[ "$#" -gt 0 ] || { echo "usage: $0 PACKAGE..." >&2; exit 2; }
install() { sudo apt-get update && sudo apt-get install -y --no-install-recommends "$@"; }
attempt=1
while [ "$attempt" -le 4 ]; do
    install "$@" && exit 0
    echo "apt attempt $attempt failed; trying again" >&2
    attempt=$((attempt + 1))
    sleep 20
done
echo "the mirror is not answering; using archive.ubuntu.com" >&2
for file in /etc/apt/sources.list /etc/apt/sources.list.d/*; do
    [ -f "$file" ] && sudo sed -i -e 's|mirror+file:/etc/apt/apt-mirrors.txt|http://archive.ubuntu.com/ubuntu/|' -e 's|http://azure.archive.ubuntu.com/ubuntu/\?|http://archive.ubuntu.com/ubuntu/|' "$file"
done
install "$@"
