#!/usr/bin/env bash
# Multi-computer reliability soak on one Linux machine: N oneMK daemons
# ("computers") paired in a full mesh over loopback, then M minutes of
# random churn (kill -9 and restart, stop and restart, SIGSTOP/SIGCONT as a
# sleep or a Wi-Fi stall, file sends, layout changes, status and doctor,
# and clipboard pushes and fetches when Xvfb and xclip are installed),
# then a quiet spell. RSS, open files, threads and CPU of every daemon are
# sampled every 10 s; n1 and n2 never restart, so their numbers show leaks.
#
# Nothing touches this machine's real oneMK or network: everything runs in
# a private network namespace (loopback only) as the same user, with
# throwaway HOME/XDG folders, the null input backend, no discovery, update
# checks off, ports PBASE+1..PBASE+N (never 24900), each computer named nI
# in its own UTS namespace, and private X servers for the clipboard.
#
# Usage: scripts/reliability-soak.sh [MINUTES] [NODES]   (default 30, 5)
#   ONE_MK=path/to/one-mk   binary to test (default: build --profile bench-opt)
#   PBASE=25100 IDLE_MIN=3 KEEP=1 (keep the work folder)
# Needs util-linux unshare with user namespaces, python3; optional Xvfb, xclip.
# Exit status 1 if an operation failed, a daemon panicked or logged an
# ERROR, or n1/n2 grew (RSS more than 25 % and 4 MB, fds or threads) after warm-up.
set -uo pipefail
if [ "$(ip -br link 2>/dev/null | wc -l)" != 1 ]; then
    self=$(realpath "$0")
    if [ -n "${ONE_MK:-}" ]; then
        ONE_MK=$(realpath "$ONE_MK")
    else
        cd "$(dirname "$self")/.." || exit 1
        cargo build -q --profile bench-opt -p one-mk --bin one-mk -j "${JOBS:-3}" || exit 1
        ONE_MK=$(realpath "${CARGO_TARGET_DIR:-$PWD/target}/bench-opt/one-mk")
    fi
    export ONE_MK
    exec unshare -Unm --map-user="$(id -u)" --map-group="$(id -g)" --keep-caps \
        bash -c 'ip link set lo up && exec "$@"' soak "$self" "$@"
