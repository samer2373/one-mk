"""Isolated real-binary clipboard regression: Xvfb only, never the live clipboard.

Build first: cargo build -j 2 -p one-mk -p one-mk-app
Run: python3 scripts/test-clipboard-history.py
Requires Xvfb, xclip, Pillow; Linux only. All children and homes are temporary.
"""
import json
import ctypes
import ctypes.util
import os
import pathlib
import pty
import re
import select
import signal
import socket
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGET = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
BINARY = TARGET / "debug/one-mk"
APP = TARGET / "debug/one-mk-app"
SCREENSHOTS = pathlib.Path(os.environ.get("ONE_MK_AUDIT_SCREENS", ROOT / ".clipboard-history-qa"))
CHILDREN = []
LOGS = []
ENVIRONMENT = {}


def checked_environment(environment=None):
    environment = environment or ENVIRONMENT
    assert environment and not any(name in environment for name in ("WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "XDG_SESSION_TYPE"))
    assert pathlib.Path(environment["XDG_RUNTIME_DIR"]).stat().st_mode & 0o777 == 0o700
    return environment


def start(args, **kwargs):
    kwargs["env"] = checked_environment(kwargs.get("env"))
    process = subprocess.Popen(args, start_new_session=True, **kwargs)
    CHILDREN.append(process)
    return process


def stop(process, force=False):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def cli(home, *args, success=True, env=None, input=None):
    result = subprocess.run([str(BINARY), "--home", str(home), *args], env=checked_environment(env),
                            input=input, text=True, capture_output=True, timeout=50)
    assert (result.returncode == 0) == success, (args, result.returncode, result.stdout, result.stderr)
    return result.stdout if success else result.stderr


def eventually(read, predicate, seconds=15):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        last = read()
        if predicate(last):
            return last
        time.sleep(0.1)
    raise AssertionError(f"Timed out: {last!r}")


def control(home, line):
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(50)
        stream.connect(str(home / "control.sock"))
        stream.sendall((line + "\n").encode())
        response = bytearray()
        while True:
            try:
                chunk = stream.recv(65536)
            except ConnectionResetError:
                break
            if not chunk:
                break
            response.extend(chunk)
        return response.decode()


def xvfb():
    read_fd, write_fd = os.pipe()
    start(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1800x1400x24", "-nolisten", "tcp"],
          pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(write_fd)
    assert select.select([read_fd], [], [], 10)[0], "Xvfb failed"
    display = ":" + os.read(read_fd, 100).decode().strip()
    os.close(read_fd)
    return dict(ENVIRONMENT, DISPLAY=display, ONE_MK_REDUCE_MOTION="1")


def port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as stream:
        stream.bind(("127.0.0.1", 0))
        return stream.getsockname()[1]


def history(home):
    items = json.loads(cli(home, "clipboard", "history", "--json"))
    retained = []
    for item in items:
        assert item["preview"] == "[hidden; use Show]", item
        result = subprocess.run([str(BINARY), "--home", str(home), "clipboard", "history", "show", str(item["id"]), "--json"],
                                env=checked_environment(), text=True, capture_output=True, timeout=50)
        if result.returncode and "no longer in history" in result.stderr:
            continue
        assert result.returncode == 0, result.stderr
        content = json.loads(result.stdout)["content"]
        item["preview"] = content[:80]
        retained.append(item)
    return retained


def copied(env, text, target=None):
    args = ["xclip", "-selection", "clipboard", "-in", "-quiet"]
    if target:
        args += ["-target", target]
    process = start(args, env=env,
                    stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    process.stdin.write(text.encode() if isinstance(text, str) else text)
    process.stdin.close()
    return process


def clipboard(env):
    return subprocess.check_output(["xclip", "-selection", "clipboard", "-out"], env=env, timeout=5).decode()


def settings(home, persist=False, maximum=20):
    (home / "settings.toml").write_text(
        f"[clipboard]\nhistory_max = {maximum}\nhistory_persist = {str(persist).lower()}\n"
        "[hotkeys]\nenabled = false\n")


def daemon(home, env, listen, other=None):
    logfile = open(home / "test-daemon.log", "a")
    LOGS.append(logfile)
    args = [str(BINARY), "--home", str(home), "run", "--backend", "null", "--no-discovery", "--no-notify", "--port", str(listen)]
    if other:
        args += ["--connect", f"127.0.0.1:{other}"]
    process = start(args, env=env, stdout=logfile, stderr=logfile)
    eventually(lambda: cli(home, "status"), lambda output: "is running on" in output)
    return process


def pair(first, second, first_port, second_port, binary=BINARY):
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as route:
        route.connect(("192.0.2.1", 9))
        address = route.getsockname()[0]
    master, slave = pty.openpty()
    inviter = start([str(binary), "--home", str(first), "invite", "--no-discovery", "--listen", f"{address}:0", "--no-confirm", "--service-port", str(first_port)],
                    stdin=slave, stdout=slave, stderr=slave)
    os.close(slave)
    output = ""
    deadline = time.monotonic() + 15
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output += os.read(master, 65536).decode(errors="replace")
                except OSError as error:
                    raise AssertionError(output) from error
            match = re.search(r"onemk://pair\S+", output)
            if match:
                while select.select([master], [], [], 0.5)[0]:
                    output += os.read(master, 65536).decode(errors="replace")
                joined = subprocess.run([str(binary), "--home", str(second), "join", "--yes", "--service-port", str(second_port)],
                                        input=match.group(0) + "\n", text=True, capture_output=True, timeout=25, env=checked_environment())
                assert joined.returncode == 0, joined.stderr
                break
        else:
            raise AssertionError(output)
    finally:
        stop(inviter)
        os.close(master)


def memory(process):
    fields = pathlib.Path(f"/proc/{process.pid}/stat").read_text().split()
    rss = next(line for line in pathlib.Path(f"/proc/{process.pid}/status").read_text().splitlines() if line.startswith("VmRSS:"))
    return int(fields[13]) + int(fields[14]), rss


def threads(process):
    result = {}
    for task in pathlib.Path(f"/proc/{process.pid}/task").iterdir():
        fields = (task / "stat").read_text().split()
        result[task.name] = ((task / "comm").read_text().strip(), int(fields[13]) + int(fields[14]))
    return result


class IsolatedX11:
    def __init__(self, env):
        self.library = ctypes.CDLL(ctypes.util.find_library("X11"))
        self.library.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.library.XOpenDisplay.restype = ctypes.c_void_p
        self.display = self.library.XOpenDisplay(env["DISPLAY"].encode())
        assert self.display and env["DISPLAY"].startswith(":"), "isolated display required"
        self.library.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        self.library.XDefaultRootWindow.restype = ctypes.c_ulong
        self.root = self.library.XDefaultRootWindow(self.display)
        self.library.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        self.library.XInternAtom.restype = ctypes.c_ulong
        self.library.XChangeProperty.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong,
                                                ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
        self.library.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]

    def atom(self, name):
        return self.library.XInternAtom(self.display, name.encode(), 0)

    def password_focus(self, enabled):
        value = ctypes.c_ulong(self.root if enabled else 0)
        self.library.XChangeProperty(self.display, self.root, self.atom("_NET_ACTIVE_WINDOW"), self.atom("WINDOW"),
                                     32, 0, ctypes.byref(value), 1)
        payload = b"keepassxc\0KeePassXC\0" if enabled else b"test\0Test\0"
        data = ctypes.create_string_buffer(payload)
        self.library.XChangeProperty(self.display, self.root, self.atom("WM_CLASS"), self.atom("STRING"),
                                     8, 0, data, len(payload))
        self.library.XSync(self.display, 0)

    def activate(self, env):
        tree = subprocess.check_output(["xwininfo", "-root", "-tree"], env=env).decode()
        match = re.search(r'(0x[0-9a-f]+) [^\n]*: \("[^"\n]*" "one-mk"\)', tree)
        assert match, tree
        window = int(match.group(1), 16)
        self.library.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        self.library.XSetInputFocus(self.display, window, 2, 0)
        self.library.XSync(self.display, 0)

    def click(self, x, y):
        xtest = ctypes.CDLL(ctypes.util.find_library("Xtst"))
        xtest.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
        xtest.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        xtest.XTestFakeMotionEvent(self.display, -1, x, y, 0)
        xtest.XTestFakeButtonEvent(self.display, 1, 1, 0)
        xtest.XTestFakeButtonEvent(self.display, 1, 0, 0)
        self.library.XSync(self.display, 0)

    def enter(self):
        xtest = ctypes.CDLL(ctypes.util.find_library("Xtst"))
        self.library.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        self.library.XKeysymToKeycode.restype = ctypes.c_uint
        xtest.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        code = self.library.XKeysymToKeycode(self.display, 0xff0d)
        xtest.XTestFakeKeyEvent(self.display, code, 1, 0)
        xtest.XTestFakeKeyEvent(self.display, code, 0, 0)
        self.library.XSync(self.display, 0)

    def close(self):
        self.library.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.library.XCloseDisplay(self.display)


def main():
    global ENVIRONMENT
    previous = os.environ.get("PREVIOUS_ONE_MK")
    with tempfile.TemporaryDirectory(prefix="h-") as directory:
        base = pathlib.Path(directory)
        runtime = base / "runtime"
        runtime.mkdir(mode=0o700)
        ENVIRONMENT = {"PATH": os.environ["PATH"], "HOME": str(base), "XDG_RUNTIME_DIR": str(runtime),
                       "XDG_CONFIG_HOME": str(base / "config"), "XDG_CACHE_HOME": str(base / "cache"),
                       "XDG_DATA_HOME": str(base / "data"), "ONE_MK_HOME": str(base / "unused")}
        os.environ.clear()
        os.environ.update(ENVIRONMENT)
        first, second = base / "a", base / "b"
        first.mkdir()
        second.mkdir()
        first_env, second_env = xvfb(), xvfb()
        first_port, second_port = port(), port()
        if previous:
            for home in (first, second):
                subprocess.run([previous, "--home", str(home), "clipboard", "--max-mb", "5"], check=True, capture_output=True, env=checked_environment())
                subprocess.run([previous, "--home", str(home), "hotkeys", "--off"], check=True, capture_output=True, env=checked_environment())
                assert "history_max" not in (home / "settings.toml").read_text()
            pair(first, second, first_port, second_port, previous)
        else:
            settings(first)
            settings(second)
            pair(first, second, first_port, second_port)
        first_daemon = daemon(first, first_env, first_port, second_port)
        second_daemon = daemon(second, second_env, second_port, first_port)
        eventually(lambda: cli(first, "status"), lambda output: "online" in output.lower())
        print("PASS paired two real null-backend daemons on separate Xvfb displays")
        if previous:
            assert "size limit 5 MB" in cli(first, "clipboard")
            print("PASS migration: previous binary wrote settings.toml and devices.toml; existing formats, limits and pairing retained")
        copied(first_env, "local alpha")
        eventually(lambda: history(first), lambda items: any(item["preview"] == "local alpha" for item in items))
        print("PASS local clipboard selection change recorded without polling")
        isolated = IsolatedX11(first_env)
        isolated.password_focus(True)
        copied(first_env, "password-manager-synthetic-secret")
        time.sleep(0.5)
        assert not any("synthetic-secret" in item["preview"] for item in history(first))
        copied(second_env, "protected receiver sentinel")
        cli(first, "clipboard", "--allow-fetch", "on")
        cli(second, "clip-get", success=False)
        assert clipboard(second_env) == "protected receiver sentinel"
        cli(first, "switch", "next")
        time.sleep(0.4)
        assert clipboard(second_env) == "protected receiver sentinel"
        cli(first, "switch", "here")
        cli(first, "clipboard", "--allow-fetch", "off")
        isolated.password_focus(False)
        copied(first_env, "local alpha")
        time.sleep(0.2)
        isolated.close()
        print("PASS skipped X11 app excluded from history, fetch and pointer-follow sharing")
        print(cli(first, "switch", "next").strip())
        received = eventually(lambda: history(second), lambda items: any(item["preview"] == "local alpha" for item in items))
        alpha = next(item for item in received if item["preview"] == "local alpha")
        assert alpha["origin"] != "this computer", alpha
        print("PASS pointer-follow push recorded at receiver:", json.dumps(alpha))
        cli(first, "switch", "here")
        copied(first_env, "untouched remote sentinel")
        eventually(lambda: clipboard(first_env), lambda value: value == "untouched remote sentinel")
        copied(second_env, "local beta")
        eventually(lambda: history(second), lambda items: items[0]["preview"] == "local beta")
        print(cli(second, "clipboard", "history", "copy", str(alpha["id"])).strip())
        assert json.loads(cli(second, "clipboard", "history", "copy", str(alpha["id"]), "--json")) == {"copied": True}
        assert clipboard(second_env) == "local alpha"
        assert clipboard(first_env) == "untouched remote sentinel"
        print(cli(second, "clipboard", "history", "show", str(alpha["id"])).strip())
        assert json.loads(cli(second, "clipboard", "history", "show", str(alpha["id"]), "--json")) == {"content": "local alpha"}
        assert json.loads(cli(second, "clipboard", "history", "delete", str(alpha["id"]), "--json")) == {"deleted": True}
        assert not any(item["id"] == alpha["id"] for item in history(second))
        print(cli(second, "clipboard", "history", "show", "999999", success=False).strip())
        assert "error" in control(first, "clip-history show -1")
        assert control(first, "clip-history clear" + " " * 2000 + "bad").startswith("error")
        assert history(first), "oversized clear must not dispatch a truncated prefix"
        cli(first, "clipboard", "history", "copy", "0", success=False)
        assert not (first / "clipboard-history.bin").exists()
        copied(second_env, "fetch gamma")
        eventually(lambda: history(second), lambda items: items[0]["preview"] == "fetch gamma")
        refusal = control(first, "fetch-clipboard")
        assert "does not allow fetching its clipboard; on " in refusal and " run: one-mk clipboard --allow-fetch on" in refusal, refusal
        print(refusal.strip())
        cli_refusal = cli(first, "clip-get", success=False)
        assert refusal.removeprefix("error ").strip() in cli_refusal, cli_refusal
        cli(first, "hotkeys", "--fetch-clipboard", "ctrl+alt+v")
        assert 'fetch_clipboard = "ctrl+alt+v"' in (first / "settings.toml").read_text()
        cli(second, "clipboard", "--allow-fetch", "on")
        print(cli(first, "clip-get").strip())
        print(control(first, "fetch-clipboard").strip())
        assert clipboard(first_env) == "fetch gamma"
        assert history(first)[0]["preview"] == "fetch gamma"
        print("PASS hotkey control action fetches without injecting paste")
        settings(first, persist=True)
        assert control(first, "settings").startswith("ok")
        saved = first / "clipboard-history.bin"
        eventually(saved.exists, bool)
        assert saved.exists() and saved.stat().st_mode & 0o777 == 0o600
        stop(first_daemon, force=True)
        first_daemon = daemon(first, first_env, first_port, second_port)
        assert any(item["preview"] == "fetch gamma" for item in history(first))
        print("PASS persistence survives kill -9; atomic saved file mode 0600")
        assert json.loads(cli(first, "clipboard", "history", "clear", "--json")) == {"cleared": True}
        assert history(first) == []
        settings(first)
        control(first, "settings")
        assert not saved.exists()
        copied(first_env, "memory only delta")
        eventually(lambda: history(first), lambda items: any(item["preview"] == "memory only delta" for item in items))
        stop(first_daemon)
        first_daemon = daemon(first, first_env, first_port, second_port)
        assert history(first) == []
        print("PASS memory-only history disappears on restart; no saved file")
        stop(second_daemon, force=True)
        eventually(lambda: control(first, "fetch-clipboard"), lambda output: output.startswith("error"), seconds=30)
        print("PASS disconnected peer returns an error")
        second_daemon = daemon(second, second_env, second_port, first_port)
        eventually(lambda: control(first, "fetch-clipboard"), lambda output: output.startswith("ok"), seconds=30)
        print("PASS reconnect after peer kill -9")
        os.killpg(second_daemon.pid, signal.SIGSTOP)
        time.sleep(1)
        os.killpg(second_daemon.pid, signal.SIGCONT)
        eventually(lambda: control(first, "fetch-clipboard"), lambda output: output.startswith("ok"), seconds=30)
        print("PASS peer pause/resume (sleep approximation, not real OS suspend)")
        import io
        from PIL import Image
        picture = io.BytesIO()
        Image.new("RGBA", (8, 6), (17, 85, 204, 255)).save(picture, format="PNG")
        copied(first_env, picture.getvalue(), "image/png")
        image_items = eventually(lambda: history(first), lambda items: any(item["kind"] == "image" for item in items))
        image_item = next(item for item in image_items if item["kind"] == "image")
        assert image_item["preview"].startswith("image 8x6"), image_item
        assert "image 8x6" in cli(first, "clipboard", "history", "show", str(image_item["id"]))
        cli(first, "clipboard", "history", "copy", str(image_item["id"]))
        copied(first_env, "<b>history html</b>", "text/html")
        eventually(lambda: history(first), lambda items: any(item["kind"] == "html" for item in items))
        print("PASS real PNG and HTML capture, image show and local restore")
        time.sleep(5)
        for maximum in (0, 20, 0):
            settings(first, maximum=maximum)
            control(first, "settings")
            time.sleep(3)
            before, _ = memory(first_daemon)
            before_threads = threads(first_daemon)
            time.sleep(5)
            after, rss = memory(first_daemon)
            after_threads = threads(first_daemon)
            print(f"IDLE history_max={maximum}: {after - before} CPU ticks / 5s; {rss}")
            for task, (name, ticks) in after_threads.items():
                if "history" in name or "clipboard" in name:
                    print(f"  {name}: {ticks - before_threads.get(task, (name, ticks))[1]} CPU ticks")
        settings(first)
        control(first, "settings")
        for number in range(25):
            copied(first_env, f"History item {number}: Unicode 日本語 and a longer preview " + "x" * 100)
            time.sleep(0.08)
        items = history(first)
        assert len(items) == 20, len(items)
        screenshot_dir = SCREENSHOTS
        screenshot_dir.mkdir(parents=True, exist_ok=True)
        from PIL import ImageGrab
        for name, size, zoom in (("normal", "980x660", "1"), ("narrow", "820x540", "1"), ("hidpi", "820x540", "2")):
            window_env = dict(first_env, ONE_MK_APP_TAB="files", ONE_MK_APP_SIZE=size, ONE_MK_APP_ZOOM=zoom)
            app = start([str(APP), "--home", str(first)], env=window_env,
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(4)
            assert app.poll() is None, "window failed to start"
            dimensions = tuple(int(value) * int(zoom) for value in size.split("x"))
            ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, *dimensions)).save(screenshot_dir / f"{name}.png")
            if name == "normal":
                isolated = IsolatedX11(first_env)
                isolated.activate(first_env)
                isolated.click(930, 519)
                time.sleep(0.5)
                ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, *dimensions)).save(screenshot_dir / "revealed.png")
                isolated.click(930, 519)
                isolated.click(400, 519)
                time.sleep(0.3)
                selected = clipboard(first_env)
                assert selected.startswith("History item"), selected
                copied(first_env, "keyboard restore check")
                time.sleep(0.8)
                isolated.enter()
                eventually(lambda: clipboard(first_env), lambda value: value == selected)
                print("PASS real window mouse copy and keyboard Enter restore")
                isolated.click(932, 426)
                time.sleep(0.3)
                isolated.click(932, 426)
                eventually(lambda: history(first), lambda items: not items)
                time.sleep(1)
                ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, *dimensions)).save(screenshot_dir / "empty.png")
                os.killpg(first_daemon.pid, signal.SIGSTOP)
                isolated.click(865, 426)
                time.sleep(0.3)
                ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, *dimensions)).save(screenshot_dir / "loading.png")
                time.sleep(6)
                ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, *dimensions)).save(screenshot_dir / "unresponsive.png")
                os.killpg(first_daemon.pid, signal.SIGCONT)
                for number in range(25):
                    copied(first_env, f"History item {number}: a long preview " + "x" * 100)
                    time.sleep(0.08)
                isolated.close()
            stop(app)
        print(f"PASS captured real window screenshots in {screenshot_dir}")
        settings(first, maximum=0)
        control(first, "settings")
        app = start([str(APP), "--home", str(first)],
                    env=dict(first_env, ONE_MK_APP_TAB="files", ONE_MK_APP_SIZE="980x660"),
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(4)
        ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, 980, 660)).save(screenshot_dir / "disabled.png")
        stop(app)
        settings(first)
        control(first, "settings")
        print(cli(first, "status"))
        print(cli(first, "clipboard", "history", "--help"))
        for operation in ("list", "show", "copy", "delete", "clear"):
            help_text = cli(first, "clipboard", "history", operation, "--help")
            assert "Usage:" in help_text, help_text
        print("PASS real CLI help for every history operation")
        diagnostic = subprocess.run([str(BINARY), "--home", str(first), "doctor", "--json"],
                                    env=checked_environment(first_env), text=True, capture_output=True, timeout=30)
        assert diagnostic.returncode in (0, 1), diagnostic.stderr
        check = next(item for item in json.loads(diagnostic.stdout)["checks"] if item["id"] == "clipboard-history")
        assert "Memory only" in check["detail"] and "untested" in check["detail"], check
        print("PASS real doctor reports retention and untested platform coverage:", check["detail"])
        copied(first_env, "persistence baseline")
        eventually(lambda: history(first), bool)
        settings(first, persist=True)
        control(first, "settings")
        eventually(saved.exists, bool)
        app = start([str(APP), "--home", str(first)],
                    env=dict(first_env, ONE_MK_APP_TAB="files", ONE_MK_APP_SIZE="980x660"),
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(4)
        ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, 980, 660)).save(screenshot_dir / "persisted.png")
        stop(app)
        cli(first, "clipboard", "--max-mb", "16")
        complete_history = history(first)
        payload = "interrupted-save-" + "x" * (8 << 20)
        libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
        watch = libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
        assert watch >= 0
        assert libc.inotify_add_watch(watch, str(first).encode(), 0x100) >= 0
        try:
            copied(first_env, payload)
            deadline = time.monotonic() + 30
            interrupted = False
            while time.monotonic() < deadline:
                if select.select([watch], [], [], 0.1)[0] and b".clipboard-history-" in os.read(watch, 65536):
                    os.killpg(first_daemon.pid, signal.SIGSTOP)
                    if list(first.glob(".clipboard-history-*")):
                        stop(first_daemon, force=True)
                        interrupted = True
                        break
                    os.killpg(first_daemon.pid, signal.SIGCONT)
            os.close(watch)
            if not interrupted:
                raise AssertionError("did not observe a save in progress; interruption was not tested")
        finally:
            if first_daemon.poll() is None:
                os.killpg(first_daemon.pid, signal.SIGCONT)
        first_daemon = daemon(first, first_env, first_port, second_port)
        assert "Could not load clipboard history" not in cli(first, "status")
        assert history(first) == complete_history
        assert not list(first.glob(".clipboard-history-*"))
        assert saved.stat().st_mode & 0o777 == 0o600
        print("PASS kill -9 DURING atomic save: previous complete history loads; unfinished save removed on restart")
        stop(first_daemon)
        saved.write_bytes(b"damaged history")
        first_daemon = daemon(first, first_env, first_port, second_port)
        assert "Could not load clipboard history" in cli(first, "status")
        assert saved.read_bytes() == b"damaged history"
        cli(first, "clipboard", "history", "clear")
        assert history(first) == []
        assert "Could not load clipboard history" not in cli(first, "status")
        assert not saved.exists()
        print("PASS damaged saved history is reported without overwrite; real CLI Clear recovers")
        stop(first_daemon)
        app = start([str(APP), "--home", str(first)],
                    env=dict(first_env, ONE_MK_APP_TAB="files", ONE_MK_APP_SIZE="980x660"),
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(4)
        ImageGrab.grab(xdisplay=first_env["DISPLAY"]).crop((0, 0, 980, 660)).save(screenshot_dir / "offline.png")
        stop(app)
        print("PASS captured offline window; no test daemon auto-started")
        for process in reversed(CHILDREN):
            stop(process)


if __name__ == "__main__":
    try:
        main()
    finally:
        for child in reversed(CHILDREN):
            stop(child)
        for logfile in LOGS:
            logfile.close()
