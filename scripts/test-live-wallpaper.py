#!/usr/bin/env python3
"""Xvfb/system-GStreamer seam, looping, occlusion, parent-EOF and cost checks."""
import ctypes as ffi
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import time
from PIL import Image


class XImage(ffi.Structure):
    _fields_ = [("width", ffi.c_int), ("height", ffi.c_int), ("offset", ffi.c_int),
                ("format", ffi.c_int), ("data", ffi.c_void_p), ("byte_order", ffi.c_int),
                ("unit", ffi.c_int), ("bit_order", ffi.c_int), ("pad", ffi.c_int),
                ("depth", ffi.c_int), ("stride", ffi.c_int), ("bits", ffi.c_int)]


def main():
    target = Path(os.environ["CARGO_TARGET_DIR"])
    player = target / "debug/one-mk-wallpaper"
    output = Path("/tmp/wallpaper-shots")
    output.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="wallpaper-e2e-") as temporary:
        root = Path(temporary)
        server = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1280x360x24", "-nolisten", "tcp", "-noreset"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        process = None
        try:
            assert select.select([server.stdout], [], [], 10)[0]
            display = b":" + server.stdout.readline().strip()
            environment = dict(os.environ, DISPLAY=display.decode())
            environment.pop("WAYLAND_DISPLAY", None)
            for name, geometry in [("left", "640/160x360/90+0+0"), ("right", "640/160x360/90+640+0")]:
                subprocess.run(["xrandr", "--setmonitor", name, geometry, "screen" if name == "left" else "none"], env=environment, check=True, capture_output=True)
            topology = subprocess.check_output(["xrandr", "--listmonitors"], env=environment)
            (output / "xrandr.txt").write_bytes(topology)
            assert b"Monitors: 2" in topology, topology
            image = Image.new("RGB", (1280, 360))
            image.putdata([(int(x * 255 / 1279), int(y * 255 / 359), int((1279 - x) * 255 / 1279)) for y in range(360) for x in range(1280)])
            poster = root / "gradient.png"
            image.save(poster)
            video = root / "gradient.webm"
            subprocess.run(["gst-launch-1.0", "-q", "filesrc", f"location={poster}", "!", "pngdec", "!", "imagefreeze", "num-buffers=120", "!", "video/x-raw,framerate=30/1", "!", "videoconvert", "!", "vp8enc", "threads=2", "deadline=1", "!", "webmmux", "!", "filesink", f"location={video}"], check=True, timeout=120)
            media = json.loads(subprocess.check_output([player, "--probe", video, output / "poster.png"], env=environment, timeout=40))
            monitors = [{"computer": "host", "name": name, "rect": {"x": x, "y": 0, "width": 640, "height": 360}, "scale": 1, "online": True} for name, x in [("left", 0), ("right", 640)]]
            plan = {"hash": "a" * 64, "title": "Gradient", "media": media, "canvas": {"monitors": monitors}, "start_ms": int(time.time() * 1000), "fit": "fill", "computers": ["host"], "pause_on_battery": False, "paused": False}
            config = {"path": str(video), "plan": plan, "computer": "host", "clock_offset_ms": 0}
            config_path = root / "player.json"
            config_path.write_text(json.dumps(config))
            library = ffi.CDLL("libX11.so.6")
            library.XOpenDisplay.argtypes = [ffi.c_char_p]
            library.XOpenDisplay.restype = ffi.c_void_p
            library.XDefaultRootWindow.argtypes = [ffi.c_void_p]
            library.XDefaultRootWindow.restype = ffi.c_ulong
            connection = library.XOpenDisplay(display)
            window = library.XDefaultRootWindow(connection)
            library.XGetImage.argtypes = [ffi.c_void_p, ffi.c_ulong, ffi.c_int, ffi.c_int, ffi.c_uint, ffi.c_uint, ffi.c_ulong, ffi.c_int]
            library.XGetImage.restype = ffi.POINTER(XImage)
            library.XDestroyImage.argtypes = [ffi.POINTER(XImage)]
            library.XCreateSimpleWindow.argtypes = [ffi.c_void_p, ffi.c_ulong, ffi.c_int, ffi.c_int, ffi.c_uint, ffi.c_uint, ffi.c_uint, ffi.c_ulong, ffi.c_ulong]
            library.XCreateSimpleWindow.restype = ffi.c_ulong
            library.XMapWindow.argtypes = [ffi.c_void_p, ffi.c_ulong]
            library.XDestroyWindow.argtypes = [ffi.c_void_p, ffi.c_ulong]
            library.XSync.argtypes = [ffi.c_void_p, ffi.c_int]
            library.XInternAtom.argtypes = [ffi.c_void_p, ffi.c_char_p, ffi.c_int]
            library.XInternAtom.restype = ffi.c_ulong
            library.XChangeProperty.argtypes = [ffi.c_void_p, ffi.c_ulong, ffi.c_ulong, ffi.c_ulong, ffi.c_int, ffi.c_int, ffi.c_void_p, ffi.c_int]

            def shot(name):
                snapshot = library.XGetImage(connection, window, 0, 0, 1280, 360, ffi.c_ulong(-1).value, 2)
                contents = snapshot.contents
                result = Image.frombytes("RGB", (1280, 360), ffi.string_at(contents.data, contents.stride * contents.height), "raw", "BGRX", contents.stride)
                library.XDestroyImage(snapshot)
                result.save(output / name)
                return result

            def ticks():
                values = Path(f"/proc/{process.pid}/stat").read_text().rsplit(")", 1)[1].split()
                return int(values[11]) + int(values[12])

            def cpu(seconds=5):
                before = ticks()
                started = time.monotonic()
                time.sleep(seconds)
                return (ticks() - before) / os.sysconf("SC_CLK_TCK") / (time.monotonic() - started) * 100

            def switches():
                counts = {}
                for task in Path(f"/proc/{process.pid}/task").iterdir():
                    try:
                        counts[task.name] = sum(int(line.split()[1]) for line in (task / "status").read_text().splitlines() if "ctxt_switches:" in line)
                    except FileNotFoundError:
                        pass
                return counts

            def paused_cost(churn=False, seconds=10):
                before_cpu, before_switches = ticks(), switches()
                title = library.XInternAtom(connection, b"_NET_WM_NAME", 0)
                user_time = library.XInternAtom(connection, b"_NET_WM_USER_TIME", 0)
                started = time.monotonic()
                while time.monotonic() - started < seconds:
                    if churn:
                        value = (ffi.c_ulong * 1)(int(time.monotonic() * 1000))
                        for atom in [title, user_time]:
                            library.XChangeProperty(connection, blocker, atom, 6, 32, 0, value, 1)
                        library.XSync(connection, 0)
                    time.sleep(0.05)
                elapsed, after = time.monotonic() - started, switches()
                return {"seconds": round(elapsed, 2), "cpu_percent": round((ticks() - before_cpu) / os.sysconf("SC_CLK_TCK") / elapsed * 100, 3),
                        "main_context_switches": after[str(process.pid)] - before_switches[str(process.pid)],
                        "all_thread_context_switches": sum(max(0, count - before_switches.get(task, count)) for task, count in after.items())}

            log = open(output / "x11-player.log", "w")
            process = subprocess.Popen([player, config_path], stdin=subprocess.PIPE, env=environment, stderr=log)
            time.sleep(6)
            assert process.poll() is None, "player exited; see x11-player.log"
            screenshot = shot("x11-seam.png")
            for x in [0, 320, 638, 639, 640, 641, 960, 1279]:
                actual = screenshot.getpixel((x, 180))
                expected = image.getpixel((x, 180))
                assert max(abs(first - second) for first, second in zip(actual, expected)) <= 18, (x, actual, expected)
            blocker = library.XCreateSimpleWindow(connection, window, 0, 0, 1280, 360, 0, 0, 0)
            library.XMapWindow(connection, blocker)
            atom = library.XInternAtom(connection, b"_NET_CLIENT_LIST_STACKING", 0)
            values = (ffi.c_ulong * 1)(blocker)
            library.XChangeProperty(connection, window, atom, 33, 32, 0, values, 1)
            library.XSync(connection, 0)
            time.sleep(2)
            assert (root / "playback.txt").read_text() == "Paused while the desktop is covered"
            measurements = {"occluded_idle": paused_cost(), "occluded_unrelated_window_changes": paused_cost(churn=True)}
            library.XDestroyWindow(connection, blocker)
            library.XChangeProperty(connection, window, atom, 33, 32, 0, None, 0)
            library.XSync(connection, 0)
            time.sleep(2)
            assert (root / "playback.txt").read_text() == "Playing"
            process.stdin.close()
            assert process.wait(timeout=5) == 0
            plan["paused"] = True
            config_path.write_text(json.dumps(config))
            process = subprocess.Popen([player, config_path], stdin=subprocess.PIPE, env=environment, stderr=log)
            time.sleep(3)
            assert process.poll() is None
            measurements["user_paused_idle"] = paused_cost()
            cover = library.XCreateSimpleWindow(connection, window, 0, 0, 1280, 360, 0, 0, 0)
            library.XMapWindow(connection, cover)
            library.XSync(connection, 0)
            time.sleep(0.2)
            library.XDestroyWindow(connection, cover)
            library.XSync(connection, 0)
            time.sleep(0.2)
            exposed = shot("user-paused-expose.png")
            for column in [0, 320, 638, 639, 640, 641, 960, 1279]:
                assert max(abs(actual - expected) for actual, expected in zip(exposed.getpixel((column, 180)), image.getpixel((column, 180)))) <= 18, "paused frame did not repaint after expose"
            process.stdin.close()
            assert process.wait(timeout=5) == 0
            plan["paused"] = False
            for title, width, height in [("1080p", 1920, 1080), ("4k", 3840, 2160)]:
                movie = root / (title + ".webm")
                subprocess.run(["gst-launch-1.0", "-q", "videotestsrc", "pattern=ball", "num-buffers=120", "!", f"video/x-raw,width={width},height={height},framerate=30/1", "!", "vp8enc", "threads=2", "deadline=1", "!", "webmmux", "!", "filesink", f"location={movie}"], check=True, timeout=180)
                config["path"] = str(movie)
                plan["media"] = json.loads(subprocess.check_output([player, "--probe", movie], env=environment, timeout=40))
                plan["start_ms"] = int(time.time() * 1000)
                plan["fit"] = "stretch"
                config_path.write_text(json.dumps(config))
                process = subprocess.Popen([player, config_path], stdin=subprocess.PIPE, env=environment, stderr=log)
                time.sleep(3)
                assert process.poll() is None
                measurements[title + "_cpu_percent"] = round(cpu(10), 2)
                first = shot(title + "-loop.png")
                time.sleep(0.2)
                second = shot(title + "-motion.png")
                assert first.tobytes() != second.tobytes(), "video froze after looping"
                process.stdin.close()
                assert process.wait(timeout=5) == 0
            measurements["gpu_percent"] = "not collected: Xvfb CPU benchmark, not a physical GPU presentation test"
            measurements["decoder"] = "system VP8 software decoder; Xvfb, two 640x360 slices, 30 fps source"
            (output / "measurements.json").write_text(json.dumps(measurements, indent=2) + "\n")
            print(json.dumps(measurements, indent=2))
            print("PASS: two slices, seam, loop, occlusion, resume and parent EOF")
        finally:
            if process and process.poll() is None:
                process.kill()
                process.wait()
            server.terminate()
            server.wait()


if __name__ == "__main__":
    main()
