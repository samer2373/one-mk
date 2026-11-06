import argparse
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import gi
from PIL import Image, ImageChops

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

specification = importlib.util.spec_from_file_location("native", Path(__file__).with_name("council-native-codex.py"))
native = importlib.util.module_from_spec(specification)
specification.loader.exec_module(native)


def application(pid):
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        node = desktop.get_child_at_index(index)
        if node.get_process_id() == pid:
            return node
    raise RuntimeError(f"Owned PID {pid} missing from AT-SPI")


def nodes(node):
    yield node
    for index in range(node.get_child_count()):
        yield from nodes(node.get_child_at_index(index))


def find(pid, name):
    return next(node for node in nodes(application(pid)) if node.get_name() == name)


def dump(pid):
    rows = []
    for node in nodes(application(pid)):
        row = {"name": node.get_name(), "role": node.get_role_name(), "interfaces": list(node.get_interfaces()), "states": [state.value_nick for state in node.get_state_set().get_states()]}
        if "Text" in row["interfaces"]:
            row["text"] = Atspi.Text.get_text(node.get_text_iface(), 0, -1)
        rows.append(row)
    return rows


parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
subprocess.run(["gdbus", "call", "--session", "--dest", "org.a11y.Bus", "--object-path", "/org/a11y/bus", "--method", "org.freedesktop.DBus.Properties.Set", "org.a11y.Status", "IsEnabled", "<true>"], check=True, capture_output=True)
desktop = native.Desktop()
desktop.xtest.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
desktop.xtest.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]


def move(left, top):
    desktop.xtest.XTestFakeMotionEvent(desktop.display, 0, int(left), int(top), 0)
    desktop.x11.XFlush(desktop.display)
    time.sleep(0.35)


def press(down):
    desktop.xtest.XTestFakeButtonEvent(desktop.display, 1, down, 0)
    desktop.x11.XFlush(desktop.display)
    time.sleep(0.22)


report = {"method": "Real native X11 XTest motion/button hold, AT-SPI target bounds. No ONE_MK_APP_PRESS hook.", "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(), "families": {}, "screens": {}}
with tempfile.TemporaryDirectory(prefix="codex-r6-native-") as directory:
    home = Path(directory)
    status = home / "status"
    status.write_text("ok\nname\tCodex reference\nid\tabcd-ef01\nport\t24800\npeer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\npeer\t3333-4444\tright\t2\t0\tauto\tGaming PC\npeer\t5555-6666\tright\t1\t1\tauto\tSamer's extremely long workstation name that goes on and on\npeer\t7777-8888\tbottom\t1\t1\tauto\tМастерская\nlink\t1111-2222\t4200\t0\t900\n")
    doctor = home / "doctor.json"
    doctor.write_text(json.dumps({"ok": False, "checks": [{"id": "pairing", "title": "Paired computers", "status": "skip", "detail": "not paired", "fix": "Pair a computer"}, {"id": "input", "title": "Permissions", "status": "warn", "detail": "The desktop has not allowed input sharing.", "fix": "Allow oneMK in the desktop sharing dialog."}]}))
    environment = dict(os.environ, WINIT_UNIX_BACKEND="x11", ONE_MK_APP_NO_SETUP="1", ONE_MK_APP_STATUS=str(status), ONE_MK_APP_SIZE="1200x780", ONE_MK_APP_ZOOM="1")
    environment.pop("WAYLAND_DISPLAY", None)
    cases = [("switch", "files", "Notify when files arrive"), ("segment", "keys", "Mouse: buttons, pointer and scrolling"), ("chip", "keys", "Caps Lock as Esc/Ctrl"), ("capsule", "files", "Show desk"), ("icon", "files", "About Keep clipboard history"), ("sidebar", "desk", "Health")]
    for family, page, name in cases:
        process = subprocess.Popen([str(args.binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_TAB=page), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            window = desktop.window(process.pid)
            desktop.focus(window)
            time.sleep(3)
            move(2, 2)
            desktop.capture(window, args.output / f"state-{family}-rest.png")
            control = find(process.pid, name)
            bounds = control.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
            move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
            hover = args.output / f"state-{family}-hover.png"
            desktop.capture(window, hover)
            press(True)
            held = args.output / f"state-{family}-pressed.png"
            desktop.capture(window, held)
            press(False)
            difference = ImageChops.difference(Image.open(hover), Image.open(held)).getbbox()
            if not difference:
                raise AssertionError(f"No visible held press: {family}")
            report["families"][family] = {"target": name, "bounds": [bounds.x, bounds.y, bounds.width, bounds.height], "hover_vs_pressed_difference": difference}
            (args.output / "atspi-and-states.json").write_text(json.dumps(report, indent=2) + "\n")
        finally:
            press(False)
            native.cleanup(process, home)
    for page in ["desk", "files", "keys", "health", "advanced"]:
        extra = {"ONE_MK_APP_KEY": "capslock", "ONE_MK_APP_KEY_MORE": "1"} if page == "keys" else {}
        if page == "health":
            extra["ONE_MK_APP_DOCTOR"] = str(doctor)
        process = subprocess.Popen([str(args.binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_TAB=page, **extra), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            window = desktop.window(process.pid)
            desktop.focus(window)
            time.sleep(4)
            report["screens"][page] = dump(process.pid)
            if page == "keys":
                control = find(process.pid, "Test keys: activate, then press keys. Tab leaves. Nothing is sent.")
                assert control.get_role_name() in ["button", "push button"]
                assert control.get_action_iface().do_action(0)
                time.sleep(0.4)
                desktop.key("a")
                time.sleep(0.4)
                report["screens"]["keys-after-capture"] = dump(process.pid)
                assert any("You press: A" in row["name"] for row in report["screens"]["keys-after-capture"])
                desktop.capture(window, args.output / "keys-capture-output.png")
            if page == "health":
                names = " ".join(row["name"] for row in report["screens"][page])
                assert "No computers paired yet" not in names
                assert "the desktop allowed it" not in names
                assert "Your desktop has not allowed sharing yet" in names
            move(2, 2)
            desktop.capture(window, args.output / f"atspi-{page}.png")
            (args.output / "atspi-and-states.json").write_text(json.dumps(report, indent=2) + "\n")
        finally:
            native.cleanup(process, home)
    process = subprocess.Popen([str(args.binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_SIZE="760x540"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        window = desktop.window(process.pid)
        desktop.focus(window)
        time.sleep(3)
        assert find(process.pid, "Show full computer names and connection states").get_action_iface().do_action(0)
        move(2, 2)
        desktop.capture(window, args.output / "rail-full-names.png")
    finally:
        native.cleanup(process, home)
(args.output / "atspi-and-states.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({"real_pressed_families": list(report["families"]), "screens": {page: len(rows) for page, rows in report["screens"].items()}}))
