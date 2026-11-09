#!/usr/bin/env python3
"""Throwaway stand-in for a one-mk renderer (spike only).

A plain GTK4 xdg_toplevel with app-id org.onemk.WallSpike that draws a moving
test pattern with its own frame counter. A new frame is drawn only when GTK's
frame clock ticks, and on Wayland the frame clock is paced by the
compositor's wl_surface.frame callbacks, so the logged rate is the rate Mutter
actually lets this client produce frames at.

Once a second it appends "t=<s> frames=<n> fps=<f> paused=<0|1> reason=<r>"
to --log. It listens (session bus) for the spike extension's PauseHint signal
and stops requesting frames while paused.
"""
import argparse
import math
import os
import sys
import time

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Graphene", "1.0")
from gi.repository import Gdk, Gio, GLib, Graphene, Gtk, Pango  # noqa: E402

BRIDGE_NAME = "org.onemk.WallpaperBridgeSpike1"
BRIDGE_PATH = "/org/onemk/WallpaperBridgeSpike1"


class Pattern(Gtk.Widget):
    """Draws with Gtk.Snapshot (GPU render nodes): no cairo dependency."""

    def __init__(self, renderer):
        super().__init__()
        self.r = renderer

    def do_snapshot(self, snap):
        r = self.r
        r.frames += 1
        r.window_frames += 1
        w, h = self.get_width(), self.get_height()
        t = time.monotonic() - r.start

        def rect(x, y, rw, rh, red, green, blue):
            snap.append_color(Gdk.RGBA(red=red, green=green, blue=blue, alpha=1.0),
                              Graphene.Rect().init(x, y, rw, rh))

        rect(0, 0, w, h, 0.08, 0.10, 0.16)
        # Eight moving bars plus a sweeping band of blocks: motion and
        # cropping continuity across monitors are easy to see.
        for i in range(8):
            rect((t * 240 + i * w / 8) % w, 0, 40, h, 0.2 + 0.1 * i, 0.5, 0.9 - 0.1 * i)
        for xx in range(0, w, 24):
            rect(xx, h / 2 + math.sin(xx / 120 + t * 3) * h / 5, 20, 20, 1.0, 0.55, 0.1)
        state = f"PAUSED ({r.reason})" if r.paused else "live"
        layout = self.create_pango_layout(f"one-mk spike frame {r.frames}  {state}")
        layout.set_font_description(Pango.FontDescription.from_string("monospace 48"))
        pt = Graphene.Point()
        pt.x, pt.y = 80, h - 160
        snap.save()
        snap.translate(pt)
        snap.append_layout(layout, Gdk.RGBA(red=1, green=1, blue=1, alpha=1))
        snap.restore()


class Renderer:
    def __init__(self, args):
        self.args = args
        self.frames = 0
        self.window_frames = 0
        self.window_start = time.monotonic()
        self.start = self.window_start
        self.paused = False
        self.reason = "-"
        self.log = open(args.log, "a", buffering=1) if args.log else sys.stdout
        self.app = Gtk.Application(application_id="org.onemk.WallSpike",
                                   flags=Gio.ApplicationFlags.NON_UNIQUE)
        self.app.connect("activate", self.on_activate)

    def make_window(self, width, height):
        """A fresh toplevel per canvas size: the bridge cannot resize a
        mapped DESKTOP-type window, so layout changes replace the window."""
        win = Gtk.ApplicationWindow(application=self.app, title=self.args.title)
        win.set_decorated(False)
        win.set_default_size(width, height)
        area = Pattern(self)
        area.set_hexpand(True)
        area.set_vexpand(True)
        win.set_child(area)
        area.add_tick_callback(self.tick)
        old = getattr(self, "win", None)
        self.win, self.area = win, area
        win.present()
        if old is not None:
            # Keep the old one until the new one is up, to avoid a flash.
            GLib.timeout_add(500, lambda: (old.destroy(), GLib.SOURCE_REMOVE)[1])
        self.log.write(f"window {width}x{height}\n")

    def on_activate(self, app):
        self.app.hold()
        self.make_window(self.args.width, self.args.height)
        GLib.timeout_add(1000, self.report)
        bus = Gio.bus_get_sync(Gio.BusType.SESSION)
        # Accept the hint only from the bridge's well-known name: GDBus
        # resolves it to the current unique owner, so another process
        # cannot spoof it without owning the name.
        bus.signal_subscribe(BRIDGE_NAME, BRIDGE_NAME, "PauseHint", BRIDGE_PATH,
                             None, Gio.DBusSignalFlags.NONE, self.on_hint)
        bus.call(BRIDGE_NAME, BRIDGE_PATH, BRIDGE_NAME, "Hello", GLib.Variant("(u)", (1,)),
                 None, Gio.DBusCallFlags.NONE, 2000, None, self.on_hello)
        bus.signal_subscribe(BRIDGE_NAME, BRIDGE_NAME, "Layout", BRIDGE_PATH,
                             None, Gio.DBusSignalFlags.NONE, self.on_layout)

    def on_hello(self, bus, res):
        try:
            self.log.write(f"hello reply {bus.call_finish(res).unpack()}\n")
        except GLib.Error as e:
            self.log.write(f"hello error {e.message}\n")

    def on_layout(self, _conn, _sender, _path, _iface, _signal, params):
        (mons,) = params.unpack()
        # Bounded: the bridge never sends more than a handful of monitors.
        mons = mons[:16]
        if not mons:
            return
        x0 = min(m[0] for m in mons); y0 = min(m[1] for m in mons)
        x1 = max(m[0] + m[2] for m in mons); y1 = max(m[1] + m[3] for m in mons)
        w, h = min(x1 - x0, 16384), min(y1 - y0, 16384)
        self.log.write(f"layout {mons} -> canvas {w}x{h}\n")
        self.make_window(w, h)

    def on_hint(self, _conn, _sender, _path, _iface, _signal, params):
        paused, reason = params.unpack()[:2]
        if self.args.ignore_hints:
            self.log.write(f"hint ignored paused={int(paused)} reason={reason}\n")
            return
        self.paused = bool(paused)
        self.reason = reason or "-"
        self.log.write(f"hint paused={int(self.paused)} reason={self.reason}\n")
        self.area.queue_draw()

    def tick(self, widget, _clock):
        if not self.paused:
            widget.queue_draw()
        return GLib.SOURCE_CONTINUE

    def report(self):
        now = time.monotonic()
        dt = now - self.window_start
        fps = self.window_frames / dt if dt > 0 else 0
        self.log.write(f"t={now - self.start:.1f} frames={self.frames} fps={fps:.1f} "
                       f"paused={int(self.paused)} reason={self.reason} pid={os.getpid()}\n")
        self.window_frames = 0
        self.window_start = now
        return GLib.SOURCE_CONTINUE


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--width", type=int, default=1920)
    p.add_argument("--height", type=int, default=1080)
    p.add_argument("--title", default="one-mk wallpaper spike renderer")
    p.add_argument("--log")
    p.add_argument("--ignore-hints", action="store_true",
                   help="keep drawing even when the bridge says paused (culling test)")
    args = p.parse_args()
    Renderer(args).app.run([sys.argv[0]])


if __name__ == "__main__":
    main()
