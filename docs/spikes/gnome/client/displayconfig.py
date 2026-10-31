#!/usr/bin/env python3
"""Spike helper (ISOLATED bus only): show or apply a monitor layout through
org.gnome.Mutter.DisplayConfig. Usage: displayconfig.py state | apply N [SCALE]
| layout JSON (apply N: enable the first N connectors side by side; layout:
a JSON list of [connector, x, y, scale, transform] logical monitors, the
first one primary). Always the temporary method."""
import sys
import math
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
def call(m, args=None):
    return bus.call_sync("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
                         "org.gnome.Mutter.DisplayConfig", m, args, None, 0, 5000).unpack()
serial, monitors, logical, props = call("GetCurrentState")
if sys.argv[1] == "state":
    for (conn, *_), modes, mprops in monitors:
        cur = [m[0] for m in modes if m[6].get("is-current")]
        print("monitor", conn, "current mode", cur)
    print("logical", [(l[0], l[1], [m[0] for m in l[5]]) for l in logical])
    sys.exit(0)
if sys.argv[1] == "layout":
    import json
    wanted = json.loads(sys.argv[2])
    modes = {conn: next(m for m in modes if m[6].get("is-current") or m[6].get("is-preferred"))
             for (conn, *_), modes, _ in monitors}
    lms = []
    for index, (conn, x, y, scale, transform) in enumerate(wanted):
        if conn not in modes or not math.isfinite(scale) or scale not in modes[conn][5] or transform not in range(8):
            raise ValueError(f'invalid logical monitor {conn} {scale} {transform}')
        lms.append((int(x), int(y), float(scale), int(transform), index == 0, [(conn, modes[conn][0], {})]))
    call("ApplyMonitorsConfig", GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})", (serial, 1, lms, {})))
    print("applied", json.dumps(wanted))
    sys.exit(0)
n = int(sys.argv[2])
scale = float(sys.argv[3]) if len(sys.argv) > 3 else 1.0
if not math.isfinite(scale) or scale <= 0 or n < 1 or n > len(monitors):
    raise ValueError('invalid monitor count or scale')
lms, x = [], 0
for (conn, *_), modes, mprops in monitors[:n]:
    mode = next(m for m in modes if m[6].get("is-current") or m[6].get("is-preferred"))
    if scale not in mode[5]:
        raise ValueError(f'scale {scale} not supported; available: {mode[5]}')
    lms.append((x, 0, scale, 0, x == 0, [(conn, mode[0], {})]))
    x += round(mode[1] / scale)
call("ApplyMonitorsConfig", GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})", (serial, 1, lms, {})))
print("applied", n, "monitor(s)")
