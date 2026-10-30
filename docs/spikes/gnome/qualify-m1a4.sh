#!/bin/bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/../../.." && pwd)
lab="$repo/target/wall-m1-lab"
mkdir -p "$lab/home"
cargo_bin=${CARGO_BIN:-$HOME/.cargo/bin}
env -i PATH="$cargo_bin:/usr/bin:/bin" HOME="$lab/home" \
    CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
    CARGO_TARGET_DIR="$repo/target" cargo build --manifest-path "$repo/Cargo.toml" --offline \
    -p one-mk-wall --features gnome-risk-probe --profile bench-opt -j 2
run() {
    local label=$1
    shift
    env -i PATH=/usr/bin:/bin HOME="$lab/home" USER="$(id -un)" PYTHONDONTWRITEBYTECODE=1 \
        python3 "$repo/docs/spikes/gnome/m1-risk-probe.py" --label "$label" \
        --baseline-log "$repo/docs/spikes/gnome/m1a2-evidence/baseline-shell.log" \
        --binary "$repo/target/bench-opt/one-mk-wall" "$@" > "$lab/$label.stdout"
}
for repeat in 1 2 3 4 5; do
    run "m1a4-stability-$repeat" --stress-only --cycles 100 --gc-stress --debug-shell
done
run m1a4-stability-500 --stress-only --cycles 500 --gc-stress --debug-shell
run m1a4-matrix --matrix --debug-shell
run m1a4-security --security-only
run m1a4-idle --matrix --idle-only
