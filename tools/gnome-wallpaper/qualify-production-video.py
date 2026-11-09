#!/usr/bin/env python3
"""Run only via isolated-session.sh run, after consented install and enable."""
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import time

from gi.repository import Gio, GLib
from PIL import Image, ImageChops

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
ROOT = Path(os.environ["XDG_RUNTIME_DIR"]).parent
assert str(ROOT).startswith("/tmp/") and Path(os.environ["HOME"]) == ROOT / "home"
assert os.environ["WAYLAND_DISPLAY"] == "spike-wl-0"
OUT = Path("/tmp/gnome-video-evidence")
OUT.mkdir(exist_ok=True)
BIN = REPO / "target/bench-opt"
UUID = "one-mk-wallpaper@one-mk.org"
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
report = {"checks": [], "cpu": {}}
processes = []


def call(destination, path, interface, method, parameters=None):
    return bus.call_sync(destination, path, interface, method, parameters, None, 0, 10000).unpack()


def evaluate(source):
    ok, result = call("org.gnome.Shell", "/org/gnome/Shell", "org.gnome.Shell", "Eval", GLib.Variant("(s)", (source,)))
    assert ok, result
    return json.loads(result) if result else None


def extension(action):
    return call("org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.gnome.Shell.Extensions", action + "Extension", GLib.Variant("(s)", (UUID,)))


def screenshot(name):
    path = OUT / (name + ".png")
    ok, _ = call("org.gnome.Shell.Screenshot", "/org/gnome/Shell/Screenshot", "org.gnome.Shell.Screenshot", "Screenshot",
                 GLib.Variant("(bbs)", (False, False, str(path))))
    assert ok
    return path


def cli(*arguments, check=True):
    result = subprocess.run([str(BIN / "one-mk"), "--home", str(ROOT / "one-mk"), *arguments], capture_output=True, text=True, timeout=40)
    if check:
        assert result.returncode == 0, result.stderr
    return result.stdout


def spawn(arguments, name, stdin=None):
    log = (OUT / (name + ".log")).open("w")
    process = subprocess.Popen([str(arg) for arg in arguments], stdin=stdin, stdout=subprocess.PIPE, stderr=log, text=True)
    processes.append(process)
    return process


def page_command(command):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(40)
        connection.connect(str(ROOT / "one-mk/control.sock"))
        connection.sendall(("wallpaper-live " + json.dumps(command) + "\n").encode())
        response = connection.makefile("r").read()
        assert not response.startswith("error "), response


def await_state(process, state, timeout=12):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if select.select([process.stdout], [], [], 0.2)[0]:
            line = process.stdout.readline().strip()
            if line:
                with (OUT / "playback-events.jsonl").open("a") as output:
                    output.write(line + "\n")
                event = json.loads(line.removeprefix("one-mk-wallpaper "))
                if event.get("state") == state:
                    return
                assert event.get("state") != "error", event
        assert process.poll() is None, f"renderer exited: {process.returncode}"
    raise AssertionError(f"no {state} report in {timeout}s")


def command(process, text):
    process.stdin.write(text + "\n")
    process.stdin.flush()


def cpu(pids, seconds=3):
    def ticks(pid):
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return int(fields[11]) + int(fields[12])
    before = {name: ticks(pid) for name, pid in pids.items()}
    start = time.monotonic()
    time.sleep(seconds)
    elapsed = time.monotonic() - start
    return {name: round((ticks(pid) - before[name]) / os.sysconf("SC_CLK_TCK") / elapsed * 100, 2) for name, pid in pids.items()}


def changed(first, second):
    first, second = Image.open(first).convert("RGB"), Image.open(second).convert("RGB")
    return ImageChops.difference(first.crop((0, 40, *first.size)), second.crop((0, 40, *second.size))).getbbox() is not None


def desktop():
    return evaluate("global.get_window_actors().map(actor => { const window = actor.get_meta_window(); return {title: window.get_title(), pid: window.get_pid(), type: window.get_window_type(), hidden: window.is_hidden(), skip: window.is_skip_taskbar(), sticky: window.is_on_all_workspaces()}; })")


def layer_count():
    return evaluate("Main.layoutManager._backgroundGroup.get_children().map(actor => actor.get_children().length).reduce((total, count) => total + count, 0)")


