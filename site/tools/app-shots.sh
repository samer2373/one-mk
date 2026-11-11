#!/usr/bin/env bash
# Screenshots of the oneMK window for the landing page, light and dark, at 2x.
# Everything is private: its own X server (Xvfb, never your session), a
# throwaway HOME/XDG_*, no daemon at all (a mocked status file stands in for
# three computers and five monitors), so nothing touches your real oneMK.
# Needs a build with the screenshot hooks (debug, or the design-evidence feature):
#   CARGO_TARGET_DIR=/somewhere cargo build --profile bench-opt -p one-mk-app -p one-mk \
#     --features one-mk-app/design-evidence
# Usage: site/tools/app-shots.sh BIN_DIR OUT_DIR [name ...]
set -u
BIN=$(realpath "$1"); OUT=$(realpath -m "$2"); shift 2
APP=$BIN/one-mk-app; CLI=$BIN/one-mk
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
W=$(mktemp -d /tmp/omk-site.XXXXXX)
mkdir -m700 -p "$W/h" "$W/x/c" "$W/x/d" "$W/x/k" "$W/x/r"
export HOME=$W/x XDG_CONFIG_HOME=$W/x/c XDG_DATA_HOME=$W/x/d XDG_CACHE_HOME=$W/x/k XDG_RUNTIME_DIR=$W/x/r
unset WAYLAND_DISPLAY; export DBUS_SESSION_BUS_ADDRESS=unix:path=$W/x/no-bus
exec 3>"$W/display"
Xvfb -displayfd 3 -screen 0 3000x1900x24 -nolisten tcp >/dev/null 2>&1 & XP=$!
trap 'kill $XP 2>/dev/null; rm -rf "$W"' EXIT
for _ in $(seq 50); do [ -s "$W/display" ] && break; sleep 0.1; done
export DISPLAY=:$(cat "$W/display") LIBGL_ALWAYS_SOFTWARE=1 ONE_MK_APP_NO_SETUP=1

# A changed key for the Keys page (written to the throwaway settings only).
"$CLI" --home "$W/h" keymap add capslock esc >/dev/null 2>&1 || true

X=$W/status
{
  printf 'ok\nname\tStudio PC\nid\t7a1c-0e42\nport\t24900\n'
  printf 'peer\t1111-2222\tleft\t3\t1\tauto\tMacBook Air\n'
  printf 'peer\t5555-6666\tright\t2\t1\tauto\tGaming PC\n'
  printf 'link\t1111-2222\t3100\t0\t900\nlink\t5555-6666\t2600\t0\t900\n'
  printf 'screen\tme\t0\t0\t2560\t1440\nscreen\tme\t2560\t180\t1920\t1080\n'
  printf 'screen\t1111-2222\t0\t0\t1470\t956\n'
  printf 'screen\t5555-6666\t0\t0\t1920\t1080\nscreen\t5555-6666\t1920\t-420\t1080\t1920\n'
  printf 'offset\t1111-2222\t-1470\t300\noffset\t5555-6666\t4480\t220\n'
  [ -n "${TRANSFER:-}" ] && printf 'transfer\t7\ts\t24\t612000000\t980000000\t52000000\tMacBook Air\n'
} >"$X"
RTT="1111-2222=3,4,3,3,5,4,3,3,4,3,3,4,6,4,3,3,4,3,3,3,4,3,3,4;5555-6666=3,2,3,3,2,3,3,2,2,3,4,3,2,3,3,2,3,3,2,3,3,2,3,3"

shot() { # name tab [VAR=VAL ...]
  local name=$1 tab=$2 theme; shift 2
  for theme in ${THEMES:-light dark}; do
    for _ in 1 2 3; do
      rm -f "$W/s.ppm"
      env ONE_MK_APP_THEME=$theme ONE_MK_APP_STATUS=$X ONE_MK_APP_TAB=$tab ONE_MK_APP_SSID="Home Wi-Fi" \
        ONE_MK_APP_RTT="$RTT" ONE_MK_APP_SHOT=$W/s.ppm ONE_MK_APP_SHOT_AFTER=${AFTER:-3.5} \
        ONE_MK_APP_SIZE=${SIZE:-1440x900} ONE_MK_APP_ZOOM=${ZOOM:-2} "$@" \
        timeout 60 "$APP" --home "$W/h" >"$W/app.log" 2>&1
      [ -s "$W/s.ppm" ] && break
    done
    python3 -I -c "from PIL import Image; Image.open('$W/s.ppm').convert('RGB').save('$OUT/$name-$theme.png')" && echo "$OUT/$name-$theme.png"
  done
}
for spec in "$@"; do eval "shot $spec"; done
