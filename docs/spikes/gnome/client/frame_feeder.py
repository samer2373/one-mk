#!/usr/bin/env python3
"""Spike step 2c ONLY (software fallback measurement): write ready-made
1920x1080 RGBA frames to $XDG_RUNTIME_DIR/one-mk-spike-frame.rgba and call
FrameReady at --fps. No decoder anywhere: frames are precomputed solid bands.
Must be spawned by the bridge (FrameReady checks the caller PID)."""
import argparse, os, time
from gi.repository import Gio, GLib
p = argparse.ArgumentParser(); p.add_argument("--fps", type=float, default=30); p.add_argument("--secs", type=float, default=15)
a = p.parse_args()
W, H = 1920, 1080
frames = []
for i in range(6):
    row = bytearray()
    for x in range(W):
        on = ((x // 160) + i) % 6 == 0
        row += bytes((255, 120, 20, 255) if on else (20, 30, 50, 255))
    frames.append(bytes(row) * H)
path = os.path.join(os.environ["XDG_RUNTIME_DIR"], "one-mk-spike-frame.rgba")
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
t0 = time.monotonic(); n = 0
while time.monotonic() - t0 < a.secs:
    with open(path + ".tmp", "wb") as f:
        f.write(frames[n % len(frames)])
    os.replace(path + ".tmp", path)
    bus.call_sync("org.onemk.WallpaperBridgeSpike1", "/org/onemk/WallpaperBridgeSpike1",
                  "org.onemk.WallpaperBridgeSpike1", "FrameReady", GLib.Variant("(u)", (n,)), None, 0, 2000, None)
    n += 1
    time.sleep(max(0, t0 + n / a.fps - time.monotonic()))
print("sent", n, "frames in", round(time.monotonic() - t0, 2), "s")
