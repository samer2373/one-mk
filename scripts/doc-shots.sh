#!/usr/bin/env bash
# Regenerates the window screenshots used by README.md and docs/WINDOW.md
# (docs/screens/desk-{light,dark}.png, the README's pair, and
# {arrange,files,wallpaper,keys,health,settings}.png in the light theme).
# Everything is private: its own X server (Xvfb on a free display, never the
# real session), two paired `--backend null` daemons in throwaway homes with
# no discovery, and a throwaway HOME/XDG_*. When user namespaces are allowed
# the computer is called "Home-PC" instead of the real host name.
#
# Usage: scripts/doc-shots.sh [OUT_DIR]     (default docs/screens)
# Needs: Xvfb, python3 with Pillow (libimagequant makes the files small).
# Uses a debug build of one-mk and one-mk-app (the ONE_MK_APP_* hooks that
# take the pictures are listed in crates/app/src/app.rs).
set -u
cd "$(dirname "$0")/.."
OUT=$(realpath -m "${1:-docs/screens}")
cargo build -q -p one-mk -p one-mk-app -j 2 || exit 1
T=${CARGO_TARGET_DIR:-$PWD/target}/debug
if [ -z "${OMK_DOCSHOTS_INNER:-}" ] && unshare -Ur --uts true 2>/dev/null; then
    export OMK_DOCSHOTS_INNER=1
    exec unshare -Ur --uts bash -c 'hostname Home-PC; exec bash "$0" "$@"' "$0" "$@"
fi
# Unix control sockets need a short path, so the homes are under /tmp.
W=$(mktemp -d /tmp/omk-docshots.XXXXXX)
mkdir -m 700 -p "$W/a" "$W/b" "$W/c" "$W/d" "$W/k" "$W/r" "$OUT"
export HOME=$W XDG_CONFIG_HOME=$W/c XDG_DATA_HOME=$W/d XDG_CACHE_HOME=$W/k XDG_RUNTIME_DIR=$W/r
unset WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS
B=$T/one-mk
APP=$T/one-mk-app
PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done; sleep 0.5; rm -rf "$W"; }
trap cleanup EXIT

D=$(( 90 + RANDOM % 9 ))
Xvfb ":$D" -screen 0 1700x1100x24 -nolisten tcp +extension GLX >"$W/xvfb.log" 2>&1 &
PIDS+=($!)
sleep 2
export DISPLAY=":$D" LIBGL_ALWAYS_SOFTWARE=1 ONE_MK_APP_NO_SETUP=1

# Pair a and b over loopback, no discovery: a prints its pairing port.
( timeout 40 "$B" --home "$W/a" pair --no-discovery --machine < <(sleep 1; echo yes; sleep 20) >"$W/pa.log" 2>&1 & )
for _ in $(seq 40); do grep -q '^me' "$W/pa.log" 2>/dev/null && break; sleep 0.25; done
PORT=$(awk '/^me/{print $4}' "$W/pa.log")
( timeout 40 "$B" --home "$W/b" pair --no-discovery --connect "127.0.0.1:$PORT" --machine < <(sleep 1; echo yes; sleep 20) >"$W/pb.log" 2>&1 & )
for _ in $(seq 80); do grep -q '^name' "$W/a/devices.toml" 2>/dev/null && grep -q '^name' "$W/b/devices.toml" 2>/dev/null && break; sleep 0.25; done
sleep 1
sed -i '0,/^name = .*/s//name = "Studio PC"/' "$W/a/devices.toml"
"$B" --home "$W/a" layout "Studio PC" left >/dev/null
"$B" --home "$W/a" run --backend null --no-discovery --port 24821 --connect 127.0.0.1:24822 >"$W/da.log" 2>&1 & PIDS+=($!)
"$B" --home "$W/b" run --backend null --no-discovery --port 24822 --connect 127.0.0.1:24821 >"$W/db.log" 2>&1 & PIDS+=($!)
sleep 5

# Keys & mouse: rules written by the real commands, as a person would.
for args in "keymap add --tap=esc --hold=lctrl -- capslock" "keymap add -- ralt layer:nav" "keymap add --layer=nav -- h left" \
    "keymap add --os=mac -- lalt lmeta" "buttons preset side-back-forward" "scroll --natural=auto --modifier=shift=horizontal"; do
    # shellcheck disable=SC2086
    "$B" --home "$W/a" $args >/dev/null
