#!/usr/bin/env bash
# Screenshots of every state of the one·mk window, for design review.
# Runs local test daemons (no real input) in throwaway settings folders.
# Usage: scripts/ui-shots.sh [OUT_DIR]   (default /tmp/omk-shots; the
# reviewed set is committed in docs/screens)
# The debug-only ONE_MK_APP_* hooks it uses are listed in crates/app/src/app.rs.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${1:-/tmp/omk-shots}
# Shots for docs/screens are stored small (see shot()); elsewhere exact.
case "$(realpath -m "$OUT")" in */docs/screens) SMALL=1 ;; *) SMALL= ;; esac
W=/tmp/omk-shots-work-$$
cargo build -q -p one-mk -p one-mk-app -p one-mk-tray -j 2
B=$PWD/target/debug/one-mk
APP=$PWD/target/debug/one-mk-app
rm -rf "$W" && mkdir -p "$OUT" && mkdir -m 700 -p "$W/a" "$W/b" "$W/c" "$W/e" "$W/off" "$W/fx"
PIDS=()
# The window puts up the tray icon (one per settings folder); end those too.
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; pkill -f "one-mk-tray --home $W/" || true; rm -rf "$W"; }
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
    # For the committed set, 256 colours by libimagequant (dithered, so the
    # desk's gradients don't band): under half the size. Exact without it.
    python3 -c "from PIL import Image, features; i = Image.open('$W/$name.ppm'); i = i.quantize(256, method=Image.Quantize.LIBIMAGEQUANT) if '$SMALL' and features.check_feature('libimagequant') else i; i.save('$OUT/$name.png', optimize=True)"
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

"$B" --home "$W/a" run --backend null --no-discovery --port 24821 --connect 127.0.0.1:24822 >"$W/da.log" 2>&1 & PIDS+=($!)
"$B" --home "$W/b" run --backend null --no-discovery --port 24822 --connect 127.0.0.1:24821 >"$W/db.log" 2>&1 & B_PID=$!; PIDS+=($B_PID)
"$B" --home "$W/e" run --backend null --no-discovery --port 24823 >"$W/de.log" 2>&1 & PIDS+=($!)
sleep 4

shot 01-intro 0.4 "$W/a"
shot 03-desk-connected 4 "$W/a"
shot 04-files-tab 3 "$W/a" ONE_MK_APP_TAB=files
shot 06-small-window 4 "$W/a" ONE_MK_APP_SIZE=760x540
shot 08-nothing-paired 3 "$W/e"
shot 09-not-running 3 "$W/off"
# Pairing: a third computer waits with `one-mk pair` (it never answers).
(sleep 60 | timeout 40 "$B" --home "$W/c" pair --machine >/dev/null 2>&1 &)
shot 10-pairing-code 6 "$W/a" ONE_MK_APP_PAIR=1
shot 11-pairing-searching 1.2 "$W/e" ONE_MK_APP_PAIR=1
shot 13-this-computer-selected 3 "$W/a" ONE_MK_APP_SELECT=me ONE_MK_APP_AUTOSTART=1
# The newer pages against the real test daemons (a real `one-mk doctor`).
shot 24-layout-two-daemons 3 "$W/a" ONE_MK_APP_TAB=layout ONE_MK_APP_SELECT="Studio PC"
shot 25-health-two-daemons 8 "$W/a" ONE_MK_APP_TAB=health
shot 26-advanced-defaults 3 "$W/a" ONE_MK_APP_TAB=advanced
shot 14-mid-drag 2.4 "$W/a" ONE_MK_APP_DRAG=1
shot 31-layout-mid-drag 2.4 "$W/a" ONE_MK_APP_TAB=layout ONE_MK_APP_DRAG=1
shot 15-keyboard-focus 2 "$W/a" ONE_MK_APP_FOCUS=1
shot 16-error-toast 1.6 "$W/a" ONE_MK_APP_TOAST=1
kill "$B_PID"; sleep 3
shot 12-peer-offline 3 "$W/a"

