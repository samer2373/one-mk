#!/usr/bin/env bash
# Screenshots of every state of the one·mk window, for design review.
# Runs local test daemons (no real input) in throwaway settings folders.
# Usage: scripts/ui-shots.sh [OUT_DIR]   (default /tmp/omk-shots)
# The debug-only ONE_MK_APP_* hooks it uses are listed in crates/app/src/app.rs.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${1:-/tmp/omk-shots}
W=/tmp/omk-shots-work
cargo build -q -p one-mk -p one-mk-app -j 4
B=$PWD/target/debug/one-mk
APP=$PWD/target/debug/one-mk-app
rm -rf "$W" && mkdir -p "$OUT" && mkdir -m 700 -p "$W/a" "$W/b" "$W/c" "$W/e" "$W/off"
PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

shot() { # name after-seconds home [extra env...]
    local name=$1 after=$2 home=$3; shift 3
    # The compositor sometimes never draws a fresh window (e.g. covered or
    # on another workspace); try again rather than fail the whole run.
    for _ in 1 2 3; do
        rm -f "$W/$name.ppm"
        env "$@" ONE_MK_APP_SHOT="$W/$name.ppm" ONE_MK_APP_SHOT_AFTER="$after" timeout 20 "$APP" --home "$home" || true
        [ -s "$W/$name.ppm" ] && break
    done
    python3 -c "from PIL import Image; Image.open('$W/$name.ppm').save('$OUT/$name.png')"
    echo "$OUT/$name.png"
}

# Pair a and b, then call b "Studio PC" as a sees it. Discovery on a
# busy network sometimes misses; try a few times.
for _ in 1 2 3; do
    (yes yes | timeout 20 "$B" --home "$W/a" pair --machine >/dev/null 2>&1 &)
    (yes yes | timeout 20 "$B" --home "$W/b" pair --machine >/dev/null 2>&1 &)
    for _ in $(seq 80); do grep -q '^name' "$W/a/devices.toml" 2>/dev/null && break 2; sleep 0.25; done
    pkill -f "home $W/[ab] pair" || true; sleep 1
done
sleep 1
sed -i '0,/^name = .*/s//name = "Studio PC"/' "$W/a/devices.toml"
"$B" --home "$W/a" layout "Studio PC" left >/dev/null

"$B" --home "$W/a" --backend null --no-discovery --port 24821 --connect 127.0.0.1:24822 >"$W/da.log" 2>&1 & PIDS+=($!)
"$B" --home "$W/b" --backend null --no-discovery --port 24822 --connect 127.0.0.1:24821 >"$W/db.log" 2>&1 & B_PID=$!; PIDS+=($B_PID)
"$B" --home "$W/e" --backend null --no-discovery --port 24823 >"$W/de.log" 2>&1 & PIDS+=($!)
sleep 4

shot 01-intro 0.4 "$W/a"
shot 02-probe-crossing 2.1 "$W/a"
shot 03-desk-connected 4 "$W/a"
shot 04-files-tab 3 "$W/a" ONE_MK_APP_TAB=files
shot 05-keyboard-tab 3 "$W/a" ONE_MK_APP_TAB=keyboard
shot 06-small-window 4 "$W/a" ONE_MK_APP_SIZE=760x540
shot 07-large-window 4 "$W/a" ONE_MK_APP_SIZE=1300x800
shot 08-nothing-paired 3 "$W/e"
shot 09-not-running 3 "$W/off"
# Pairing: a third computer waits with `one-mk pair` (it never answers).
(sleep 60 | timeout 40 "$B" --home "$W/c" pair --machine >/dev/null 2>&1 &)
shot 10-pairing-code 6 "$W/a" ONE_MK_APP_PAIR=1
shot 11-pairing-searching 1.2 "$W/e" ONE_MK_APP_PAIR=1
shot 13-this-computer-selected 3 "$W/a" ONE_MK_APP_SELECT=me ONE_MK_APP_AUTOSTART=1
shot 14-mid-drag 2.4 "$W/a" ONE_MK_APP_DRAG=1
shot 15-keyboard-focus 2 "$W/a" ONE_MK_APP_FOCUS=1
shot 16-error-toast 1.6 "$W/a" ONE_MK_APP_TOAST=1
kill "$B_PID"; sleep 3
shot 12-peer-offline 3 "$W/a"

# A third computer pairs and confirms; the window says the codes match.
(yes yes | timeout 40 "$B" --home "$W/c" pair --machine >/dev/null 2>&1 &)
shot 22-paired 9 "$W/a" ONE_MK_APP_PAIR=1 ONE_MK_APP_CONFIRM=1
shot 23-reduced-motion-intro 0.3 "$W/a" ONE_MK_REDUCE_MOTION=1

# States that need more computers than two test daemons: a status file
# stands in for the daemon (debug builds only).
F=$W/many.status
LONG="Samer's extremely long workstation name that goes on and on and on"
printf 'ok\nname\tsamer-Victus\nid\tabcd-ef01\nport\t24800\n' >"$F"
printf 'peer\t1111-2222\tleft\t3\t1\tauto\tMacBook Air\n' >>"$F"
printf 'peer\t3333-4444\tright\t1\t1\ton\t%s\n' "$LONG" >>"$F"
printf 'peer\t5555-6666\ttop\t2\t0\toff\tGaming PC\n' >>"$F"
printf 'peer\t7777-8888\tbottom\t1\t1\tauto\tМастерская\n' >>"$F"
printf 'peer\t9999-0000\tright\t2\t1\tauto\t工作站\n' >>"$F"
shot 17-many-long-names 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_SELECT="$LONG"
shot 18-many-keyboard 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_TAB=keyboard
shot 19-drop-chooser 2.2 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_DROP=3
shot 20-many-small 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_SIZE=760x540 ONE_MK_APP_SELECT="$LONG"
C=$W/controlled.status
printf 'ok\nname\tsamer-Victus\nid\tabcd-ef01\nport\t24800\ncontrolled_by\tStudio PC\n' >"$C"
printf 'peer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\n' >>"$C"
shot 21-controlled-small 3 "$W/off" ONE_MK_APP_STATUS="$C" ONE_MK_APP_SIZE=760x540