done

shot() { # name after-seconds [env...]
    local name=$1 after=$2; shift 2
    for _ in 1 2 3; do
        rm -f "$W/$name.ppm"
        env ONE_MK_APP_THEME=light ONE_MK_APP_SSID="Home Wi-Fi" "$@" ONE_MK_APP_SIZE=1280x800 ONE_MK_APP_SHOT="$W/$name.ppm" ONE_MK_APP_SHOT_AFTER="$after" timeout 40 "$APP" --home "$W/a" >"$W/app-$name.log" 2>&1
        [ -s "$W/$name.ppm" ] && break
    done
    # 256 colours by libimagequant, dithered so the desk's gradients don't band.
    python3 -I -c "
from PIL import Image
i = Image.open('$W/$name.ppm').convert('RGB')
i.quantize(256, method=Image.Quantize.LIBIMAGEQUANT, dither=Image.Dither.FLOYDSTEINBERG).save('$OUT/$name.png', optimize=True)"
    echo "$OUT/$name.png"
}
UPDATED='{"check":true,"install_automatically":false,"available":null,"error":null,"can_update":false,"needs_password":false,"move_to_applications":false,"rollback_available":false,"advice":""}'
# The connection graph gains a reading a second: give it half a minute.
shot desk-light 30 ONE_MK_APP_SELECT="Studio PC"
shot desk-dark 30 ONE_MK_APP_SELECT="Studio PC" ONE_MK_APP_THEME=dark
shot arrange 4 ONE_MK_APP_TAB=arrange ONE_MK_APP_SELECT="Studio PC"
shot files 4 ONE_MK_APP_TAB=files
# Wallpaper playing needs a video player on each computer: a status file
# (debug builds) stands in for the daemon, with a drawn picture as the poster.
python3 -I -c "
from PIL import Image
w, h = 1920, 1080
top, mid, low = (44, 40, 92), (196, 112, 148), (236, 176, 128)
rows = []
for y in range(h):
    t = y / (h - 1)
    a, b, k = (top, mid, t / 0.55) if t < 0.55 else (mid, low, (t - 0.55) / 0.45)
    rows.append(tuple(int(a[i] + (b[i] - a[i]) * k) for i in range(3)))
im = Image.new('RGB', (1, h))
im.putdata(rows)
im.resize((w, h)).save('$W/poster.png')"
WALL='{"plan": {"hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "title": "Dunes at dusk.mp4", "media": {"width": 3840, "height": 2160, "duration_ms": 10000, "frame_rate": 30}, "canvas": {"monitors": [{"computer": "abcd-ef01", "name": "Home-PC · 1", "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440}, "local": {"x": 0, "y": 0, "width": 2560, "height": 1440}, "scale": 1.0, "online": true}, {"computer": "abcd-ef01", "name": "Home-PC · 2", "rect": {"x": 2560, "y": 180, "width": 1920, "height": 1080}, "local": {"x": 2560, "y": 180, "width": 1920, "height": 1080}, "scale": 1.0, "online": true}, {"computer": "1111-2222", "name": "Studio PC · 1", "rect": {"x": -1920, "y": 180, "width": 1920, "height": 1080}, "local": {"x": 0, "y": 0, "width": 1920, "height": 1080}, "scale": 1.0, "online": true}]}, "start_ms": 1, "fit": "fill", "computers": ["abcd-ef01", "1111-2222"], "pause_on_battery": true, "paused": false, "picture": false}, "message": "Playing on 3 monitors on 2 computers", "available": true, "poster": "'"$W"'/poster.png", "computers": {"abcd-ef01": {"state": "ready"}, "1111-2222": {"state": "ready"}}}'
printf 'ok\nname\tHome-PC\nid\tabcd-ef01\nport\t24800\npeer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\nlink\t1111-2222\t1200\t0\t900\nwallpaper\t%s\n' "$WALL" >"$W/wall.status"
shot wallpaper 4 ONE_MK_APP_TAB=wallpaper ONE_MK_APP_STATUS="$W/wall.status"
shot keys 4 ONE_MK_APP_TAB=keys
shot health 9 ONE_MK_APP_TAB=health
shot settings 4 ONE_MK_APP_TAB=settings ONE_MK_APP_UPDATE_STATUS="$UPDATED"
echo "Look at every picture before committing: a real Wi-Fi name or host name must not show."
