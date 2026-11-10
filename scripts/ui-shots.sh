#!/usr/bin/env bash
# Screenshots of every state of the oneMK window, for design review, in the
# light and the dark look (each shot is NAME-light.png and NAME-dark.png).
# Everything is private: its own X server (Xvfb on a free display, never the
# real session), `--backend null` daemons with no discovery on loopback, and
# a throwaway HOME/XDG_*. Needs Xvfb and python3 with Pillow.
# Usage: scripts/ui-shots.sh [OUT_DIR]   (default /tmp/omk-shots)
#        THEMES=dark scripts/ui-shots.sh   (one look only)
# The debug-only ONE_MK_APP_* hooks it uses are listed in crates/app/src/app.rs.
# The pictures in docs/screens come from scripts/doc-shots.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=$(realpath -m "${1:-/tmp/omk-shots}")
THEMES=${THEMES:-light dark}
cargo build -q -p one-mk -p one-mk-app -j 2
T=${CARGO_TARGET_DIR:-$PWD/target}/debug
B=$T/one-mk
APP=$T/one-mk-app
# Unix control sockets need a short path, so the homes are under /tmp.
W=$(mktemp -d /tmp/omk-shots.XXXXXX)
mkdir -p "$OUT" && mkdir -m 700 -p "$W/a" "$W/b" "$W/c" "$W/e" "$W/off" "$W/fx" "$W/x/c" "$W/x/d" "$W/x/k" "$W/x/r"
export HOME=$W/x XDG_CONFIG_HOME=$W/x/c XDG_DATA_HOME=$W/x/d XDG_CACHE_HOME=$W/x/k XDG_RUNTIME_DIR=$W/x/r
unset WAYLAND_DISPLAY
export DBUS_SESSION_BUS_ADDRESS=unix:path=$W/x/no-bus
PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; pkill -f "one-mk-tray --home $W/" || true; sleep 0.5; rm -rf "$W"; }
trap cleanup EXIT
D=$(( 90 + RANDOM % 9 ))
Xvfb ":$D" -screen 0 2000x1300x24 -nolisten tcp >"$W/xvfb.log" 2>&1 & PIDS+=($!)
sleep 2
export DISPLAY=":$D" LIBGL_ALWAYS_SOFTWARE=1 ONE_MK_APP_NO_SETUP=1

shot() { # name after-seconds home [extra env...]
    local name=$1 after=$2 home=$3 theme; shift 3
    for theme in $THEMES; do
        for _ in 1 2 3; do
            rm -f "$W/$name.ppm"
            env ONE_MK_APP_SSID="Home Wi-Fi" ONE_MK_APP_THEME="$theme" "$@" ONE_MK_APP_SHOT="$W/$name.ppm" ONE_MK_APP_SHOT_AFTER="$after" \
                timeout 30 "$APP" --home "$home" >/dev/null 2>&1 || true
            [ -s "$W/$name.ppm" ] && break
        done
        python3 -I -c "from PIL import Image; Image.open('$W/$name.ppm').convert('RGB').save('$OUT/$name-$theme.png', optimize=True)"
        echo "$OUT/$name-$theme.png"
    done
}

# Pair a and b over loopback, no discovery: a prints its pairing port; then
# call b "Studio PC" as a sees it.
( timeout 40 "$B" --home "$W/a" pair --no-discovery --machine < <(sleep 1; echo yes; sleep 20) >"$W/pa.log" 2>&1 & )
for _ in $(seq 40); do grep -q '^me' "$W/pa.log" 2>/dev/null && break; sleep 0.25; done
PORT=$(awk '/^me/{print $4}' "$W/pa.log")
( timeout 40 "$B" --home "$W/b" pair --no-discovery --connect "127.0.0.1:$PORT" --machine < <(sleep 1; echo yes; sleep 20) >"$W/pb.log" 2>&1 & )
for _ in $(seq 80); do grep -q '^name' "$W/a/devices.toml" 2>/dev/null && grep -q '^name' "$W/b/devices.toml" 2>/dev/null && break; sleep 0.25; done
sleep 1
sed -i '0,/^name = .*/s//name = "Studio PC"/' "$W/a/devices.toml"
"$B" --home "$W/a" layout "Studio PC" left >/dev/null

"$B" --home "$W/a" run --backend null --no-discovery --port 24821 --connect 127.0.0.1:24822 >"$W/da.log" 2>&1 & PIDS+=($!)
"$B" --home "$W/b" run --backend null --no-discovery --port 24822 --connect 127.0.0.1:24821 >"$W/db.log" 2>&1 & B_PID=$!; PIDS+=($B_PID)
"$B" --home "$W/e" run --backend null --no-discovery --port 24823 >"$W/de.log" 2>&1 & PIDS+=($!)
sleep 4

