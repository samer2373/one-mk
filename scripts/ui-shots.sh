#!/usr/bin/env bash
# Screenshots of every state of the one·mk window, for design review.
# Runs local test daemons (no real input) in throwaway settings folders.
# Usage: scripts/ui-shots.sh [OUT_DIR]   (default /tmp/omk-shots)
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
    env "$@" ONE_MK_APP_SHOT="$W/$name.ppm" ONE_MK_APP_SHOT_AFTER="$after" timeout 30 "$APP" --home "$home" || true
    python3 -c "from PIL import Image; Image.open('$W/$name.ppm').save('$OUT/$name.png')"
    echo "$OUT/$name.png"
}

# Pair a and b, then call b "Studio PC" as a sees it.
(yes yes | timeout 30 "$B" --home "$W/a" pair --machine >/dev/null 2>&1 &)
(yes yes | timeout 30 "$B" --home "$W/b" pair --machine >/dev/null 2>&1 &)
for _ in $(seq 60); do grep -q '^name' "$W/a/devices.toml" 2>/dev/null && break; sleep 0.25; done
sleep 1
sed -i '0,/^name = .*/s//name = "Studio PC"/' "$W/a/devices.toml"
"$B" --home "$W/a" layout "Studio PC" left >/dev/null

"$B" --home "$W/a" --backend null --no-discovery --port 24821 --connect 127.0.0.1:24822 >"$W/da.log" 2>&1 & PIDS+=($!)
"$B" --home "$W/b" --backend null --no-discovery --port 24822 --connect 127.0.0.1:24821 >"$W/db.log" 2>&1 & B_PID=$!; PIDS+=($B_PID)
"$B" --home "$W/e" --backend null --no-discovery --port 24823 >"$W/de.log" 2>&1 & PIDS+=($!)
sleep 4

shot 01-intro 0.4 "$W/a"
shot 02-probe-crossing 1.5 "$W/a"
shot 03-desk-connected 4 "$W/a"
shot 04-files-tab 3 "$W/a" ONE_MK_APP_TAB=files
shot 05-keyboard-tab 3 "$W/a" ONE_MK_APP_TAB=keyboard
shot 06-small-window 4 "$W/a" ONE_MK_APP_SIZE=760x540
shot 07-large-window 4 "$W/a" ONE_MK_APP_SIZE=1440x900
shot 08-nothing-paired 3 "$W/e"
shot 09-not-running 3 "$W/off"
# Pairing: a third computer waits with `one-mk pair` (it never answers).
(sleep 60 | timeout 40 "$B" --home "$W/c" pair --machine >/dev/null 2>&1 &)
shot 10-pairing-code 6 "$W/a" ONE_MK_APP_PAIR=1
shot 11-pairing-searching 1.2 "$W/e" ONE_MK_APP_PAIR=1
kill "$B_PID"; sleep 3
shot 12-peer-offline 3 "$W/a"
