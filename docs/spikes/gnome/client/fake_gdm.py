#!/usr/bin/env python3
"""Spike helper, PRIVATE system bus only: answer org.gnome.DisplayManager's
Version property so the isolated Shell enables its lock screen
(LoginManager.canLock). RegisterDisplay/RegisterSession are no-ops."""
from gi.repository import Gio, GLib
XML = """<node><interface name="org.gnome.DisplayManager.Manager">
<method name="RegisterDisplay"><arg type="a{ss}" direction="in"/></method>
<method name="RegisterSession"><arg type="a{sv}" direction="in"/></method>
<property name="Version" type="s" access="read"/></interface></node>"""
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM)

info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
def call(conn, sender, path, iface, method, params, inv): inv.return_value(None)
def prop(conn, sender, path, iface, name): return GLib.Variant("s", "50.0")
bus.register_object("/org/gnome/DisplayManager/Manager", info, call, prop, None)
Gio.bus_own_name_on_connection(bus, "org.gnome.DisplayManager", 0, None, None)
GLib.MainLoop().run()