shot 01-intro 0.4 "$W/a"
shot 03-desk-connected 4 "$W/a"
shot 04-files-tab 3 "$W/a" ONE_MK_APP_TAB=files
shot 06-small-window 4 "$W/a" ONE_MK_APP_SIZE=820x540
shot 08-nothing-paired 3 "$W/e"
shot 09-not-running 3 "$W/off"
# Pairing, each step as the sheet shows it (no third computer: discovery
# would go out on the real network).
shot 10-pairing-code 2 "$W/a" ONE_MK_APP_PAIR_STATE=code
shot 11-pairing-searching 2 "$W/e" ONE_MK_APP_PAIR_STATE=searching
shot 11b-pairing-slow-hint 2 "$W/e" ONE_MK_APP_PAIR_STATE=hint
shot 11c-pairing-waiting 2 "$W/a" ONE_MK_APP_PAIR_STATE=waiting
shot 11d-pairing-ended 2 "$W/a" ONE_MK_APP_PAIR_STATE=ended
shot 13-this-computer-selected 3 "$W/a" ONE_MK_APP_SELECT=me ONE_MK_APP_AUTOSTART=1
# The newer pages against the real test daemons (a real `one-mk doctor`).
shot 24-layout-two-daemons 3 "$W/a" ONE_MK_APP_TAB=arrange ONE_MK_APP_SELECT="Studio PC"
shot 25-health-two-daemons 8 "$W/a" ONE_MK_APP_TAB=health
shot 26-advanced-defaults 3 "$W/a" ONE_MK_APP_TAB=settings
shot 14-mid-drag 2.4 "$W/a" ONE_MK_APP_DRAG=1
shot 31-layout-mid-drag 2.4 "$W/a" ONE_MK_APP_TAB=arrange ONE_MK_APP_DRAG=1
shot 15-keyboard-focus 2 "$W/a" ONE_MK_APP_FOCUS=1
shot 16-error-toast 1.6 "$W/a" ONE_MK_APP_TOAST=1
# The latency graph on real round trips (nothing scripted): the daemons
# have talked for a while, and the window samples once a second while it
# shows the graph, so it has built up a few readings by now.
shot 70-latency-live-two-daemons 9 "$W/a" ONE_MK_APP_SELECT="Studio PC"
kill "$B_PID"; sleep 3
shot 12-peer-offline 3 "$W/a"

shot 22-paired 2 "$W/a" ONE_MK_APP_PAIR_STATE=paired
shot 23-reduced-motion-intro 0.3 "$W/a" ONE_MK_REDUCE_MOTION=1
shot 27-health-not-running 6 "$W/off" ONE_MK_APP_TAB=health

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
# Round trips for the connected ones, as a daemon reports them (rtt in us, packets lost, sent).
for id in 1111-2222 3333-4444 7777-8888 9999-0000; do printf 'link\t%s\t4200\t0\t900\n' "$id" >>"$F"; done
shot 17-many-long-names 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_SELECT="$LONG"
shot 18-many-advanced 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_TAB=settings
shot 19-drop-chooser 2.2 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_DROP=3
C=$W/controlled.status
printf 'ok\nname\tsamer-Victus\nid\tabcd-ef01\nport\t24800\ncontrolled_by\tStudio PC\n' >"$C"
printf 'peer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\nlink\t1111-2222\t4200\t0\t900\n' >>"$C"
shot 21-controlled-small 3 "$W/off" ONE_MK_APP_STATUS="$C" ONE_MK_APP_SIZE=820x540
# Broadcast (docs/BROADCAST.md): to five computers, long names included, wide and small.
BC=$W/broadcast.status
{ printf 'ok\nname\tsamer-Victus\nid\tabcd-ef01\nport\t24800\nbroadcast_hotkey\tCtrl+Alt+B\n'
  for n in "MacBook Air" "$LONG" "Gaming PC" "Мастерская" "工作站"; do printf 'broadcast\t%s\n' "$n"; done
  grep '^peer' "$F"; } >"$BC"
shot 52-broadcasting-many 3 "$W/off" ONE_MK_APP_STATUS="$BC"
shot 53-broadcasting-small 3 "$W/off" ONE_MK_APP_STATUS="$BC" ONE_MK_APP_SIZE=820x540

