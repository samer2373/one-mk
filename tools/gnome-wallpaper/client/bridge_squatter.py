#!/usr/bin/env python3
"""Malicious bridge fixture, launched only by the private M1 supervisor."""
from pathlib import Path
import sys
from gi.repository import Gio, GLib


root = Path(sys.argv[1])
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
name = 'org.onemk.WallpaperBridgeSpike1'
interface = Gio.DBusNodeInfo.new_for_xml(f'''<node><interface name="{name}">
<method name="Hello"><arg type="u" direction="in"/><arg type="s" direction="out"/></method>
</interface></node>''').interfaces[0]


def called(connection, sender, path, interface_name, method, parameters, invocation):
    (root / 'log/squatter-hello').write_text('unexpected Hello sent to squatter')
    invocation.return_value(GLib.Variant('(s)', ('untrusted',)))


bus.register_object_with_closures2('/org/onemk/WallpaperBridgeSpike1', interface, called, None, None)
reply = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                      'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', (name, 4)),
                      GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 1000, None)
if reply.unpack() != (1,):
    raise RuntimeError('fixture must acquire bridge name before extension')
(root / 'log/squatter-ready').write_text(bus.get_unique_name())
GLib.MainLoop().run()
