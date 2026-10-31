#!/bin/bash
# Throwaway spike harness: run a FULLY ISOLATED headless GNOME Shell.
#
#   isolated-session.sh start [N_MONITORS]   start Shell with N virtual 1920x1080 monitors
#   isolated-session.sh run CMD...           run CMD inside the isolated session's env
#   isolated-session.sh restart [N]          kill only the Shell session (buses, Shell) and
#                                            start a new one in the same state dir
#   isolated-session.sh stop                 kill everything and delete the state dir
#
# Isolation: private session bus (dbus-run-session), a private *system* bus
# (so the Shell cannot attach to the owner's logind session, register a
# polkit agent or take sleep inhibitors), private HOME / XDG_* dirs under
# $SPIKE, GSETTINGS_BACKEND=memory, no Xwayland, empty environment (env -i),
# so WAYLAND_DISPLAY / DISPLAY / DBUS_* of the live desktop never leak in.
# Never point this at the live session.
set -euo pipefail

SPIKE=${SPIKE:-/tmp/omk-gs-spike}
HERE=$(cd "$(dirname "$0")" && pwd)

isolated_env() {
	local bus=""
	[ -f "$SPIKE/session-bus" ] && bus=$(cat "$SPIKE/session-bus")
	echo env -i \
		PATH=/usr/bin:/bin \
		HOME="$SPIKE/home" \
		ONE_MK_HOME="$SPIKE/one-mk" \
		USER="$USER" LOGNAME="$USER" \
		LANG=C.UTF-8 \
		XDG_RUNTIME_DIR="$SPIKE/run" \
		XDG_DATA_HOME="$SPIKE/data" \
		XDG_CONFIG_HOME="$SPIKE/config" \
		XDG_CACHE_HOME="$SPIKE/cache" \
		XDG_STATE_HOME="$SPIKE/state" \
		XDG_SESSION_TYPE=wayland \
		XDG_CURRENT_DESKTOP=GNOME \
		GSETTINGS_BACKEND=memory \
		DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SPIKE/run/system_bus" \
		${bus:+DBUS_SESSION_BUS_ADDRESS="$bus"} \
		WAYLAND_DISPLAY=spike-wl-0 \
		GDK_BACKEND=wayland \
		GDK_DEBUG=no-portals \
		GTK_A11Y=none NO_AT_BRIDGE=1 \
		GIO_USE_VFS=local GVFS_DISABLE_FUSE=1
}

kill_session() {
	if [ -f "$SPIKE/leader.pid" ]; then
		sid=$(cat "$SPIKE/leader.pid")
		pkill -TERM -s "$sid" 2>/dev/null || true
		sleep 2
		pkill -KILL -s "$sid" 2>/dev/null || true
	fi
}

launch() {
	n=$1
	cat > "$SPIKE/system-bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path=$SPIKE/run/system_bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>
EOF
	mons=""
	for _ in $(seq "$n"); do mons="$mons --virtual-monitor 1920x1080"; done
	# The inner script runs inside dbus-run-session: record the private bus
	# address, start a private system bus, then the headless Shell.
	cat > "$SPIKE/inner.sh" <<EOF
#!/bin/bash
echo "\$DBUS_SESSION_BUS_ADDRESS" > "$SPIKE/session-bus"
dbus-daemon --config-file="$SPIKE/system-bus.conf" --nofork --nopidfile &
echo \$! > "$SPIKE/system-bus.pid"
for i in \$(seq 50); do [ -S "$SPIKE/run/system_bus" ] && break; sleep 0.1; done
# FAKE_GDM=1: answer GDM's Version on the PRIVATE system bus so the Shell
# enables its lock screen (needed only for the lock-screen test).
[ "${FAKE_GDM:-0}" = 1 ] && { python3 "$HERE/client/fake_gdm.py" & sleep 0.5; }
ulimit -c 0
if [ "${DEBUG_SHELL:-0}" = 1 ]; then
    gdb --batch -ex 'set pagination off' -ex 'set confirm off' \
        -ex 'set debuginfod enabled off' -ex 'handle SIGPIPE nostop noprint pass' \
        -ex run -ex 'thread apply all bt full' \
        -ex 'generate-core-file $SPIKE/log/shell.core' \
        --args gnome-shell --wayland --headless --no-x11 --wayland-display spike-wl-0 $mons
else
    gnome-shell --wayland --headless --no-x11 --wayland-display spike-wl-0 $mons
fi
echo \$? > "$SPIKE/shell.exit"
EOF
	chmod +x "$SPIKE/inner.sh"
	# setsid: own process group/session so stop can kill all of it.
	setsid $(isolated_env) dbus-run-session -- "$SPIKE/inner.sh" \
		> "$SPIKE/log/shell.log" 2>&1 < /dev/null &
	echo $! > "$SPIKE/leader.pid"
	for i in $(seq 100); do [ -S "$SPIKE/run/spike-wl-0" ] && break; sleep 0.2; done
	[ -S "$SPIKE/run/spike-wl-0" ] || { echo "Shell did not come up; see $SPIKE/log/shell.log" >&2; exit 1; }
	sleep 2
	echo "isolated Shell up: leader $(cat "$SPIKE/leader.pid") bus $(cat "$SPIKE/session-bus")"
}

case "${1:-}" in
start)
	[ -e "$SPIKE" ] && { echo "$SPIKE exists; run stop first" >&2; exit 1; }
	mkdir -p "$SPIKE"/{home/Desktop,run,data/gnome-shell/extensions,config,cache,state,log}
	chmod 700 "$SPIKE/run"
	echo "spike test file" > "$SPIKE/home/Desktop/spike-icon.txt"
	for ext in "${EXT_SRC:-$HERE/ext}"/*/; do cp -r "$ext" "$SPIKE/data/gnome-shell/extensions/"; done
	launch "${2:-1}"
	;;
restart)
	# Only the session's own process group (Shell, private buses) is killed;
	# processes started outside it are the caller's to check and stop.
	[ -d "$SPIKE" ] || { echo "$SPIKE does not exist" >&2; exit 1; }
	kill_session
	rm -f "$SPIKE/run/spike-wl-0" "$SPIKE/run/spike-wl-0.lock" "$SPIKE/run/system_bus" \
		"$SPIKE/session-bus" "$SPIKE/shell.exit" "$SPIKE/system-bus.pid"
	launch "${2:-1}"
	;;
run)
	shift
	exec $(isolated_env) "$@"
	;;
stop)
	[ -d "$SPIKE" ] || exit 0
	kill_session
	# Strays that escaped the session (D-Bus-activated services keep our env).
	for p in $(pgrep -u "$USER" . ); do
		if { tr "\0" "\n" < /proc/$p/environ; } 2>/dev/null | grep -qx "XDG_RUNTIME_DIR=$SPIKE/run"; then
			echo "killing stray $p $(cat /proc/$p/comm)"; kill -KILL "$p" 2>/dev/null || true
		fi
	done
	rm -rf "$SPIKE"
	echo stopped
	;;
*)
	echo "usage: $0 start [N]|run CMD...|restart [N]|stop" >&2; exit 2;;
esac