# Layout, transfers, health and settings in detail: a status with several
# monitors, offsets, profiles and transfers, settings files to match, and
# a doctor report with failures.
X=$W/x.status
printf 'ok\nname\tsamer-Victus\nid\tabcd-ef01\nport\t24800\n' >"$X"
printf 'peer\t1111-2222\tleft\t3\t1\tauto\tMacBook Air\n' >>"$X"
printf 'peer\t3333-4444\tbottom\t1\t1\tauto\tStudio PC\n' >>"$X"
printf 'peer\t5555-6666\tright\t2\t0\toff\tGaming PC\n' >>"$X"
# Monitors (this computer's, then each peer's) and offsets, as the daemon
# reports them: the MacBook beside the left monitor, 120 px lower; the
# Studio PC below but 360 px clear of it (its screens touch nothing);
# the Gaming PC anywhere along the right.
printf 'link\t1111-2222\t4200\t0\t900\nlink\t3333-4444\t4200\t0\t900\n' >>"$X"
printf 'screen\tme\t0\t0\t2560\t1440\nscreen\tme\t2560\t180\t1920\t1080\n' >>"$X"
printf 'screen\t1111-2222\t0\t0\t1470\t956\n' >>"$X"
printf 'screen\t3333-4444\t0\t0\t1920\t1080\nscreen\t3333-4444\t1920\t-300\t1080\t1920\n' >>"$X"
printf 'offset\t1111-2222\t-1470\t120\noffset\t3333-4444\t320\t2100\n' >>"$X"
printf "warning\tlayout of Studio PC does not overlap yours: none of its screens touches this computer's on its bottom, so the pointer can't cross to it\n" >>"$X"
printf 'profile\thome office\nprofiles\thome office\tcafé, 2nd floor\tStudio\n' >>"$X"
printf 'gaming\tauto\nlock_sync\t1\npointer_locked\t1\n' >>"$X"
printf 'edges\tdead corners 20 px, dwell 300 ms, hold shift, fullscreen guard; left edge: no dwell\n' >>"$X"
printf 'hotkeys\ton\tthrough the GlobalShortcuts portal\n' >>"$X"
printf 'transfer\t7\tr\t3\t12300000\t27100000\t8100000\tMacBook Air\n' >>"$X"
printf 'transfer\t8\ts\t1\t500000\t9000000\t0\tStudio PC\n' >>"$X"
XI=$W/x-input.status
{ cat "$X"; printf "input\tWaiting for your approval: allow one-mk in the desktop's dialog.\n"; } >"$XI"
cat >"$W/fx/settings.toml" <<'TOML'
notify_files = false
gaming = "auto"
lock_sync = true

[clipboard]
html = false
max_mb = 10

[edge]
corner = 20
dwell_ms = 300
modifier = "shift"
fullscreen_guard = true

[edge.left]
dwell_ms = 0

[hooks]
on_enter = ["notify-send", "one-mk", "Pointer is here"]
on_peer_connect = "/home/me/bin/dim-the-other-monitor-and-switch-audio-to-the-desk-speakers --quiet"

[power]
sleep_sync = true

[hotkeys]
cycle = "ctrl+alt+n"
TOML
cat >"$W/fx/profiles.toml" <<'TOML'
active = "home office"

[[profile]]
name = "home office"
ssid = ["Home Wi-Fi"]

[[profile]]
name = "café, 2nd floor"

