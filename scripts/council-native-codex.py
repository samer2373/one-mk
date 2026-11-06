import argparse
import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

from PIL import Image


class XImage(ctypes.Structure):
    _fields_ = [(name, ctypes.c_int) for name in ["width", "height", "xoffset", "format"]] + [("data", ctypes.c_void_p)] + [(name, ctypes.c_int) for name in ["byte_order", "bitmap_unit", "bitmap_bit_order", "bitmap_pad", "depth", "bytes_per_line", "bits_per_pixel"]] + [(name, ctypes.c_ulong) for name in ["red_mask", "green_mask", "blue_mask"]]


class ClientMessage(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int), ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong), ("format", ctypes.c_int), ("data", ctypes.c_long * 5)]


class XEvent(ctypes.Union):
    _fields_ = [("client", ClientMessage), ("padding", ctypes.c_long * 24)]


class Desktop:
    def __init__(self):
        self.x11 = ctypes.CDLL("libX11.so.6")
        self.xtest = ctypes.CDLL("libXtst.so.6")
        self.x11.XOpenDisplay.restype = ctypes.c_void_p
        self.display = self.x11.XOpenDisplay(None)
        if not self.display:
            raise RuntimeError("No X11 display")
        signatures = {
            "XDefaultRootWindow": ([ctypes.c_void_p], ctypes.c_ulong),
            "XInternAtom": ([ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int], ctypes.c_ulong),
            "XGetWindowProperty": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_long, ctypes.c_long, ctypes.c_int, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_void_p)], ctypes.c_int),
            "XFree": ([ctypes.c_void_p], ctypes.c_int),
            "XRaiseWindow": ([ctypes.c_void_p, ctypes.c_ulong], ctypes.c_int),
            "XSetInputFocus": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong], ctypes.c_int),
            "XIconifyWindow": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int], ctypes.c_int),
            "XFlush": ([ctypes.c_void_p], ctypes.c_int),
            "XStringToKeysym": ([ctypes.c_char_p], ctypes.c_ulong),
            "XKeysymToKeycode": ([ctypes.c_void_p, ctypes.c_ulong], ctypes.c_uint),
            "XGetGeometry": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)], ctypes.c_int),
            "XTranslateCoordinates": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong)], ctypes.c_int),
            "XGetImage": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int], ctypes.POINTER(XImage)),
            "XDestroyImage": ([ctypes.POINTER(XImage)], ctypes.c_int),
            "XSendEvent": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.POINTER(XEvent)], ctypes.c_int),
            "XQueryTree": ([ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)], ctypes.c_int),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.x11, name)
            function.argtypes = arguments
            function.restype = result
        self.xtest.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.root = self.x11.XDefaultRootWindow(self.display)

    def atom(self, name):
        return self.x11.XInternAtom(self.display, name.encode(), False)

    def property(self, window, name):
        actual, count, remaining = ctypes.c_ulong(), ctypes.c_ulong(), ctypes.c_ulong()
        format_bits = ctypes.c_int()
        data = ctypes.c_void_p()
        self.x11.XGetWindowProperty(self.display, window, self.atom(name), 0, 4096, False, 0, ctypes.byref(actual), ctypes.byref(format_bits), ctypes.byref(count), ctypes.byref(remaining), ctypes.byref(data))
        try:
            return list(ctypes.cast(data, ctypes.POINTER(ctypes.c_ulong))[:count.value]) if data.value and format_bits.value == 32 else []
        finally:
            if data.value:
                self.x11.XFree(data)

    def window(self, pid):
        for _ in range(80):
            windows = self.property(self.root, "_NET_CLIENT_LIST")
            if not windows:
                root, parent = ctypes.c_ulong(), ctypes.c_ulong()
                children, count = ctypes.POINTER(ctypes.c_ulong)(), ctypes.c_uint()
                self.x11.XQueryTree(self.display, self.root, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(children), ctypes.byref(count))
                try:
                    windows = list(children[:count.value]) if children else []
                finally:
                    if children:
                        self.x11.XFree(children)
            for window in windows:
                if self.property(window, "_NET_WM_PID") == [pid]:
                    return window
            time.sleep(0.1)
        raise RuntimeError(f"No owned window for PID {pid}")

    def focus(self, window):
        event = XEvent()
        event.client.type = 33
        event.client.display = self.display
        event.client.window = window
        event.client.message_type = self.atom("_NET_ACTIVE_WINDOW")
        event.client.format = 32
        event.client.data[0] = 2
        self.x11.XSendEvent(self.display, self.root, False, (1 << 20) | (1 << 19), ctypes.byref(event))
        self.x11.XFlush(self.display)
        time.sleep(0.3)
        self.x11.XRaiseWindow(self.display, window)
        self.x11.XSetInputFocus(self.display, window, 1, 0)
        self.x11.XFlush(self.display)

    def key(self, name):
        code = self.x11.XKeysymToKeycode(self.display, self.x11.XStringToKeysym(name.encode()))
        for pressed in [True, False]:
            self.xtest.XTestFakeKeyEvent(self.display, code, pressed, 0)
            self.x11.XFlush(self.display)
            time.sleep(0.05)
        self.x11.XFlush(self.display)

    def minimize(self, window):
        self.x11.XIconifyWindow(self.display, window, 0)
        self.x11.XFlush(self.display)
        time.sleep(1)
        return self.atom("_NET_WM_STATE_HIDDEN") in self.property(window, "_NET_WM_STATE")

    def capture(self, window, destination):
        root, child = ctypes.c_ulong(), ctypes.c_ulong()
        left, top = ctypes.c_int(), ctypes.c_int()
        width, height, border, depth = ctypes.c_uint(), ctypes.c_uint(), ctypes.c_uint(), ctypes.c_uint()
        self.x11.XGetGeometry(self.display, window, ctypes.byref(root), ctypes.byref(left), ctypes.byref(top), ctypes.byref(width), ctypes.byref(height), ctypes.byref(border), ctypes.byref(depth))
        self.x11.XTranslateCoordinates(self.display, window, self.root, 0, 0, ctypes.byref(left), ctypes.byref(top), ctypes.byref(child))
        pointer = self.x11.XGetImage(self.display, window, 0, 0, width.value, height.value, ctypes.c_ulong(-1).value, 2)
        if not pointer:
            raise RuntimeError("Owned-window XGetImage failed")
        try:
            image = pointer.contents
            if image.bits_per_pixel != 32 or image.red_mask != 0xFF0000:
                raise RuntimeError("Unsupported X11 pixel format")
            data = ctypes.string_at(image.data, image.bytes_per_line * image.height)
            Image.frombytes("RGB", (image.width, image.height), data, "raw", "BGRX", image.bytes_per_line).save(destination)
        finally:
            self.x11.XDestroyImage(pointer)
        return [width.value, height.value]


