#!/bin/bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/../../.." && pwd)
lab="$repo/target/wall-m1-lab"
mkdir -p "$lab/home"
case "$(env -i PATH=/usr/bin:/bin HOME="$lab/home" gnome-shell --version)" in
    'GNOME Shell 50.1') ;;
    *) echo 'Unqualified Shell version: this isolated test requires 50.1' >&2; exit 3 ;;
esac
cargo_bin=${CARGO_BIN:-$HOME/.cargo/bin}
env -i PATH="$cargo_bin:/usr/bin:/bin" HOME="$HOME" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
    CARGO_TARGET_DIR="$repo/target" cargo build --manifest-path "$repo/Cargo.toml" --offline \
    -p one-mk-wall --features gnome-risk-probe --profile bench-opt -j 2
for label in baseline candidate; do
    arguments=(--baseline)
    if [ "$label" = candidate ]; then
        arguments=(--baseline-log "$lab/baseline-shell.log")
    fi
    env -i PATH=/usr/bin:/bin HOME="$lab/home" USER="$(id -un)" PYTHONDONTWRITEBYTECODE=1 \
        python3 "$repo/docs/spikes/gnome/m1-risk-probe.py" --matrix --label "$label" \
        --binary "$repo/target/bench-opt/one-mk-wall" "${arguments[@]}" > "$lab/$label.stdout"
done