fi
MIN=${1:-30} N=${2:-5} PBASE=${PBASE:-25100} IDLE_MIN=${IDLE_MIN:-3} B=$ONE_MK
[ "$PBASE" -le 24900 ] && [ $((PBASE + N)) -ge 24900 ] && { echo "PBASE range would include 24900"; exit 2; }
W=$(mktemp -d /tmp/omk-soak.XXXXXX)   # short: control sockets live here
export HOME=$W XDG_CONFIG_HOME=$W/c XDG_DATA_HOME=$W/d XDG_CACHE_HOME=$W/k XDG_RUNTIME_DIR=$W/r NO_COLOR=1
mkdir -m700 -p "$W/c" "$W/d" "$W/k" "$W/r" "$W/logs" "$W/src"
unset WAYLAND_DISPLAY DISPLAY
export DBUS_SESSION_BUS_ADDRESS=unix:path=$W/no-bus
CLIP=0; command -v Xvfb >/dev/null && command -v xclip >/dev/null && CLIP=1
declare -A PID XPID
h() { echo "$W/n$1"; }
as() { local i=$1; shift; unshare -u sh -c 'hostname "n$0" && exec "$@"' "$i" "$@"; }
mk() { as "$1" "$B" --home "$(h "$1")" "${@:2}"; }
xd() { echo $((150 + PBASE % 100 + $1)); }
log() { echo "[$(date +%T)] $*" | tee -a "$W/logs/soak.log"; }
start() {
    local i=$1 j a=()
    mkdir -m700 -p "$(h "$i")" "$W/dl$i"
    [ -e "$(h "$i")/settings.toml" ] || printf '[update]\ncheck = false\n' >"$(h "$i")/settings.toml"
    for j in $(seq 1 "$N"); do [ "$j" = "$i" ] || a+=(--connect "127.0.0.1:$((PBASE + j))"); done
    [ "$CLIP" = 1 ] || a+=(--no-clipboard)
    setsid env ${XPID[$i]:+DISPLAY=:$(xd "$i")} unshare -u sh -c 'hostname "n$0" && exec "$@"' "$i" \
        "$B" --home "$(h "$i")" run --backend null --no-discovery --port $((PBASE + i)) --downloads "$W/dl$i" \
        --no-notify "${a[@]}" >>"$W/logs/d$i.log" 2>&1 </dev/null &
    PID[$i]=$!
}
stop() { [ -n "${PID[$1]:-}" ] && kill "${PID[$1]}" 2>/dev/null; wait "${PID[$1]}" 2>/dev/null; PID[$1]=; }
kill9() { [ -n "${PID[$1]:-}" ] && kill -9 "${PID[$1]}" 2>/dev/null; wait "${PID[$1]}" 2>/dev/null; PID[$1]=; }
cleanup() {
    local i
    [ -n "${SAMPLER:-}" ] && kill "$SAMPLER" 2>/dev/null
    for i in "${!PID[@]}"; do [ -n "${PID[$i]}" ] && kill "${PID[$i]}" 2>/dev/null; done
    sleep 0.5
    for i in "${!PID[@]}"; do [ -n "${PID[$i]}" ] && kill -9 "${PID[$i]}" 2>/dev/null; done
    for i in "${!XPID[@]}"; do kill "${XPID[$i]}" 2>/dev/null; done
    pkill -P $$ xclip 2>/dev/null
    if [ "${KEEP:-0}" = 1 ]; then echo "kept $W"; else rm -rf "$W"; fi
}
trap cleanup EXIT
pair() { # i j, over loopback with the --machine protocol
    local i=$1 j=$2 pa=$W/logs/pair-$1-$2-a pb=$W/logs/pair-$1-$2-b pp
    ( timeout 40 unshare -u sh -c 'hostname "n$0" && exec "$@"' "$i" "$B" --home "$(h "$i")" pair --no-discovery --machine \
        < <(sleep 1; echo yes; sleep 30) >"$pa" 2>&1 & )
    for _ in $(seq 80); do grep -q '^me' "$pa" 2>/dev/null && break; sleep 0.1; done
    pp=$(awk -F'\t' '/^me/{print $4}' "$pa")
    ( timeout 40 unshare -u sh -c 'hostname "n$0" && exec "$@"' "$j" "$B" --home "$(h "$j")" pair --no-discovery \
        --connect "127.0.0.1:$pp" --machine < <(sleep 1; echo yes; sleep 30) >"$pb" 2>&1 & )
    for _ in $(seq 150); do grep -q '^paired' "$pa" && grep -q '^paired' "$pb" && return 0; sleep 0.1; done
    echo "pairing n$i and n$j failed"; cat "$pa" "$pb"; exit 1
}
state() { mk "$1" status 2>/dev/null | awk -v n="n$2" '/^  / && $1==n {print; exit}' | grep -oE 'online|offline' | head -1; }
mesh_up() {
    local i j
    for i in $(seq 1 "$N"); do [ -n "${PID[$i]:-}" ] || continue
        for j in $(seq 1 "$N"); do [ "$i" = "$j" ] || [ -z "${PID[$j]:-}" ] || [ "$(state "$i" "$j")" = online ] || return 1; done
    done
}
waitfor() { local end=$(( $(date +%s) + $1 )) s; s=$(date +%s); shift
    while [ "$(date +%s)" -le "$end" ]; do "$@" >/dev/null 2>&1 && { echo $(( $(date +%s) - s )); return 0; }; sleep 0.2; done
    echo TIMEOUT; return 1; }
xset_clip() { DISPLAY=:$(xd "$1") xclip -selection clipboard; }
clip_is() { [ "$(DISPLAY=:$(xd "$1") timeout 3 xclip -selection clipboard -o 2>/dev/null)" = "$2" ]; }

log "oneMK $("$B" --version) | $N computers | $MIN min churn + $IDLE_MIN min quiet | clipboard ops: $CLIP | work: $W"
if [ "$CLIP" = 1 ]; then for i in $(seq 1 "$N"); do
    Xvfb ":$(xd "$i")" -screen 0 1280x800x24 -nolisten tcp -nolisten local >/dev/null 2>&1 & XPID[$i]=$!; done; sleep 1; fi
for i in $(seq 1 "$N"); do start "$i"; done; sleep 1.5
for i in $(seq 1 "$N"); do for j in $(seq $((i + 1)) "$N"); do pair "$i" "$j"; done; done
log "paired; mesh up after $(waitfor 60 mesh_up) s"
for i in 1 2; do mk "$i" clipboard --allow-fetch on >/dev/null 2>&1; done
mkdir -p "$W/src/small"; for k in $(seq 200); do head -c $((RANDOM % 8000 + 1)) /dev/urandom >"$W/src/small/f$k"; done
head -c 40M /dev/urandom >"$W/src/m.bin"

