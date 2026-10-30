#!/usr/bin/env python3
"""5F real-binary QA, exclusively on a private Xvfb and disposable homes.

Build: cargo build -j 2 -p one-mk -p one-mk-app
Run: python3 scripts/test-mouse-modes.py --output PATH
Requires Linux /proc, Xvfb, XTest, xwininfo and Pillow. No live-session access.
"""

import argparse
import ctypes
import ctypes.util
import importlib.util
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time
import tomllib

from PIL import ImageGrab


ROOT = Path(__file__).resolve().parents[1]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


idle = module("idle", ROOT / "scripts/test-idle-wakeups.py")
clipboard = module("clipboard_qa", ROOT / "scripts/test-clipboard-history.py")


def eventually(read, predicate, seconds=10):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        result = read()
        if predicate(result):
            return result
        time.sleep(.05)
    raise AssertionError(f"condition timed out; last value: {result!r}")


class X11(clipboard.IsolatedX11):
    def move(self, horizontal, vertical):
        xtest = ctypes.CDLL(ctypes.util.find_library("Xtst"))
        xtest.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
        xtest.XTestFakeMotionEvent(self.display, -1, horizontal, vertical, 0)
        self.library.XSync(self.display, 0)

    def key(self, symbol, down):
        xtest = ctypes.CDLL(ctypes.util.find_library("Xtst"))
        self.library.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        self.library.XKeysymToKeycode.restype = ctypes.c_uint
        xtest.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        xtest.XTestFakeKeyEvent(self.display, self.library.XKeysymToKeycode(self.display, symbol), down, 0)
        self.library.XSync(self.display, 0)

    def button(self, number):
        xtest = ctypes.CDLL(ctypes.util.find_library("Xtst"))
        xtest.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        for down in [1, 0]:
            xtest.XTestFakeButtonEvent(self.display, number, down, 0)
        self.library.XSync(self.display, 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--skip-idle", action="store_true", help="repeat UI checks without replacing idle.json")
    options = parser.parse_args()
    options.output.mkdir(parents=True, exist_ok=True)
    binary = str(ROOT / "target/debug/one-mk")
    app = str(ROOT / "target/debug/one-mk-app")
    with tempfile.TemporaryDirectory(prefix="mouse-qa-", dir=options.output) as directory:
        lab = idle.Lab(Path(directory), binary)
        try:
            home = lab.home("xvfb")
            xvfb = lab.spawn(["Xvfb", "-displayfd", "1", "-screen", "0", "1800x1400x24", "-nolisten", "tcp"], home,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            lab.cleanup.callback(xvfb.stdout.close)
            assert select.select([xvfb.stdout], [], [], 10)[0], "private Xvfb failed"
            display = xvfb.stdout.readline().decode().strip()
            assert display.isdigit()
            lab.display = ":" + display
            x11 = X11(lab.environment(home))
            lab.cleanup.callback(x11.close)

            def cli(home, *args, success=True):
                result = subprocess.run([binary, "--home", str(home), *args], env=lab.environment(home),
                                        capture_output=True, text=True, timeout=15)
                assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
                return result

            primary, peer = lab.home("sender"), lab.home("peer")
            primary_id, peer_id = lab.identity(primary), lab.identity(peer)
            listen, peer_listen = idle.free_port(), idle.free_port()
            for folder, fingerprint, port in [(primary, peer_id, peer_listen), (peer, primary_id, listen)]:
                (folder / "devices.toml").write_text(f'[[device]]\nname="qa-peer"\nfingerprint="{fingerprint}"\nside="right"\naddrs=["127.0.0.1:{port}"]\n')
            lab.daemon(peer, "null", peer_listen, False)
            sender = lab.daemon(primary, "x11", listen, False)
            eventually(lambda: lab.status(primary), lambda status: any(line.startswith("peer\t") and line.split("\t")[4] == "1" for line in status.splitlines()))
            for command in [("autoscroll", "--on", "--button", "button1"), ("autoscroll", "--on", "--button", "button2"),
                            ("gestures", "add", "button1", "left", "back"), ("gestures", "add", "button2", "left", "back")]:
                assert "primary left/right" in cli(primary, *command, success=False).stderr

            measurements = []

            def sample(label):
                stat = lambda: Path(f"/proc/{sender.pid}/stat").read_text().split(") ", 1)[1].split()
                before = stat()
                began = time.monotonic()
                time.sleep(options.seconds)
                after = stat()
                elapsed = time.monotonic() - began
                ticks = sum(int(after[index]) - int(before[index]) for index in [11, 12])
                result = dict(state=label, seconds=elapsed, ticks=ticks,
                              cpu_percent=100 * ticks / os.sysconf("SC_CLK_TCK") / elapsed,
                              rss_mib=int(after[21]) * os.sysconf("SC_PAGE_SIZE") / 1048576)
                assert sender.poll() is None
                measurements.append(result)
                print(json.dumps(result), flush=True)

            for enabled in ([] if options.skip_idle else [False, True]):
                cli(primary, "autoscroll", "--on" if enabled else "--off")
                cli(primary, "gestures", *(["preset", "side-swipe"] if enabled else ["clear"]))
                time.sleep(2)
                sample(f"{'ON' if enabled else 'OFF'} before broadcast")
                cli(primary, "broadcast", "on")
                x11.button(2)
                x11.button(8)
                time.sleep(2)
                sample(f"{'ON' if enabled else 'OFF'} broadcast after middle/side clicks")
                assert "broadcast\tqa-peer" in lab.status(primary), lab.status(primary)
                cli(primary, "broadcast", "off")
                time.sleep(2)
                sample(f"{'ON' if enabled else 'OFF'} after broadcast")
            if measurements:
                (options.output / "idle.json").write_text(json.dumps(measurements, indent=2) + "\n")
            lab.stop(sender)

            window_home = lab.home("window")
            daemon = lab.daemon(window_home, "null", idle.free_port(), False)
            states = [("off", "", {}), ("on", '[input.autoscroll]\nenabled=true\n', {}),
                      ("primary-rejected", '[input.autoscroll]\nenabled=true\nbutton="button1"\n[[input.gesture]]\nbutton="button2"\ndirection="Left"\nto="back"\n', {}),
                      ("primary-numeric", '[input.autoscroll]\nenabled=true\nbutton="1"\n[[input.gesture]]\nbutton="2"\ndirection="Left"\nto="back"\n', {}),
                      ("device-config", '[input.autoscroll]\nenabled=true\nscope={device="usb/1234:5678//test"}\n', {}),
                      ("device-selected", "", {"ONE_MK_APP_KEY_SCOPE": "usb/1234:5678//test"}),
                      ("narrow", '[input.autoscroll]\nenabled=true\n', {"ONE_MK_APP_SIZE": "820x900"}),
                      ("hidpi", '[input.autoscroll]\nenabled=true\n', {"ONE_MK_APP_ZOOM": "2", "ONE_MK_APP_SIZE": "1800x1400"}),
                      ("long-rules", "\n".join(f'[[input.gesture]]\nbutton="button4"\ndirection="Left"\nto="back"\napp="long-editor-{index}*"\n' for index in range(12)), {})]
            states.extend((name + "-scrolled", settings, extra) for name, settings, extra in list(states)
                          if name in ["narrow", "long-rules"])
            for name, settings, extra in states:
                (window_home / "settings.toml").write_text(settings)
                with socket.socket(socket.AF_UNIX) as stream:
                    stream.connect(str(window_home / "control.sock"))
                    stream.sendall(b"reload\n")
                    while stream.recv(65536):
                        pass
                environment = dict(lab.environment(window_home), ONE_MK_APP_TAB="keys", ONE_MK_APP_KEYS="mouse",
                                   ONE_MK_APP_SIZE="1300x1200", ONE_MK_REDUCE_MOTION="1")
                environment.update(extra)
                environment["ONE_MK_APP_SHOT"] = str((options.output / f"{name}.ppm").resolve())
                environment["ONE_MK_APP_SHOT_AFTER"] = "3"
                process = subprocess.Popen([app, "--home", str(window_home)], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                lab.cleanup.callback(lab.stop, process)
                if name.endswith("-scrolled"):
                    time.sleep(1.5)
                    x11.activate(environment)
                    x11.move(600, 750)
                    for _ in range(20):
                        x11.button(5)
                    time.sleep(.5)
                process.wait(timeout=20)
                from PIL import Image
                with Image.open(environment["ONE_MK_APP_SHOT"]) as image:
                    image.save(options.output / f"{name}.png")
                Path(environment["ONE_MK_APP_SHOT"]).unlink()
            (window_home / "settings.toml").write_text('[[input.gesture]]\nbutton="button4"\ndirection="Left"\nto="back"\n')
            environment = dict(lab.environment(window_home), ONE_MK_APP_TAB="keys", ONE_MK_APP_KEYS="mouse",
                               ONE_MK_APP_SIZE="1300x1200", ONE_MK_REDUCE_MOTION="1")
            process = subprocess.Popen([app, "--home", str(window_home)], env=environment,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            lab.cleanup.callback(lab.stop, process)
            time.sleep(3)
            x11.activate(environment)
            x11.click(160, 960)
            eventually(lambda: tomllib.loads((window_home / "settings.toml").read_text()).get("input", {}).get("gesture", []),
                       lambda rules: not rules)
            time.sleep(.5)
            ImageGrab.grab(xdisplay=lab.display).save(options.output / "capitalized-remove.png")
            print("REMOVE capitalized Left: settings.toml verified empty", flush=True)
            x11.click(500, 784)

            def enabled():
                return tomllib.loads((window_home / "settings.toml").read_text()).get("input", {}).get("autoscroll", {}).get("enabled", False)

            eventually(enabled, bool)
            for expected in [False, True]:
                time.sleep(1)
                x11.click(1120, 844)
                x11.key(0xffe1, 1)
                for _ in range(2):
                    x11.key(0xff09, 1)
                    x11.key(0xff09, 0)
                    time.sleep(.1)
                x11.key(0xffe1, 0)
                x11.key(0x20, 1)
                x11.key(0x20, 0)
                eventually(enabled, lambda value: value == expected)
                time.sleep(.5)
                ImageGrab.grab(xdisplay=lab.display).save(options.output / f"keyboard-{expected}.png")
            print("KEYBOARD speed field, Shift+Tab twice, Space: off then on; settings.toml verified", flush=True)
            lab.stop(daemon)
            time.sleep(1)
            ImageGrab.grab(xdisplay=lab.display).save(options.output / "offline.png")
            lab.stop(process)

            daemon = lab.daemon(window_home, "null", idle.free_port(), False)
            wrapper_home = lab.home("wrapper")
            os.link(app, wrapper_home / "one-mk-app")
            wrapper = wrapper_home / "one-mk"
            wrapper.write_text('#!/bin/sh\ncase "$*" in *autoscroll*) sleep 2; if [ "$QA_MOUSE_ERROR" = 1 ]; then echo "QA: simulated write failure" >&2; exit 1; fi;; esac\nexec ' + binary + ' "$@"\n')
            wrapper.chmod(0o700)
            for fail in [False, True]:
                environment["QA_MOUSE_ERROR"] = "1" if fail else "0"
                process = subprocess.Popen([str(wrapper_home / "one-mk-app"), "--home", str(window_home)],
                                           env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                lab.cleanup.callback(lab.stop, process)
                time.sleep(3)
                x11.activate(environment)
                x11.click(500, 784)
                time.sleep(.3)
                ImageGrab.grab(xdisplay=lab.display).save(options.output / f"saving-{fail}.png")
                time.sleep(3)
                ImageGrab.grab(xdisplay=lab.display).save(options.output / f"saved-error-{fail}.png")
                lab.stop(process)
            lab.stop(daemon)
        finally:
            lab.cleanup.close()


if __name__ == "__main__":
    main()
