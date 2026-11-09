#!/usr/bin/env python3
"""Spike helper: org.gnome.Shell.Eval on the (isolated) session bus.
Usage: shell_eval.py 'js' | shell_eval.py -f file.js"""
import sys
from gi.repository import Gio, GLib
src = open(sys.argv[2]).read() if sys.argv[1] == "-f" else sys.argv[1]
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
ok, res = bus.call_sync("org.gnome.Shell", "/org/gnome/Shell", "org.gnome.Shell", "Eval",
                        GLib.Variant("(s)", (src,)), None, 0, 20000).unpack()
print(res if ok else "EVAL FAILED: " + res)
sys.exit(0 if ok else 1)
