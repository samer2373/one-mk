#!/usr/bin/env python3
"""Spike helper, PRIVATE system bus only: a stand-in for logind's sleep
signal. Owns org.freedesktop.login1 on the isolated harness's own system bus
and exports org.freedesktop.login1.Manager with just the PrepareForSleep
signal (no properties, no methods), so the isolated Shell's LoginManager sees
a suspend/resume exactly as logind announces one. A separate test-only
interface, org.onemk.Test.Sleep.Simulate(b about_to_sleep), makes it emit
PrepareForSleep(about_to_sleep). Nothing is actually suspended.

Never run this against a real system bus: it refuses unless
DBUS_SYSTEM_BUS_ADDRESS names a socket inside the harness directory given
as the only argument."""
import os
import sys

from gi.repository import Gio, GLib

HARNESS = os.path.realpath(sys.argv[1])
ADDRESS = os.environ.get('DBUS_SYSTEM_BUS_ADDRESS', '')
if not ADDRESS.startswith('unix:path=') or \
        not os.path.realpath(ADDRESS[len('unix:path='):]).startswith(HARNESS + os.sep):
    sys.exit('fake_logind: refusing: not the private harness system bus')

XML = """<node>
<interface name="org.freedesktop.login1.Manager">
  <signal name="PrepareForSleep"><arg type="b" name="start"/></signal>
</interface>
<interface name="org.onemk.Test.Sleep">
  <method name="Simulate"><arg type="b" direction="in" name="about_to_sleep"/></method>
</interface>
</node>"""
PATH = '/org/freedesktop/login1'
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM)
node = Gio.DBusNodeInfo.new_for_xml(XML)


def call(connection, sender, path, interface, method, parameters, invocation):
    (about_to_sleep,) = parameters.unpack()
    connection.emit_signal(None, PATH, 'org.freedesktop.login1.Manager', 'PrepareForSleep',
                           GLib.Variant('(b)', (about_to_sleep,)))
    invocation.return_value(None)


bus.register_object(PATH, node.interfaces[0], None, None, None)
bus.register_object(PATH, node.interfaces[1], call, None, None)
Gio.bus_own_name_on_connection(bus, 'org.freedesktop.login1', Gio.BusNameOwnerFlags.DO_NOT_QUEUE,
                               lambda *_: print('fake_logind: ready', flush=True),
                               lambda *_: sys.exit('fake_logind: name unavailable'))
GLib.MainLoop().run()
