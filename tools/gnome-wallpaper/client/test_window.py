#!/usr/bin/env python3
"""Spike helper: an ordinary decorated GTK4 window ("normal app") used to prove
stacking. Usage: test_window.py TITLE [WIDTH HEIGHT] [--maximize|--fullscreen]"""
import sys
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gio, Gtk
title = sys.argv[1]
w, h = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (640, 400)
app = Gtk.Application(application_id="org.onemk.SpikeTestWindow", flags=Gio.ApplicationFlags.NON_UNIQUE)
def act(a):
    win = Gtk.ApplicationWindow(application=a, title=title)
    win.set_default_size(w, h)
    lbl = Gtk.Label(label=f"normal window: {title}")
    lbl.add_css_class("title-1")
    win.set_child(lbl)
    if "--maximize" in sys.argv: win.maximize()
    if "--fullscreen" in sys.argv: win.fullscreen()
    win.present()
app.connect("activate", act)
app.run([sys.argv[0]])