def cleanup(process, home):
    process.terminate()
    process.wait(timeout=15)
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            command = (entry / "cmdline").read_bytes().split(b"\0")
            if command and Path(os.fsdecode(command[0])).name == "one-mk-tray" and os.fsencode(home) in command:
                os.kill(int(entry.name), signal.SIGTERM)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass


def cpu_seconds(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["evidence", "scales", "keyboard", "runtime"])
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--tabs", default="desk,advanced")
    parser.add_argument("--steps", type=int, default=120)
    parser.add_argument("--runtime-tab", default="desk")
    parser.add_argument("--skip-frames", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    desktop = None if args.mode == "scales" else Desktop()
    with tempfile.TemporaryDirectory(prefix="codex-r2-evidence-") as directory:
        home = Path(directory)
        status = home / "status"
        status.write_text("ok\nname\tCodex reference\nid\tabcd-ef01\nport\t24800\npeer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\nlink\t1111-2222\t4200\t0\t900\n")
        environment = dict(os.environ, ONE_MK_APP_NO_SETUP="1", ONE_MK_APP_STATUS=str(status), ONE_MK_APP_SIZE="1200x780")
        environment.pop("WAYLAND_DISPLAY", None)
        environment["WINIT_UNIX_BACKEND"] = "x11"
        if args.mode in ["evidence", "scales", "keyboard"]:
            for tab in ([] if args.mode == "keyboard" else ["desk", "advanced", "keys"]):
                for scale in [1, 1.25, 1.5, 1.75, 2]:
                    shot = home / "shot.ppm"
                    shot.unlink(missing_ok=True)
                    size = "1200x780"
                    subprocess.run([str(args.binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_TAB=tab, ONE_MK_APP_ZOOM=str(scale), ONE_MK_APP_SIZE=size, ONE_MK_APP_SHOT=str(shot), ONE_MK_APP_SHOT_AFTER="3"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=25)
                    Image.open(shot).save(args.output / f"scale-{tab}-{scale}x.png")
                    print(f"scale {tab} {scale}", flush=True)
            for tab in ([] if args.mode == "keyboard" else ["desk", "advanced"]):
                shot = home / "reduced.ppm"
                shot.unlink(missing_ok=True)
                subprocess.run([str(args.binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_TAB=tab, ONE_MK_NO_GLASS="1", ONE_MK_REDUCE_MOTION="1", ONE_MK_APP_SHOT=str(shot), ONE_MK_APP_SHOT_AFTER="3"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=25)
                Image.open(shot).save(args.output / f"reduced-{tab}.png")
            if args.mode == "scales":
                return
            walkthrough = {}
            for tab in args.tabs.split(","):
                focus = home / "focus.txt"
                extra = {"ONE_MK_APP_KEY": "capslock", "ONE_MK_APP_KEY_MORE": "1"} if tab == "keys" else {}
                process = subprocess.Popen([str(args.binary.resolve()), "--home", str(home)], env=dict(environment, **extra, ONE_MK_APP_TAB=tab, ONE_MK_APP_FOCUS_LOG=str(focus)), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                try:
                    window = desktop.window(process.pid)
                    desktop.focus(window)
                    time.sleep(4)
                    rows = []
                    for step in range(1, args.steps + 1):
                        desktop.key("Tab")
                        time.sleep(0.3)
                        rows.append({"tab_press": step, "focus": focus.read_text() if focus.exists() else "No debug label"})
                        time.sleep(0.2)
                        desktop.capture(window, args.output / f"keyboard-{tab}-{step:02}.png")
                    walkthrough[tab] = rows
                finally:
                    cleanup(process, home)
            (args.output / "keyboard-order.json").write_text(json.dumps(walkthrough, indent=2) + "\n")
        else:
            environment["ONE_MK_APP_TAB"] = args.runtime_tab
            results = {"build": "release, opt-in design-evidence, LTO disabled", "page": args.runtime_tab, "idle_seconds": 60, "samples": [], "frames": []}
            binaries = [("baseline", args.baseline), ("candidate", args.binary)]
            for label, binary in binaries:
                if binary is None:
                    continue
                for scenario in ["visible", "minimized", "help"]:
                    extra = {"ONE_MK_APP_HELP": "history-enabled"} if scenario == "help" and args.runtime_tab == "files" else {"ONE_MK_APP_HELP": "lock-pointer"} if scenario == "help" else {}
                    with (args.output / f"{label}-{scenario}.log").open("w") as log:
                        process = subprocess.Popen([str(binary.resolve()), "--home", str(home)], env=dict(environment, **extra), stdout=log, stderr=log)
                        try:
                            window = desktop.window(process.pid)
                            desktop.focus(window)
                            time.sleep(12)
                            minimized = desktop.minimize(window) if scenario == "minimized" else False
                            if scenario != "minimized":
                                desktop.capture(window, args.output / f"runtime-{label}-{scenario}.png")
                            before = cpu_seconds(process.pid)
                            started = time.monotonic()
                            time.sleep(60)
                            elapsed = time.monotonic() - started
                            consumed = cpu_seconds(process.pid) - before
                            sample = {"build": label, "scenario": scenario, "minimized_verified": minimized, "elapsed_s": round(elapsed, 3), "cpu_s": round(consumed, 3), "one_core_percent": round(consumed / elapsed * 100, 3)}
                            results["samples"].append(sample)
                            print(json.dumps(sample), flush=True)
                        finally:
                            cleanup(process, home)
                    (args.output / "runtime.json").write_text(json.dumps(results, indent=2) + "\n")
                for repeat in range(0 if args.skip_frames else 3):
                    path = args.output / f"{label}-moving-{repeat}.log"
                    with path.open("w") as log:
                        subprocess.run([str(binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_BENCH="360"), stdout=log, stderr=log, timeout=90, check=True)
                    results["frames"].append({"build": label, "repeat": repeat, "log": path.name, "result": [line for line in path.read_text().splitlines() if line.startswith("bench ")]})
                (args.output / "runtime.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
