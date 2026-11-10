#!/usr/bin/env python3
"""Private negative fixture: retain the old toplevel during replacement."""
import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gio, GLib, Gtk


application = Gtk.Application(application_id='org.onemk.ReplacementProbe',
                              flags=Gio.ApplicationFlags.NON_UNIQUE)
windows = []


def window(title):
    surface = Gtk.ApplicationWindow(application=application, title=title)
    surface.set_default_size(640, 400)
    surface.set_child(Gtk.Label(label=title))
    surface.connect('close-request', lambda *_: True)
    windows.append(surface)
    surface.present()
    return GLib.SOURCE_REMOVE


def activate(app):
    window('M1 replacement old')
    GLib.timeout_add(500, lambda: window('M1 replacement new'))


application.connect('activate', activate)
application.run([])