# A third computer pairs and confirms; the window says the codes match.
(yes yes | timeout 40 "$B" --home "$W/c" pair --machine >/dev/null 2>&1 &)
shot 22-paired 9 "$W/a" ONE_MK_APP_PAIR=1 ONE_MK_APP_CONFIRM=1
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
shot 17-many-long-names 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_SELECT="$LONG"
shot 18-many-advanced 3 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_TAB=advanced
shot 19-drop-chooser 2.2 "$W/off" ONE_MK_APP_STATUS="$F" ONE_MK_APP_DROP=3
C=$W/controlled.status
printf 'ok\nname\tsamer-Victus\nid\tabcd-ef01\nport\t24800\ncontrolled_by\tStudio PC\n' >"$C"
printf 'peer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\n' >>"$C"
shot 21-controlled-small 3 "$W/off" ONE_MK_APP_STATUS="$C" ONE_MK_APP_SIZE=760x540

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
shot 29-layout-offset-multi-monitor 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=layout ONE_MK_APP_SELECT="MacBook Air"
shot 30-layout-edges-dont-meet 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=layout ONE_MK_APP_SELECT="Studio PC" ONE_MK_APP_PROFILE="home office"
shot 32-layout-keyboard-focus 2 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=layout ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_FOCUS=1
shot 33-files-transfers-clipboard 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=files
shot 34-health-failing 3 "$W/fx" ONE_MK_APP_STATUS="$XI" ONE_MK_APP_TAB=health ONE_MK_APP_DOCTOR="$D"
shot 35-advanced-settings 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=advanced
shot 36-small-layout 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=layout ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_SIZE=760x540
# Layout learning's question (docs/EDGE-PUSH.md), wide and small.
XS=$W/x-suggest.status
{ cat "$X"; printf 'suggest\t5555-6666\ttop\t"Gaming PC"\tGaming PC\n'; } >"$XS"
shot 52-layout-suggestion 3 "$W/fx" ONE_MK_APP_STATUS="$XS" ONE_MK_APP_TAB=layout ONE_MK_APP_SELECT="Gaming PC"
shot 53-layout-suggestion-small 3 "$W/fx" ONE_MK_APP_STATUS="$XS" ONE_MK_APP_TAB=layout ONE_MK_APP_SIZE=760x540 ONE_MK_REDUCE_MOTION=1

shot 39-small-advanced 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=advanced ONE_MK_APP_SIZE=760x540
# A high-density screen: everything drawn 1.5x on top of the desktop's scale.
shot 40-hidpi-desk 3 "$W/fx" ONE_MK_APP_STATUS="$X" ONE_MK_APP_SELECT="MacBook Air" ONE_MK_APP_ZOOM=1.5 ONE_MK_APP_SIZE=1140x810
shot 42-layout-nothing-paired 3 "$W/e" ONE_MK_APP_TAB=layout

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
shot 45-keys-press-a-key-small 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEY=f5 ONE_MK_APP_KEY_CAPTURE=1 ONE_MK_APP_SIZE=760x540
shot 46-keys-list-search 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEY=capslock ONE_MK_APP_KEY_PICK=vol
shot 47-keys-mouse 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEYS=mouse ONE_MK_APP_BUTTON=4
shot 48-keys-mouse-small 3 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_KEYS=mouse ONE_MK_APP_SIZE=760x540
shot 49-keys-problems-long-name 3 "$W/kx" ONE_MK_APP_STATUS="$F" ONE_MK_APP_TAB=keys ONE_MK_APP_KEY_SCOPE="$LONG"
shot 50-keys-focus 2 "$W/k" ONE_MK_APP_STATUS="$X" ONE_MK_APP_TAB=keys ONE_MK_APP_FOCUS=1
shot 51-keys-nothing-set 3 "$W/e" ONE_MK_APP_TAB=keys
# Saving from the window (as a click would): the rule must land in settings.toml.
mkdir -m 700 -p "$W/ks"
env ONE_MK_APP_TAB=keys ONE_MK_APP_KEY=f5 ONE_MK_APP_KEY_CHOOSE=ctrl+shift+esc ONE_MK_APP_SHOT="$W/ks.ppm" ONE_MK_APP_SHOT_AFTER=3 timeout 20 "$APP" --home "$W/ks" || true
grep -q '^to = "ctrl+shift+esc"' "$W/ks/settings.toml" || { echo "saving a key rule from the window failed" >&2; exit 1; }
