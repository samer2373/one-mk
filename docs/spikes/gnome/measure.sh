#!/bin/bash
# Spike: measure for SECS seconds: renderer fps (its own counter), CPU% of the
# isolated gnome-shell and of the renderer (utime+stime / wall, one core =
# 100%), and damage events/s seen by the extension.
H=$(cd "$(dirname "$0")" && pwd)
label=$1; secs=${2:-10}
shell=$(pgrep -f '^gnome-shell --wayland --headless' | head -1)
client=$(tail -1 /tmp/omk-gs-spike/log/fps.log | sed -n 's/.*pid=\([0-9]*\).*/\1/p')
tick=$(getconf CLK_TCK)
cpu() { [ -n "$1" ] && [ -r /proc/$1/stat ] && awk '{print $14+$15}' /proc/$1/stat || echo 0; }
dmg() { "$H/S.sh" eval 'String(Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge.stats.damage)' | tr -dc 0-9; }
l0=$(wc -l < /tmp/omk-gs-spike/log/fps.log); s0=$(cpu $shell); c0=$(cpu $client); d0=$(dmg); t0=$(date +%s.%N)
sleep "$secs"
s1=$(cpu $shell); c1=$(cpu $client); d1=$(dmg); t1=$(date +%s.%N)
fps=$(tail -n +$((l0+2)) /tmp/omk-gs-spike/log/fps.log | grep '^t=' | sed -n 's/.*fps=\([0-9.]*\).*/\1/p' | awk '{s+=$1;n++} END {if(n) printf "%.1f (min %s over %d s)", s/n, "-", n; else print "n/a"}')
min=$(tail -n +$((l0+2)) /tmp/omk-gs-spike/log/fps.log | grep '^t=' | sed -n 's/.*fps=\([0-9.]*\).*/\1/p' | sort -n | head -1)
awk -v l="$label" -v s0=$s0 -v s1=$s1 -v c0=$c0 -v c1=$c1 -v d0=$d0 -v d1=$d1 -v t0=$t0 -v t1=$t1 -v tk=$tick -v fps="${fps/- /$min }" \
 'BEGIN{w=t1-t0; printf "%-44s client fps %-26s shell CPU %5.1f%%  client CPU %5.1f%%  damage %5.1f/s\n", l, fps, 100*(s1-s0)/tk/w, 100*(c1-c0)/tk/w, (d1-d0)/w}'