[[profile]]
name = "Studio"
TOML
D=$W/doctor.json
cat >"$D" <<'JSON'
{"ok":false,"summary":{"pass":5,"skip":1,"warn":1,"fail":2},"checks":[
{"id":"daemon","title":"Program","status":"pass","detail":"one-mk is running (pid 4242).","fix":null},
{"id":"config","title":"Settings","status":"pass","detail":"Found at /home/me/.config/one-mk.","fix":null},
{"id":"pairing","title":"Paired computers","status":"pass","detail":"Paired with 3 computers.","fix":null},
{"id":"port","title":"Port","status":"fail","detail":"UDP port 24900 is already used by \"barrier\".","fix":"Close that program, or use another port on both computers: `one-mk run --port 24910`."},
{"id":"mdns","title":"Discovery","status":"pass","detail":"This computer can use mDNS; 2 one-mk computers announced themselves.","fix":null},
{"id":"firewall","title":"Firewall","status":"fail","detail":"ufw is on and blocks UDP 24900.","fix":"Allow it: `sudo ufw allow 24900/udp comment one-mk`."},
{"id":"input","title":"Permissions","status":"warn","detail":"The desktop offers the needed portals, but you have not allowed one-mk yet.","fix":"Run `one-mk run` and choose Allow in the dialog your desktop shows. (If you dismissed it, run it again.)"},
{"id":"hotkeys","title":"Keyboard shortcuts","status":"skip","detail":"Not checked: one-mk isn't installed for this desktop yet.","fix":null},
{"id":"network","title":"Network","status":"pass","detail":"Connected to a private network (192.168.1.18 on wlo1).","fix":null}]}
JSON
shot 28-desk-quick-actions-transfer 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="MacBook Air"
shot 29-layout-offset-multi-monitor 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=arrange ONE_MK_APP_SELECT="MacBook Air"
shot 30-layout-edges-dont-meet 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=arrange ONE_MK_APP_SELECT="Studio PC" ONE_MK_APP_PROFILE="home office"
shot 32-layout-keyboard-focus 2 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=arrange ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_FOCUS=1
shot 33-files-transfers-clipboard 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=files
shot 34-health-failing 3 "$W/fx" ONE_MK_APP_STATUS="$XI" ONE_MK_APP_TAB=health ONE_MK_APP_DOCTOR="$D"
shot 35-advanced-settings 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=settings
shot 36-small-layout 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=arrange ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_SIZE=820x540
# Layout learning's question (docs/EDGE-PUSH.md), wide and small.
XS=$W/x-suggest.status
{ cat "$X"; printf 'suggest\t5555-6666\ttop\t"Gaming PC"\tGaming PC\n'; } >"$XS"
shot 52-layout-suggestion 3 "$W/fx" ONE_MK_APP_STATUS="$XS" ONE_MK_APP_TAB=arrange ONE_MK_APP_SELECT="Gaming PC"
shot 53-layout-suggestion-small 3 "$W/fx" ONE_MK_APP_STATUS="$XS" ONE_MK_APP_TAB=arrange ONE_MK_APP_SIZE=820x540 ONE_MK_REDUCE_MOTION=1

shot 39-small-advanced 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=settings ONE_MK_APP_SIZE=820x540
# A high-density screen: everything drawn 1.5x on top of the desktop's scale.
shot 40-hidpi-desk 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_ZOOM=1.5 ONE_MK_APP_SIZE=1140x810
shot 42-layout-nothing-paired 3 "$W/e" ONE_MK_APP_TAB=arrange

