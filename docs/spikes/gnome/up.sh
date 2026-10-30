#!/bin/bash
# Spike: (re)start the isolated session with N monitors and enable the test
# harness, DING and the spike extension via the normal D-Bus EnableExtension.
set -e
H=$(cd "$(dirname "$0")" && pwd)
"$H/isolated-session.sh" stop >/dev/null
"$H/isolated-session.sh" start "${1:-1}"
cp "$H/client/wall_spike_client.py" /tmp/omk-gs-spike/client.py
"$H/S.sh" ext enable spike-harness@one-mk.test >/dev/null
"$H/S.sh" eval 'Main.overview.hide(); Main.messageTray._bannerBin.hide(); "overview hidden"'
"$H/S.sh" ext enable ding@rastersoft.com >/dev/null
"$H/S.sh" ext enable wallpaper-spike@one-mk.test >/dev/null
sleep 3
for u in ding@rastersoft.com wallpaper-spike@one-mk.test; do
	echo "$u: $("$H/S.sh" info $u | tr ',' '\n' | grep -E "'state'|'error'" | tr '\n' ' ')"
done