HZ=$(getconf CLK_TCK)
echo "time,node,pid,rss_kb,fds,threads,cpu_ticks" >"$W/samples.csv"
sample() { local i p; for i in $(seq 1 "$N"); do p=${PID[$i]:-}; [ -n "$p" ] && [ -d "/proc/$p" ] || continue
    echo "$(date +%s),$i,$p,$(awk '/VmRSS/{print $2}' "/proc/$p/status"),$(ls "/proc/$p/fd" | wc -l),$(awk '/Threads/{print $2}' "/proc/$p/status"),$(awk '{print $14+$15}' "/proc/$p/stat")"
    done >>"$W/samples.csv"; }
sample; ( while sleep 10; do sample; done ) & SAMPLER=$!

declare -A OPS FAILS CHURNED
# A miss involving a computer that went away in the last 10 s is expected:
# its peers see the old link for up to 4 s (the idle timeout). Counted apart.
TRANSIENT=0
fail() {
    local x
    for x in "${@:3}"; do
        if [ $(( $(date +%s) - ${CHURNED[$x]:-0} )) -lt 10 ]; then TRANSIENT=$((TRANSIENT + 1)); echo "[$(date +%T)] (transient) $1: $2" >>"$W/logs/transient.log"; return; fi
    done
    FAILS[$1]=$(( ${FAILS[$1]:-0} + 1 )); echo "[$(date +%T)] $1: $2" >>"$W/logs/failures.log"
}
peer() { local b; while :; do b=$((RANDOM % N + 1)); [ "$b" != "$1" ] && [ -n "${PID[$b]:-}" ] && { echo "$b"; return; }; done; }
churn() { echo $((RANDOM % (N - 2) + 3)); }   # n3..nN come and go
end=$(( $(date +%s) + MIN * 60 ))
while [ "$(date +%s)" -lt "$end" ]; do
    r=$((RANDOM % 100)); a=$((RANDOM % 2 + 1))
    if [ $r -lt 10 ]; then i=$(churn); OPS[kill9]=$(( ${OPS[kill9]:-0} + 1 )); kill9 "$i"; sleep $((RANDOM % 4)); start "$i"; CHURNED[$i]=$(date +%s)
    elif [ $r -lt 18 ]; then i=$(churn); OPS[restart]=$(( ${OPS[restart]:-0} + 1 )); stop "$i"; sleep "0.$((RANDOM % 9))"; start "$i"; CHURNED[$i]=$(date +%s)
    elif [ $r -lt 24 ]; then i=$(churn); [ -n "${PID[$i]:-}" ] || continue; OPS[sigstop]=$(( ${OPS[sigstop]:-0} + 1 ))
        kill -STOP "${PID[$i]}"; sleep $((RANDOM % 12 + 1)); kill -CONT "${PID[$i]}"; CHURNED[$i]=$(date +%s)
    elif [ $r -lt 44 ]; then b=$(peer "$a"); OPS[send]=$(( ${OPS[send]:-0} + 1 ))
        f=$W/src/small; [ $((RANDOM % 2)) = 0 ] && f=$W/src/m.bin
        rm -rf "$W/dl$b/small" "$W/dl$b"/m*.bin
        out=$(timeout 120 "$B" --home "$(h "$a")" send --to "n$b" "$f" 2>&1) || fail send "n$a->n$b: $(echo "$out" | tail -1)" "$a" "$b"
    elif [ $r -lt 64 ] && [ "$CLIP" = 1 ]; then b=$(peer "$a"); OPS[clip]=$(( ${OPS[clip]:-0} + 1 )); v="soak $RANDOM $(date +%s%N)"
        echo -n "$v" | xset_clip "$a"; out=$(mk "$a" switch "n$b" 2>&1)
        waitfor 5 clip_is "$b" "$v" >/dev/null || fail clip "n$a->n$b ($(state "$a" "$b")): $out" "$a" "$b"; mk "$a" switch here >/dev/null 2>&1
    elif [ $r -lt 72 ] && [ "$CLIP" = 1 ]; then b=$(peer "$a"); OPS[clipget]=$(( ${OPS[clipget]:-0} + 1 )); v="get $RANDOM"
        echo -n "$v" | xset_clip "$a"; out=$(mk "$b" clip-get --from "n$a" 2>&1) && clip_is "$b" "$v" || fail clipget "n$b<-n$a: $out" "$a" "$b"
    elif [ $r -lt 84 ]; then b=$(peer "$a"); OPS[layout]=$(( ${OPS[layout]:-0} + 1 )); sides=(left right top bottom)
        mk "$a" layout "n$b" "${sides[$((RANDOM % 4))]}" >/dev/null 2>&1 || fail layout "n$a n$b" "$a" "$b"
    elif [ $r -lt 90 ]; then OPS[status]=$(( ${OPS[status]:-0} + 1 ))
        for i in $(seq 1 "$N"); do [ -n "${PID[$i]:-}" ] && { mk "$i" status >/dev/null 2>&1 || fail status "n$i"; mk "$i" doctor >/dev/null 2>&1; }; done
    else OPS[converge]=$(( ${OPS[converge]:-0} + 1 )); [ "$(waitfor 60 mesh_up)" = TIMEOUT ] && fail converge "mesh not up within 60 s"
    fi
