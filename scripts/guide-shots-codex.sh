#!/usr/bin/env bash
# Screenshots of the macOS permission guide (crates/app/src/guide.rs), drawn
# on a pretend desktop by the debug-only ONE_MK_GUIDE_STAGE aid, so they can
# be made on any system. Nothing here touches System Settings or a daemon.
# Usage: scripts/guide-shots.sh [OUT_DIR]   (default /tmp/omk-guide-shots;
# the reviewed set is committed in docs/screens as guide-*.png)
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${1:-/tmp/omk-guide-shots}
case "$(realpath -m "$OUT")" in */docs/screens) SMALL=1 ;; *) SMALL= ;; esac
W=/tmp/omk-guide-shots-work-$$
export CARGO_TARGET_DIR="$PWD/target-codex"
cargo build -q -p one-mk-app -j 3
APP=${CARGO_TARGET_DIR:-$PWD/target}/debug/one-mk-app
rm -rf "$W" && mkdir -p "$OUT" && mkdir -m 700 -p "$W/home"
cleanup() { pkill -f "one-mk-tray --home $W/" || true; rm -rf "$W"; }
trap cleanup EXIT

# A 13-inch Mac: 1280x800 points, the menu bar's 25 points above the usable area.
MAC="0,25,1280,775"
shot() { # name after-seconds [env...]
    local name=$1 after=$2; shift 2
    for _ in 1 2 3; do
        rm -f "$W/$name.ppm"
        env ONE_MK_APP_NO_SETUP=1 ONE_MK_APP_SIZE=1280x800 ONE_MK_GUIDE_STAGE=1 ONE_MK_GUIDE_FAKE_SCREENS="$MAC" "$@" \
            ONE_MK_APP_SHOT="$W/$name.ppm" ONE_MK_APP_SHOT_AFTER="$after" timeout 30 "$APP" --home "$W/home" || true
        if python3 -c "from PIL import Image; Image.open('$W/$name.ppm').load()" 2>/dev/null; then break; fi
    done
    python3 -c "from PIL import Image, features; i = Image.open('$W/$name.ppm'); i = i.quantize(256, method=Image.Quantize.LIBIMAGEQUANT) if '$SMALL' and features.check_feature('libimagequant') else i; i.save('$OUT/$name.png', optimize=True)"
    echo "$OUT/$name.png"
}

G="ONE_MK_APP_GUIDE=both"
shot guide-01-beside-window 2.53 $G ONE_MK_GUIDE_FAKE_RECT=150,95,715,560
shot guide-02-arrow-right 2.2 $G ONE_MK_GUIDE_FAKE_RECT=520,95,715,560
shot guide-03-arrow-up 2.2 $G ONE_MK_GUIDE_FAKE_RECT=20,40,1240,430
shot guide-04-arrow-down 2.2 $G ONE_MK_GUIDE_FAKE_RECT=20,330,1240,440
shot guide-05-success-tick 2.3 $G ONE_MK_GUIDE_FAKE_RECT=150,95,715,560 ONE_MK_GUIDE_FAKE_GRANT=1.5
shot guide-06-step-two 3.9 $G ONE_MK_GUIDE_FAKE_RECT=150,95,715,560 ONE_MK_GUIDE_FAKE_GRANT=1.5
shot guide-07-settings-closed 3.6 $G ONE_MK_GUIDE_FAKE_RECT=150,95,715,560 ONE_MK_GUIDE_FAKE_CLOSE=1
shot guide-08-settings-hidden 9.2 $G ONE_MK_GUIDE_FAKE_RECT=hidden
shot guide-09-reduced-motion 2.53 $G ONE_MK_GUIDE_FAKE_RECT=150,95,715,560 ONE_MK_REDUCE_MOTION=1
shot guide-11-dark-settings 2.2 $G ONE_MK_GUIDE_FAKE_RECT=150,95,715,560 ONE_MK_GUIDE_STAGE=dark
shot guide-12-two-screens 2.2 $G ONE_MK_GUIDE_FAKE_RECT=0,25,1280,775 ONE_MK_GUIDE_FAKE_SCREENS="$MAC;1280,-100,1440,900"
shot guide-14-all-set 5.5 $G ONE_MK_GUIDE_FAKE_RECT=150,95,715,560 ONE_MK_GUIDE_FAKE_GRANT=1.5
shot guide-13-single-step 2.2 ONE_MK_APP_GUIDE=input-monitoring ONE_MK_GUIDE_FAKE_RECT=150,95,715,560
