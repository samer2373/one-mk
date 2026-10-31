#!/usr/bin/env bash
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
scratch=$(mktemp -d /tmp/one-mk-x11-typing.XXXXXX)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
server_pid=
cleanup() {
    if [[ -n "$server_pid" ]]; then
        kill -TERM "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf -- "$scratch"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -m 700 "$scratch/home" "$scratch/runtime" "$scratch/state" "$scratch/tmp"
clean=(env -i "PATH=$PATH" "HOME=$scratch/home" "XDG_RUNTIME_DIR=$scratch/runtime" "ONE_MK_HOME=$scratch/state"
    "TMPDIR=$scratch/tmp" "CARGO_HOME=$cargo_home" "RUSTUP_HOME=$rustup_home" "CARGO_TARGET_DIR=$repo/target-x11-typing")
test_name=x11::tests::temporary_remap_restoration_races_a_delayed_client
while (( $# )); do
    case "$1" in
        --require-delivery) clean+=(ONE_MK_REQUIRE_X11_TEXT_DELIVERY=1); shift ;;
        --pool-retention) test_name=x11::tests::retained_pool_delivers_only_before_retirement; shift ;;
        --pool-reuse) test_name=x11::tests::retained_pool_reuse_corrupts_a_delayed_burst; shift ;;
        *) break ;;
    esac
done
command -v Xvfb >/dev/null
command -v xkbcomp >/dev/null
command -v setxkbmap >/dev/null
command -v xev >/dev/null
command -v stdbuf >/dev/null
coproc PRIVATE_XVFB { exec "${clean[@]}" Xvfb -displayfd 1 -screen 0 1280x800x24 -nolisten tcp -noreset 2>/dev/null; }
server_pid=$PRIVATE_XVFB_PID
read -r -t 15 display <&"${PRIVATE_XVFB[0]}"
[[ "$display" =~ ^[0-9]+$ ]] || { printf 'Xvfb returned an invalid display\n' >&2; exit 1; }
(( display >= 2 )) || { printf 'Refusing reserved desktop display :%s\n' "$display" >&2; exit 1; }
cd -- "$repo"
if (( $# )); then
    "${clean[@]}" "DISPLAY=:$display" "$@"
else
    "${clean[@]}" "DISPLAY=:$display" cargo test --offline -j 2 -p one-mk-input \
        "$test_name" -- --ignored --exact --nocapture
fi
