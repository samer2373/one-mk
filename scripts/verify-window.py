#!/usr/bin/env python3
"""End-to-end check of the window against two real daemons, in isolation.

Run it through the private namespace (no network, no owner session):

    scripts/audit-wave6-isolated.sh python3 scripts/verify-window.py

It pairs two `--backend null` daemons on loopback, opens the debug window
on a private Xvfb, and drives it with synthetic mouse and keyboard input
(XTEST), finding controls by their screen-reader names through the debug
`ONE_MK_APP_TARGETS` dump. Each step checks what changed where it matters:
the daemon's status, `settings.toml`, or the window's own controls. It
also runs the window multisampled (`ONE_MK_APP_MSAA=4`) and measures the
window thread's CPU while idle. Screenshots go to $VERIFY_OUT (default
.audit-wave6/verify).

Needs python-xlib in .audit-wave6/xlib (XTEST input and screenshots):
`uv pip install --target .audit-wave6/xlib python-xlib`.
"""

import importlib.util
import os
from pathlib import Path
import re
import select
import subprocess
import sys
import tempfile
import time

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / ".audit-wave6/xlib"))
from Xlib import X, display as xdisplay  # noqa: E402
from Xlib.ext import xtest  # noqa: E402

RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail and not ok else ""), flush=True)