# The live latency graph (plan 5B). A history stands in for the real one
# (ONE_MK_APP_RTT: one reading a second, in ms; +n: n packets lost) so every
# state can be seen: smooth, stalls, loss, slow, still measuring, offline.
rtt() { python3 -c "
import random, sys
random.seed(7)
kind, out = sys.argv[1], []
for i in range(120):
    v = random.uniform(3.5, 6.5)
    if kind == 'stalls' and i % 17 in (9, 10): v = random.uniform(60, 95) if i % 17 == 9 else random.uniform(30, 50)
    if kind == 'loss' and i in (30, 31, 70, 98): v = random.uniform(45, 70)
    if kind == 'slow': v = random.uniform(90, 160)
    s = f'{v:.1f}'
    if kind == 'loss': s += {31: '+2', 70: '+1', 98: '+2'}.get(i, '')
    out.append(s)
print(','.join(out))" "$1"; }
SMOOTH=$(rtt smooth); STALLS=$(rtt stalls); LOSS=$(rtt loss); SLOW=$(rtt slow)
shot 71-latency-smooth 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_RTT="1111-2222=$SMOOTH;3333-4444=$STALLS"
shot 72-latency-stalls 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="Studio PC" ONE_MK_APP_RTT="1111-2222=$SMOOTH;3333-4444=$STALLS"
shot 73-latency-loss 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_RTT="1111-2222=$LOSS"
shot 74-latency-slow 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_RTT="1111-2222=$SLOW"
shot 75-latency-offline 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="Gaming PC" ONE_MK_APP_RTT="1111-2222=$SMOOTH"
shot 76-latency-measuring 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_RTT="1111-2222=4,5"
shot 77-latency-narrow 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="Studio PC" ONE_MK_APP_SIZE=820x540 ONE_MK_APP_RTT="3333-4444=$STALLS"
shot 78-latency-hidpi 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="Studio PC" ONE_MK_APP_ZOOM=1.5 ONE_MK_APP_SIZE=1140x810 ONE_MK_APP_RTT="3333-4444=$LOSS"
# A daemon from before the graph (still running after an upgrade) reports no round trips.
grep -v '^link' "$C" >"$W/older.status"
shot 82-latency-older-daemon 3 "$W/off" ONE_MK_APP_STATUS="$W/older.status"
shot 79-latency-health 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=health ONE_MK_APP_DOCTOR="$D" ONE_MK_APP_RTT="1111-2222=$SMOOTH;3333-4444=$STALLS"
shot 80-latency-health-narrow 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=health ONE_MK_APP_DOCTOR="$D" ONE_MK_APP_SIZE=820x540 ONE_MK_APP_RTT="1111-2222=$SLOW;3333-4444=$LOSS"
shot 81-latency-health-long-names 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_TAB=health ONE_MK_APP_DOCTOR="$D" ONE_MK_APP_RTT="1111-2222=$SMOOTH;3333-4444=$STALLS;7777-8888=$LOSS"

# Keys & mouse: rules written by the real `one-mk keymap|buttons|scroll`,
# so the window reads what the command line writes.
mkdir -m 700 -p "$W/k" "$W/kx"
for args in "keymap add --tap=esc --hold=lctrl -- capslock" "keymap add -- ralt layer:nav" "keymap add --layer=nav -- h left" \
    "keymap add --os=mac -- lalt lmeta" "buttons preset side-back-forward" "scroll --natural=auto --modifier=shift=horizontal"; do
    # shellcheck disable=SC2086
    "$B" --home "$W/k" $args >/dev/null
done
"$B" --home "$W/k" keymap add -- f13 "switch next" >/dev/null
cp "$W/k/settings.toml" "$W/kx/"
cat >>"$W/kx/settings.toml" <<TOML

[[input.keymap.rule]]
from = "capslok"
to = "esc"

[[input.keymap.rule]]
from = "f5"
to = "f6"
scope = { peer = "$LONG" }
TOML
shot 43-keys-keyboard 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys
shot 44-keys-edit-and-test 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEY=capslock ONE_MK_APP_KEY_MORE=1 ONE_MK_APP_KEY_TEST=58d0,58u85,58d300,46d350,46u400,58u450
shot 45-keys-press-a-key-small 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEY=f5 ONE_MK_APP_KEY_CAPTURE=1 ONE_MK_APP_SIZE=820x540
shot 46-keys-list-search 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEY=capslock ONE_MK_APP_KEY_PICK=vol
shot 47-keys-mouse 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEYS=mouse ONE_MK_APP_BUTTON=4
shot 48-keys-mouse-small 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEYS=mouse ONE_MK_APP_SIZE=820x540
shot 49-keys-problems-long-name 3 "$W/kx" ONE_MK_APP_STATUS="$F" ONE_MK_APP_TAB=keys ONE_MK_APP_KEY_SCOPE="$LONG"
shot 50-keys-focus 2 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_FOCUS=1
shot 51-keys-nothing-set 3 "$W/e" ONE_MK_APP_TAB=keys
# Saving from the window (as a click would): the rule must land in settings.toml.
mkdir -m 700 -p "$W/ks"
env ONE_MK_APP_TAB=keys ONE_MK_APP_KEY=f5 ONE_MK_APP_KEY_CHOOSE=ctrl+shift+esc ONE_MK_APP_SHOT="$W/ks.ppm" ONE_MK_APP_SHOT_AFTER=3 timeout 20 "$APP" --home "$W/ks" || true
grep -q '^to = "ctrl+shift+esc"' "$W/ks/settings.toml" || { echo "saving a key rule from the window failed" >&2; exit 1; }

# Overlays and the shared controls (handoff 5.8 and 6).
shot 90-palette 2 "$W/off" ONE_MK_APP_STATUS="$X" ONE_MK_APP_PALETTE=appear
shot 91-remove-sheet 2.5 "$W/off" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="Studio PC" ONE_MK_APP_REMOVE="Studio PC"
shot 92-delete-layout-sheet 2.5 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=arrange ONE_MK_APP_DELETE_PROFILE=Studio
shot 93-timer-menu 2 "$W/off" ONE_MK_APP_STATUS="$X" ONE_MK_APP_MENU=timer
shot 94-tooltip 2.5 "$W/off" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TIP=1
shot 95-update-ready 3 "$W/off" ONE_MK_APP_STATUS="$X" ONE_MK_APP_UPDATE=9.9.9
for c in general edges power games shortcuts commands typing pointer; do
    shot "96-settings-$c" 2.5 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=settings ONE_MK_APP_SETTINGS=$c
done
THEMES=light shot 99-widget-gallery 2 "$W/off" ONE_MK_APP_GALLERY=1 ONE_MK_APP_SIZE=1440x900