done
for i in $(seq 1 "$N"); do [ -n "${PID[$i]:-}" ] || start "$i"; done
t=$(waitfor 90 mesh_up); [ "$t" = TIMEOUT ] && fail converge "final: mesh not up within 90 s"
log "churn done: $(for k in "${!OPS[@]}"; do echo -n "$k=${OPS[$k]} "; done); failures: $(for k in "${!FAILS[@]}"; do echo -n "$k=${FAILS[$k]} "; done)(+$TRANSIENT transient, within 10 s of a restart or stall); final mesh up after $t s"
quiet=$(date +%s); sleep $((IDLE_MIN * 60)); sample; kill "$SAMPLER"; SAMPLER=
python3 -I - "$W/samples.csv" "$quiet" "$HZ" <<'PY' | tee -a "$W/logs/soak.log"
import csv, sys
rows = list(csv.DictReader(open(sys.argv[1]))); quiet = int(sys.argv[2]); hz = int(sys.argv[3]); bad = 0
for node in ('1', '2'):
    r = [x for x in rows if x['node'] == node]
    r = [x for x in r if x['pid'] == r[0]['pid']]
    v = lambda x, k: int(x[k])
    warm = [x for x in r if v(x, 'time') >= v(r[0], 'time') + 120] or r
    idle = [x for x in r if v(x, 'time') >= quiet]
    cpu = (v(idle[-1], 'cpu_ticks') - v(idle[0], 'cpu_ticks')) / hz / max(1, v(idle[-1], 'time') - v(idle[0], 'time')) * 100 if len(idle) > 1 else -1
    rss0, rss1 = v(warm[0], 'rss_kb'), v(r[-1], 'rss_kb')
    grew = (rss1 > rss0 * 1.25 and rss1 - rss0 > 4096) or v(r[-1], 'fds') > v(warm[0], 'fds') + 4 or v(r[-1], 'threads') > v(warm[0], 'threads') + 2
    bad |= grew
    print(f"n{node}: RSS {v(r[0],'rss_kb')} kB at start, {rss0} after warm-up, {rss1} at end (max {max(v(x,'rss_kb') for x in r)}); "
          f"fds {v(warm[0],'fds')}->{v(r[-1],'fds')}; threads {v(warm[0],'threads')}->{v(r[-1],'threads')}; "
          f"CPU while quiet {cpu:.2f} %{'  GREW' if grew else ''}")
sys.exit(1 if bad else 0)
PY
leak=$?
errors=0
for i in $(seq 1 "$N"); do
    e=$(grep -c ERROR "$W/logs/d$i.log"); p=$(grep -c panicked "$W/logs/d$i.log"); errors=$((errors + e + p))
    log "n$i: $(wc -l <"$W/logs/d$i.log") log lines, $(grep -c WARN "$W/logs/d$i.log") WARN, $e ERROR, $p panics"
done
grep -h 'WARN\|ERROR\|panicked' "$W"/logs/d*.log | sed 's/peer=[^ ]*//; s/[0-9a-f]\{4\}-[0-9a-f]\{4\}/ID/g; s/[0-9][0-9.]*/N/g' \
    | sort | uniq -c | sort -rn | head -15
[ -s "$W/logs/failures.log" ] && { echo "failures:"; cut -c12- "$W/logs/failures.log" | sort | uniq -c | sort -rn | head -15; }
[ ! -s "$W/logs/failures.log" ] && [ "$leak" = 0 ] && [ "$errors" = 0 ] && { log PASS; exit 0; }
log FAIL; exit 1