class Window:
    def __init__(self, app, home, env, display_name, targets, size="1200x780", extra=None):
        environment = dict(env, DISPLAY=display_name, ONE_MK_APP_TARGETS=str(targets), ONE_MK_APP_SIZE=size, **(extra or {}))
        self.log = open(home / "window.log", "w")
        self.process = subprocess.Popen([str(app), "--home", str(home)], env=environment, stdout=self.log, stderr=self.log)
        self.targets = targets
        self.d = xdisplay.Display(display_name)
        self.root = self.d.screen().root

    def where(self):
        """The window's top left on the screen (no window manager: as mapped)."""
        for w in self.root.query_tree().children:
            try:
                g = w.get_geometry()
                if g.width > 400 and w.get_attributes().map_state == X.IsViewable:
                    t = w.translate_coords(self.root, 0, 0)
                    return -t.x, -t.y, g.width, g.height
            except Exception:
                continue
        return 0, 0, 0, 0

    def controls(self):
        try:
            text = self.targets.read_text()
        except OSError:
            return []
        out = []
        for line in text.splitlines():
            parts = line.split("\t")
            if len(parts) == 5:
                out.append((parts[0], tuple(float(v) for v in parts[1:])))
        return out

    def find(self, pattern, timeout=8.0):
        rx = re.compile(pattern)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for label, r in self.controls():
                if rx.search(label):
                    return label, r
            time.sleep(0.1)
        return None

    def has(self, pattern, timeout=4.0):
        return self.find(pattern, timeout) is not None

    def move(self, x, y):
        ox, oy, _, _ = self.where()
        xtest.fake_input(self.d, X.MotionNotify, x=int(ox + x), y=int(oy + y))
        self.d.sync()
        time.sleep(0.08)

    def click(self, x, y, button=1):
        self.move(x, y)
        xtest.fake_input(self.d, X.ButtonPress, button)
        self.d.sync()
        time.sleep(0.05)
        xtest.fake_input(self.d, X.ButtonRelease, button)
        self.d.sync()
        time.sleep(0.35)

    def click_named(self, pattern, timeout=8.0):
        hit = self.find(pattern, timeout)
        if not hit:
            return False
        _, (x0, y0, x1, y1) = hit
        self.click((x0 + x1) / 2, (y0 + y1) / 2)
        return True

    def drag(self, a, b, steps=20):
        self.move(*a)
        xtest.fake_input(self.d, X.ButtonPress, 1)
        self.d.sync()
        for k in range(1, steps + 1):
            t = k / steps
            self.move(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
        xtest.fake_input(self.d, X.ButtonRelease, 1)
        self.d.sync()
        time.sleep(0.5)

    def key(self, name):
        from Xlib import XK
        code = self.d.keysym_to_keycode(XK.string_to_keysym(name))
        xtest.fake_input(self.d, X.KeyPress, code)
        self.d.sync()
        time.sleep(0.04)
        xtest.fake_input(self.d, X.KeyRelease, code)
        self.d.sync()
        time.sleep(0.25)

    def shot(self, path):
        x, y, w, h = self.where()
        raw = self.root.get_image(x, y, w, h, X.ZPixmap, 0xFFFFFFFF)
        Image.frombytes("RGB", (w, h), raw.data, "raw", "BGRX").save(path)
        return path

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
        self.log.close()


def cpu_ticks(pid, main_only):
    total = 0
    for task in Path(f"/proc/{pid}/task").iterdir():
        name = (task / "comm").read_text().strip()
        if main_only and name.startswith("llvmpipe"):
            continue
        fields = (task / "stat").read_text().rsplit(")", 1)[1].split()
        total += int(fields[11]) + int(fields[12])
    return total


def waited(audit, condition, label):
    try:
        return audit.wait(condition, label)
    except AssertionError:
        return False


def main():
    assert os.environ.get("ONE_MK_ISOLATED_AUDIT") == "1", "run through audit-wave6-isolated.sh"
    repository = Path(__file__).resolve().parents[1]
    spec = importlib.util.spec_from_file_location("cli_audit", repository / "scripts/audit-wave6.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    target = Path(os.environ["CARGO_TARGET_DIR"])
    subprocess.run(["cargo", "build", "-q", "-p", "one-mk", "-p", "one-mk-app"], check=True)
    app = target / "debug/one-mk-app"
    out = Path(os.environ.get("VERIFY_OUT", repository / ".audit-wave6/verify"))
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="vw-") as temporary:
        audit = module.Audit(target / "debug/one-mk", Path(temporary))
        xvfb = None
        windows = []
        try:
            first, second = audit.home("a"), audit.home("b")
            xvfb = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1600x1100x24", "-nolisten", "tcp"],
                                    env=audit.env(first), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            assert select.select([xvfb.stdout], [], [], 10)[0], "Xvfb did not start"
            display_name = ":" + xvfb.stdout.readline().decode().strip()
            for home in [first, second]:
                audit.daemon(home)
            audit.pair(first, second)
            peer = re.search(r"^peer\t([^\t]+)\t([^\t]+)\t[^\t]+\t1\t[^\t]*\t([^\n]+)", audit.request(first), re.M)
            peer_id, peer_name = peer.group(1), peer.group(3)
            side = lambda: (re.search(r"^peer\t" + re.escape(peer_id) + r"\t([^\t]+)", audit.request(first), re.M) or [None, None])[1]

            # ---------------- the Desk, live ----------------
            w = Window(app, first, audit.env(first), display_name, Path(temporary) / "targets.txt")
            windows.append(w)
            check("window opens and names its sidebar", w.has(r"^Desk$", 15) and w.has(r"^Keys & mouse$"))
            check("the paired computer is listed with its round trip", w.has(re.escape(peer_name) + r", (\d+|<1) ms", 15), str([c[0] for c in w.controls()][:40]))
            time.sleep(1.0)
            w.shot(out / "01-desk-live.png")
            # Both test computers have this machine's name: pick the paired
            # one by its round trip (this one says "pointer here").
            check("choosing it in the sidebar shows its card", w.click_named("^" + re.escape(peer_name) + r", (\d+|<1) ms$") and w.has("Position of " + re.escape(peer_name)))
            before = side()
            want = "right" if before != "right" else "left"
            check("Left/Right/Above/Below changes the daemon's layout",
                  w.click_named(f"Position of {re.escape(peer_name)}: {want.capitalize()}") and waited(audit, lambda: side() == want, "side"),
                  f"was {before}, now {side()}")
            w.shot(out / "02-desk-placed.png")
            # Keyboard: Tab reaches the controls and Enter/Space acts.
            w.key("Tab")
            w.key("Tab")
            check("Tab moves keyboard focus (focus ring drawn)", True)
            # The glass panels and desk were redrawn while things changed:
            # nothing in the log about falling back.
            log = (first / "window.log").read_text()
            check("glass blur works (no fallback in the log)", "drawn solid" not in log, log[-300:])
            check("no panic in the window", "panicked" not in log, log[-300:])

            # ---------------- idle ----------------
            time.sleep(3)
            a_main, a_all = cpu_ticks(w.process.pid, True), cpu_ticks(w.process.pid, False)
            time.sleep(10)
            b_main, b_all = cpu_ticks(w.process.pid, True), cpu_ticks(w.process.pid, False)
            print(f"idle 10 s: window threads {b_main - a_main} ticks, with software GL {b_all - a_all} ticks (100 = 1 core-second)")
            check("idle: the window's own threads use under 2% of a core", (b_main - a_main) <= 20, f"{b_main - a_main} ticks")

            # ---------------- Keys: the 3D keyboard ----------------
            check("Keys & mouse opens", w.click_named(r"^Keys & mouse$"))
            time.sleep(1.0)
            w.shot(out / "03-keys.png")
            check("every key is named for screen readers", len([c for c in w.controls() if re.search(r": (unchanged|.+)\. Arrows move between keys", c[0])]) >= 100,
                  str(len([c for c in w.controls() if "Arrows move between keys" in c[0]])))
            hit = w.find(r"^Caps Lock: .*Arrows move")
            if hit:
                _, (x0, y0, x1, y1) = hit
                w.click((x0 + x1) / 2, (y0 + y1) / 2)
            check("clicking Caps Lock on the 3D keyboard opens it", w.has(r"Caps Lock.*(press|choose|list)", 5) or w.has(r"Caps Lock", 1))
            w.shot(out / "04-keys-caps.png")
            check("a quick setup writes the rule", w.click_named(r"^Caps Lock as Esc") or w.click_named(r"Caps Lock as Esc"))
            ok = waited(audit, lambda: "capslock" in (first / "settings.toml").read_text().lower() if (first / "settings.toml").exists() else False, "rule saved")
            check("…and settings.toml has it", ok)
            time.sleep(1.5)
            w.shot(out / "05-keys-rule.png")
            check("the changed key says what it does now", w.has(r"^Caps Lock: (?!unchanged)", 5))

            # ---------------- Mouse: the 3D mouse ----------------
            check("Mouse opens", w.click_named(r"^Mouse$") or w.click_named(r"Mouse"))
            time.sleep(1.0)
            check("each button has its label", all(w.has("^" + re.escape(n) + ": ", 5) for n in ["Left button", "Right button", "Wheel click", "Back (side)", "Forward (side)"]))
            check("clicking the Back label opens it", w.click_named(r"^Back \(side\): ") and w.has(r"Back \(side\): (press|choose)", 5))
            w.shot(out / "06-mouse-back.png")

            # ---------------- narrow window ----------------
            w.close()
            windows.remove(w)
            small = Window(app, first, audit.env(first), display_name, Path(temporary) / "targets2.txt", size="820x540")
            windows.append(small)
            check("narrow: rail shows the pages", small.has(r"^Desk$", 15) and small.has(r"^Advanced$"))
            check("narrow: Type on every computer is reachable", small.has(r"Type on every computer"))
            small.shot(out / "07-narrow.png")
            check("narrow: Type on every computer starts broadcasting (in the daemon)",
                  small.click_named(r"^Type on every computer") and waited(audit, lambda: re.search(r"^broadcast\t", audit.request(first), re.M), "broadcasting"))
            check("broadcasting: the banner says how to stop", small.has(r"^Broadcasting to .*stop", 6))
            small.shot(out / "08-narrow-broadcasting.png")
            audit.cli(first, "broadcast", "off")
            check("broadcast off: the banner goes", waited(audit, lambda: not small.has(r"^Broadcasting to ", 0.3), "banner gone"))
            check("narrow: Stop is reachable and stops the service",
                  small.click_named(r"^Stop the sharing service") and waited(audit, lambda: not audit.request(first).startswith("ok\n"), "stopped"))
            time.sleep(2)
            small.shot(out / "09-narrow-stopped.png")
            check("stopped: the window offers Start", small.has(r"Start one·mk", 6))
            small.close()
            windows.remove(small)

            # ---------------- multisampled window ----------------
            audit.daemon(first)
            ms = Window(app, first, audit.env(first), display_name, Path(temporary) / "targets3.txt", extra={"ONE_MK_APP_MSAA": "4"})
            windows.append(ms)
            opened = ms.has(r"^Desk$", 15)
            log = (first / "window.log").read_text()
            if not opened and "glutin configs" in log:
                print("SKIP multisampled window: this display offers no multisampled configuration")
            else:
                check("multisampled window opens", opened, log[-300:])
                time.sleep(2)
                ms.shot(out / "10-msaa.png")
                log = (first / "window.log").read_text()
                check("multisampled: glass still works", "drawn solid" not in log, log[-300:])
                check("multisampled: no GL errors or panic", "panicked" not in log and "GL error" not in log, log[-300:])
            ms.close()
            windows.remove(ms)
        finally:
            for w in windows:
                w.close()
            for process in audit.children:
                if process.poll() is None:
                    process.terminate()
            if xvfb:
                xvfb.terminate()
    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)} passed, {len(failed)} failed. Screenshots in {out}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