try:
    report["shell"] = call("org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.freedesktop.DBus.Properties", "Get",
                           GLib.Variant("(ss)", ("org.gnome.Shell.Extensions", "ShellVersion")))[0]
    evaluate("Main.overview.hide(); true")
    time.sleep(2)
    baseline = screenshot("00-desktop-before")
    shell_pid = call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID", GLib.Variant("(s)", ("org.gnome.Shell",)))[0]
    daemon = spawn([BIN / "one-mk", "--home", ROOT / "one-mk", "--backend", "null", "--port", "35991", "--no-discovery", "--no-notify", "--no-clipboard", "run"], "daemon")
    for _ in range(60):
        if "Start oneMK first" not in cli("wallpaper", "status", check=False) and (ROOT / "one-mk/control.sock").exists():
            break
        time.sleep(0.1)
    report["product_set"] = cli("wallpaper", "set", str(OUT / "clip.webm"))
    time.sleep(3)
    report["product_status"] = cli("wallpaper", "status")
    assert "playing" in report["product_status"].lower(), report
    for fit in ["fit", "stretch", "fill"]:
        page_command({"action": "fit", "fit": fit})
        time.sleep(0.5)
        assert "playing" in cli("wallpaper", "status").lower()
    cli("wallpaper", "set", str(OUT / "clip.webm"))
    time.sleep(0.5)
    assert "playing" in cli("wallpaper", "status").lower()
    screenshot("01-product-video")
    cli("wallpaper", "pause")
    time.sleep(0.5)
    report["product_paused"] = cli("wallpaper", "status")
    assert "paused" in report["product_paused"].lower()
    cli("wallpaper", "play")
    time.sleep(0.5)
    cli("wallpaper", "off")
    time.sleep(0.5)
    assert "off" in cli("wallpaper", "status").lower()
    report["checks"].append("Wallpaper page command path: set/status/all Fit modes/replacement/pause/play/Off through production daemon and helper")
    state = call("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig", "org.gnome.Mutter.DisplayConfig", "GetCurrentState")
    monitors = []
    for x, y, scale, transform, primary, specs, _ in state[2]:
        monitors.append({"computer": "qa", "name": specs[0][0], "rect": {"x": x, "y": y, "width": 1920, "height": 1080},
                         "local": {"x": x, "y": y, "width": 1920, "height": 1080}, "scale": scale, "online": True})
    assert len(monitors) == 2, monitors
    config = {"computer": "qa", "path": str(OUT / "clip.webm"), "clock_offset_ms": 0, "plan": {"hash": "a" * 64,
              "title": "GNOME two-monitor qualification", "media": {"width": 1920, "height": 1080, "duration_ms": 10000, "frame_rate": 30},
              "canvas": {"monitors": monitors}, "start_ms": int(time.time() * 1000), "fit": "stretch", "computers": ["qa"],
              "pause_on_battery": False, "paused": False, "picture": False}}
    plan = OUT / "plan.json"

    def renderer(name):
        config["plan"]["start_ms"] = int(time.time() * 1000)
        plan.write_text(json.dumps(config))
        process = spawn([BIN / "one-mk-wallpaper", plan], name, subprocess.PIPE)
        await_state(process, "ready")
        return process

    player = renderer("two-monitors")
    report["windows"] = desktop()
    own = [window for window in report["windows"] if window["pid"] == player.pid]
    assert len(own) == 2 and all(window["skip"] and window["sticky"] and window["type"] == 1 for window in own), own
    assert evaluate(f"global.display.get_tab_list(imports.gi.Meta.TabList.NORMAL_ALL, null).every(window => window.get_pid() !== {player.pid})")
    report["cpu"]["playing"] = cpu({"renderer": player.pid, "shell": shell_pid})
    first = screenshot("02-two-monitors-frame-a")
    time.sleep(0.8)
    second = screenshot("03-two-monitors-frame-b")
    assert changed(first, second), "frames are not advancing"
    command(player, "pause")
    await_state(player, "paused")
    time.sleep(0.5)
    first = screenshot("04-paused-a")
    report["cpu"]["paused"] = cpu({"renderer": player.pid, "shell": shell_pid})
    second = screenshot("05-paused-b")
    assert not changed(first, second), "paused picture changed"
    command(player, "play")
    await_state(player, "ready")
    report["checks"].append("two 1080p monitors, common-frame crops, hidden desktop windows, frames advance; pause freezes; play resumes without restart")
    window = spawn(["python3", HERE / "client/test_window.py", "oneMK normal window above video", "800", "500"], "normal-window")
    time.sleep(2)
    screenshot("06-window-above-video")
    window.terminate()
    window.wait(timeout=5)
    command(player, "quit")
    player.wait(timeout=5)
    time.sleep(0.5)
    restored = screenshot("07-off-restored")
    assert not changed(baseline, restored), "Off did not restore the desktop"
    report["checks"].append("normal window covers wallpaper; Off removes clones and restores original desktop pixels")
    player = renderer("crash")
    player.kill()
    player.wait(timeout=5)
    time.sleep(0.5)
    assert not changed(baseline, screenshot("08-crash-restored"))
    assert call("org.onemk.WallpaperVideo1", "/org/onemk/WallpaperVideo1", "org.onemk.WallpaperVideo1", "Inspect") == ("50.1",)
    report["checks"].append("SIGKILL renderer: Shell and extension survive, original desktop pixels return")
    player = renderer("disable")
    extension("Disable")
    player.wait(timeout=5)
    assert player.returncode != 0
    time.sleep(0.5)
    assert not changed(baseline, screenshot("09-disable-restored"))
    extension("Enable")
    time.sleep(0.5)
    report["checks"].append("disable extension: renderer exits with error report and desktop restored; re-enable alone starts nothing")
    player = renderer("hotplug")
    subprocess.run(["python3", str(HERE / "client/displayconfig.py"), "apply", "1"], check=True, capture_output=True)
    time.sleep(2)
    assert player.poll() is None
    assert len([window for window in desktop() if window["pid"] == player.pid]) == 1
    screenshot("10-monitor-removed")
    subprocess.run(["python3", str(HERE / "client/displayconfig.py"), "apply", "2"], check=True, capture_output=True)
    time.sleep(2)
    assert player.poll() is None
    assert len([window for window in desktop() if window["pid"] == player.pid]) == 2
    screenshot("11-monitor-returned")
    command(player, "quit")
    player.wait(timeout=5)
    report["checks"].append("monitor removed and returned while playing; renderer and Shell survive")
    report["result"] = "passed"
except Exception as error:
    report["result"] = "failed"
    report["error"] = str(error)
    raise
finally:
    if "daemon" in globals() and daemon.poll() is None:
        cli("wallpaper", "off", check=False)
    for process in reversed(processes):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
    (OUT / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
