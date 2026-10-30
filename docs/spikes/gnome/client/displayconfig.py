#!/usr/bin/env python3
"""Spike helper (ISOLATED bus only): show or apply a monitor layout through
org.gnome.Mutter.DisplayConfig. Usage: displayconfig.py state | apply N
(apply N: enable the first N connectors side by side, temporary method)."""
import sys
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
n = int(sys.argv[2])
lms, x = [], 0
for (conn, *_), modes, mprops in monitors[:n]:
    mode = next(m for m in modes if m[6].get("is-current") or m[6].get("is-preferred"))
    lms.append((x, 0, 1.0, 0, x == 0, [(conn, mode[0], {})]))
    x += mode[1]
call("ApplyMonitorsConfig", GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})", (serial, 1, lms, {})))
print("applied", n, "monitor(s)")
