#!/usr/bin/env bash
set -euo pipefail

if [[ ${1:-} != --owner-desktop ]]; then
    printf 'OWNER-ONLY manual desktop test; never run this in automated QA.\nUsage: %s --owner-desktop de|fr|pl|tr\n' "$0" >&2
    exit 2
fi
[[ ${XDG_SESSION_TYPE:-} == wayland ]] || { printf 'Run from the owner’s GNOME/KDE Wayland desktop.\n' >&2; exit 2; }
[[ -n ${DBUS_SESSION_BUS_ADDRESS:-} ]] || { printf 'No owner session bus address.\n' >&2; exit 2; }
case ${2:-} in
    de) text='z y Z Y @ € ß' ;;
    fr) text='a z q w é è ç à @ # { } [ ] | €' ;;
    pl) text='ą ć ę ł ń ó ś ź ż Ą Ć Ę Ł Ń Ó Ś Ź Ż' ;;
    tr) text='ı i İ I ş Ş ğ Ğ' ;;
    *) printf 'Choose de, fr, pl or tr.\n' >&2; exit 2 ;;
esac
[[ $# == 2 ]] || { printf 'Unexpected arguments.\n' >&2; exit 2; }
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
scratch=$(mktemp -d /tmp/one-mk-portal-manual.XXXXXX)
probe_pid=
cleanup() {
    if [[ -n "$probe_pid" ]]; then
        kill -TERM "$probe_pid" 2>/dev/null || true
        wait "$probe_pid" 2>/dev/null || true
    fi
    rm -rf -- "$scratch"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -m 700 "$scratch/home" "$scratch/runtime" "$scratch/state"
clean=(env -i "PATH=$PATH" "HOME=$scratch/home" "XDG_RUNTIME_DIR=$scratch/runtime" "ONE_MK_HOME=$scratch/state"
    "CARGO_HOME=$cargo_home" "RUSTUP_HOME=$rustup_home")
cd -- "$repo"
"${clean[@]}" cargo build --offline -j 2 -p one-mk-input --features portal-typing-probe --example portal-typing-probe
printf 'UNTESTED on real GNOME/KDE. Choose the %s layout manually; open a DISPOSABLE editor.\n' "$2"
printf 'Expected text (compare visually, including case and spaces):\n%s\n' "$text"
printf 'Approve the keyboard portal dialog, then focus that editor within five seconds.\n'
"${clean[@]}" "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS" \
    "$repo/target/debug/examples/portal-typing-probe" --allow-focused-editor-input "$text" &
probe_pid=$!
wait "$probe_pid"
probe_pid=
printf 'Inspect the editor: a successful D-Bus reply does NOT prove correct text delivery.\n'
