import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import gi
from PIL import Image, ImageDraw

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

specification = importlib.util.spec_from_file_location("native", Path(__file__).with_name("council-native-codex.py"))
native = importlib.util.module_from_spec(specification)
specification.loader.exec_module(native)


def find_control(node, wanted):
    if node.get_name() == wanted:
        return node
    for index in range(node.get_child_count()):
        found = find_control(node.get_child_at_index(index), wanted)
        if found:
            return found
    return None


def application_for(pid):
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        application = desktop.get_child_at_index(index)
        if application.get_process_id() == pid:
            return application
    raise RuntimeError("Owned application missing from AT-SPI")


def strip(paths, destination):
    result = Image.new("RGB", (4 * 480, 2 * 340), "#101216")
    drawing = ImageDraw.Draw(result)
    for index, path in enumerate(paths):
        image = Image.open(path)
        image.thumbnail((480, 312))
        left, top = index % 4 * 480, index // 4 * 340
        result.paste(image, (left, top + 24))
        drawing.text((left + 8, top + 6), path.stem, fill="white")
    result.save(destination)


parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
subprocess.run(["gdbus", "call", "--session", "--dest", "org.a11y.Bus", "--object-path", "/org/a11y/bus", "--method", "org.freedesktop.DBus.Properties.Set", "org.a11y.Status", "IsEnabled", "<true>"], check=True, capture_output=True)
desktop = native.Desktop()
report = {"method": "Owned native X11 window, AT-SPI click action, original frames retained", "actions": []}
with tempfile.TemporaryDirectory(prefix="codex-round5-") as directory:
    home = Path(directory)
    status = home / "status"
    base = "ok\nname\tCodex reference\nid\tabcd-ef01\nport\t24800\npeer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\n"
    status.write_text(base)
    environment = dict(os.environ, WINIT_UNIX_BACKEND="x11", ONE_MK_APP_NO_SETUP="1", ONE_MK_APP_STATUS=str(status), ONE_MK_APP_SIZE="1200x780", ONE_MK_APP_ZOOM="1")
    environment.pop("WAYLAND_DISPLAY", None)
    for page in ["keys", "files"]:
        process = subprocess.Popen([str(args.binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_TAB=page, **({"ONE_MK_APP_KEY": "capslock"} if page == "keys" else {})), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            window = desktop.window(process.pid)
            time.sleep(4)
            desktop.focus(window)
            frames = []
            if page == "keys":
                control = find_control(application_for(process.pid), "Key detail")
                if not control or not control.get_action_iface().do_action(0):
                    raise RuntimeError("Key detail AT-SPI action unavailable")
                report["actions"].append("Clicked Key detail through real AT-SPI")
                time.sleep(0.5)
                key = find_control(application_for(process.pid), "Caps Lock: unchanged. Arrows move between keys, Enter changes it")
                if not key or not key.get_component_iface().grab_focus():
                    raise RuntimeError("Keyboard cap AT-SPI focus unavailable")
                desktop.key("Right")
                desktop.key("Return")
                report["actions"].append("Focused Caps Lock through AT-SPI, then Right and Enter through XTest to select A")
            for index in range(8):
                if page == "files":
                    rate = 1_000_000 if index < 3 else 0
                    status.write_text(base + f"transfer\t8\ts\t1\t500000\t9000000\t{rate}\tStudio PC\n")
                    time.sleep(0.6)
                else:
                    time.sleep(0.06)
                path = args.output / f"{page}-frame-{index:02d}.png"
                desktop.capture(window, path)
                frames.append(path)
            strip(frames, args.output / f"{page}-strip.png")
        finally:
            native.cleanup(process, home)
(args.output / "method.json").write_text(json.dumps(report, indent=2) + "\n")
