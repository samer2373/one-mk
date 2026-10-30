#!/usr/bin/env python3
"""Capture the supplied app under a caller-provided, isolated Xvfb display."""
import ctypes
import json
import os
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from PIL import Image

if os.environ.get("ONE_MK_ISOLATED_XVFB") != "1":
    sys.exit("Run only inside a private xvfb-run with ONE_MK_ISOLATED_XVFB=1")
root = Path(__file__).resolve().parent.parent
output = root / "target/update-validation"
output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="one-mk-update-window-") as directory, tempfile.TemporaryDirectory(prefix="isolated-app-", dir=output) as programs_directory:
    home = Path(directory)
    programs = Path(programs_directory)
    for name in ["one-mk-app", "one-mk"]:
        destination = programs / name
        destination.unlink(missing_ok=True)
        os.link(root / "target/debug" / name, destination)
    xlib = ctypes.CDLL("libX11.so.6")
    xtst = ctypes.CDLL("libXtst.so.6")
    xlib.XOpenDisplay.restype = ctypes.c_void_p
    xlib.XFlush.argtypes = [ctypes.c_void_p]
    xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
    xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
    xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
    xlib.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    xlib.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
    xlib.XDefaultRootWindow.restype = ctypes.c_ulong
    xlib.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)]
    xlib.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    xlib.XFree.argtypes = [ctypes.c_void_p]
    display = xlib.XOpenDisplay(None)
    assert display, "Run inside xvfb-run, never on the live desktop"
    assert os.environ.get("ONE_MK_ISOLATED_XVFB") == "1"
    for name, size, zoom, available, click in [
        ("offline", "980x660", "1", False, False),
        ("available", "980x660", "1", True, False),
        ("error-narrow", "760x540", "1", True, True),
        ("hidpi", "980x660", "2", True, False),
        ("keyboard", "760x540", "1", True, False),
        ("long-version", "760x540", "1", True, False),
        ("loading", "760x540", "1", True, True),
    ]:
        state = home / "update-state.json"
        if available:
            version = "0.1.13-" + "preview" * 16 if name == "long-version" else "0.1.13"
            state.write_text(json.dumps(dict(sequence=1, digest="00" * 32, highest_version=version, last_attempt=0, available=version)))
        else:
            state.unlink(missing_ok=True)
        shot = output / f"window-{name}.ppm"
        runtime = home / "runtime"
        runtime.mkdir(mode=0o700, exist_ok=True)
        environment = dict(PATH=os.environ["PATH"], HOME=str(home), XDG_RUNTIME_DIR=str(runtime), ONE_MK_HOME=str(home),
                           DISPLAY=os.environ["DISPLAY"], XAUTHORITY=os.environ["XAUTHORITY"], WINIT_UNIX_BACKEND="x11",
                           ONE_MK_APP_SIZE=size, ONE_MK_APP_ZOOM=zoom, ONE_MK_REDUCE_MOTION="1",
                           ONE_MK_APP_SHOT=str(shot), ONE_MK_APP_SHOT_AFTER="4")
        if name == "loading":
            daemon = programs / "one-mk"
            daemon.unlink()
            daemon.write_text("#!/bin/sh\nsleep 8\nprintf 'Fixture only: delayed update result\\n'\n")
            daemon.chmod(0o700)
        with (output / f"window-{name}.log").open("w") as log:
            process = subprocess.Popen([str(programs / "one-mk-app"), "--home", str(home)], env=environment, stdout=log, stderr=log, start_new_session=True)
            try:
                if name == "keyboard":
                    time.sleep(2)
                    parent = ctypes.c_ulong()
                    root_window = ctypes.c_ulong()
                    children = ctypes.POINTER(ctypes.c_ulong)()
                    count = ctypes.c_uint()
                    assert xlib.XQueryTree(display, xlib.XDefaultRootWindow(display), ctypes.byref(root_window), ctypes.byref(parent), ctypes.byref(children), ctypes.byref(count))
                    assert count.value > 0
                    xlib.XSetInputFocus(display, children[count.value - 1], 2, 0)
                    xlib.XFree(children)
                    xlib.XFlush(display)
                    time.sleep(0.15)
                    for keysym in [0xff09, 0xff0d]:
                        keycode = xlib.XKeysymToKeycode(display, keysym)
                        xtst.XTestFakeKeyEvent(display, keycode, 1, 0)
                        xtst.XTestFakeKeyEvent(display, keycode, 0, 0)
                        xlib.XFlush(display)
                        time.sleep(0.15)
                if click:
                    time.sleep(2)
                    xtst.XTestFakeMotionEvent(display, -1, 45, 52, 0)
                    xtst.XTestFakeButtonEvent(display, 1, 1, 0)
                    xtst.XTestFakeButtonEvent(display, 1, 0, 0)
                    xlib.XFlush(display)
                xtst.XTestFakeMotionEvent(display, -1, 1500, 1100, 0)
                xlib.XFlush(display)
                assert process.wait(timeout=20) == 0
                with Image.open(shot) as image:
                    image.save(output / f"window-{name}.png")
                shot.unlink()
                print(f"Captured {name}: {size}, zoom {zoom}", flush=True)
            finally:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                if process.poll() is None:
                    process.wait(timeout=5)
