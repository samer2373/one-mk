#!/usr/bin/env python3
"""Only gst subprocesses, explicit private Xvfb, never the session desktop.

Run from repository root. Temporary media stays in ignored target/wall-spike.
CPU is summed user+system, normalized to one second at 60 fps, not wall time.
"""
import csv
import ctypes
import ctypes.util
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from PIL import Image, ImageChops

OUT = Path("docs/spikes/transcripts/wallpaper")
SCRATCH = Path("target/wall-spike")
SCRATCH.mkdir(parents=True, exist_ok=True)


def gst(arguments, **options):
    return subprocess.run(["gst-launch-1.0", "-q", *arguments], check=True, **options)


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def crop_args(width, outputs, sink):
    args = ["tee", "name=split"]
    for index in range(outputs):
        args += ["split.", "!", "queue", "max-size-buffers=2", "!", "videocrop",
                 f"left={index * width // outputs}", f"right={width - (index + 1) * width // outputs}", "!", *sink]
    return args


def benchmark():
  with (OUT / "gst-benchmark.csv").open("w") as output:
    writer = csv.writer(output)
    writer.writerow(["width", "height", "outputs", "decoders", "repeat", "frames_per_decoder", "wall_s", "cpu_s", "cpu_percent_at_60fps", "max_rss_mib", "decode_fps"])
    for width, height in [(1920, 1080), (3840, 2160), (7680, 2160), (11520, 2160)]:
        clip = SCRATCH / f"{width}.h264"
        gst(["videotestsrc", "num-buffers=120", "pattern=smpte", "!",
             f"video/x-raw,width={width},height={height},framerate=60/1,format=I420", "!",
             "x264enc", "speed-preset=ultrafast", "tune=zerolatency", "threads=2", "key-int-max=60", "!",
             "video/x-h264,stream-format=byte-stream", "!", "filesink", f"location={clip}"])
        for outputs in [1, 2, 3]:
            for decoders in sorted({1, outputs}):
                for repeat in range(3):
                    args = []
                    for decoder in range(decoders):
                        args += ["filesrc", f"location={clip}", "!", "h264parse", "!", "avdec_h264", "max-threads=2", "!"]
                        if decoders == 1:
                            args += crop_args(width, outputs, ["fakesink", "sync=false"])
                        else:
                            args += ["videocrop", f"left={decoder * width // outputs}", f"right={width - (decoder + 1) * width // outputs}", "!", "fakesink", "sync=false"]
                    timing = SCRATCH / "time.txt"
                    subprocess.run(["/usr/bin/time", "-f", "%e,%U,%S,%M", "-o", str(timing), "gst-launch-1.0", "-q", *args], check=True)
                    elapsed, user, system, rss = map(float, timing.read_text().strip().split(","))
                    cpu = user + system
                    writer.writerow([width, height, outputs, decoders, repeat, 120, elapsed, cpu, cpu / 2 * 100, rss / 1024, 120 / elapsed])
                    output.flush()
        clip.unlink()


def screenshots():
    display = ":187"
    if Path("/tmp/.X187-lock").exists() or Path("/tmp/.X11-unix/X187").exists():
        raise RuntimeError("private display :187 already occupied; refusing to reuse")
    for width, height, outputs in [(960, 320, 2), (1440, 480, 3)]:
        xvfb = subprocess.Popen(["Xvfb", display, "-screen", "0", f"{width}x{height}x24", "-nolisten", "tcp", "-ac"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        pipeline = None
        try:
            time.sleep(0.5)
            xvfb.poll()
            if xvfb.returncode is not None:
                raise RuntimeError("Xvfb failed")
            clip = SCRATCH / "crop.h264"
            reference = SCRATCH / "reference.png"
            gst(["videotestsrc", "num-buffers=1", "pattern=smpte", "!", f"video/x-raw,width={width},height={height},format=I420", "!",
                 "x264enc", "speed-preset=ultrafast", "tune=zerolatency", "threads=2", "!", "video/x-h264,stream-format=byte-stream", "!", "filesink", f"location={clip}"])
            gst(["filesrc", f"location={clip}", "!", "h264parse", "!", "avdec_h264", "max-threads=2", "!", "videoconvert", "dither=none", "!", "video/x-raw,format=RGB", "!", "pngenc", "!", "filesink", f"location={reference}"])
            args = ["gst-launch-1.0", "-q", "filesrc", f"location={clip}", "!", "h264parse", "!", "avdec_h264", "max-threads=2", "!", "imagefreeze", "!",
                    *crop_args(width, outputs, ["videoconvert", "dither=none", "!", "video/x-raw,format=BGRx", "!", "ximagesink", f"display={display}", "sync=false", "force-aspect-ratio=false"])]
            pipeline = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env={**os.environ, "DISPLAY": display})
            time.sleep(1)
            tree = subprocess.check_output(["xwininfo", "-display", display, "-root", "-tree"], text=True)
            windows = re.findall(r"(0x[0-9a-f]+).*?: \(\"gst-launch-1.0\"", tree)
            if len(windows) != outputs:
                raise RuntimeError(f"expected {outputs} private windows: {tree}")
            xlib = ctypes.CDLL(ctypes.util.find_library("X11"))
            xlib.XOpenDisplay.restype = ctypes.c_void_p
            xlib.XMoveResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint]
            xlib.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
            xlib.XCloseDisplay.argtypes = [ctypes.c_void_p]
            connection = xlib.XOpenDisplay(display.encode())
            if not connection:
                raise RuntimeError("private X connection unavailable")
            try:
                for index, window in enumerate(reversed(windows)):
                    xlib.XMoveResizeWindow(connection, int(window, 16), index * width // outputs, 0, width // outputs, height)
                xlib.XSync(connection, 0)
                time.sleep(0.4)
                provisional = SCRATCH / "provisional.png"
                gst(["ximagesrc", f"display-name={display}", "show-pointer=false", "num-buffers=1", "!", "videoconvert", "!", "pngenc", "!", "filesink", f"location={provisional}"])
                captured = Image.open(provisional).convert("RGB")
                expected = Image.open(reference).convert("RGB")
                assigned = set()
                for index, window in enumerate(reversed(windows)):
                    region = captured.crop((index * width // outputs, 0, (index + 1) * width // outputs, height))
                    matches = []
                    for destination in range(outputs):
                        candidate = expected.crop((destination * width // outputs, 0, (destination + 1) * width // outputs, height))
                        difference = ImageChops.difference(region, candidate)
                        if max(value[1] for value in difference.getextrema()) <= 3:
                            matches.append(destination)
                    assert len(matches) == 1 and matches[0] not in assigned, "missing/duplicated/wrong crop"
                    assigned.add(matches[0])
                    xlib.XMoveResizeWindow(connection, int(window, 16), matches[0] * width // outputs, 0, width // outputs, height)
                xlib.XSync(connection, 0)
                provisional.unlink()
            finally:
                xlib.XCloseDisplay(connection)
            time.sleep(0.4)
            screenshot = OUT / f"private-xvfb-{width}-{outputs}.png"
            gst(["ximagesrc", f"display-name={display}", "show-pointer=false", "num-buffers=1", "!", "videoconvert", "!", "pngenc", "!", "filesink", f"location={screenshot}"])
            actual = Image.open(screenshot).convert("RGB")
            expected = Image.open(reference).convert("RGB")
            differences = ImageChops.difference(actual, expected)
            print(f"crop {width}x{height}, {outputs} windows: full-image difference bbox={differences.getbbox()}")
            maximum = max(value[1] for value in differences.getextrema())
            print(f"full-image max channel error={maximum}")
            assert maximum <= 3, "crop placement/color differs from decoded reference"
            for boundary in range(1, outputs):
                column = boundary * width // outputs
                seam = differences.crop((column - 1, 0, column + 1, height))
                print(f"boundary x={column}: max channel error={max(value[1] for value in seam.getextrema())}")
        finally:
            if pipeline is not None:
                stop(pipeline)
            stop(xvfb)
            for name in ["crop.h264", "reference.png", "provisional.png"]:
                (SCRATCH / name).unlink(missing_ok=True)


if "--screenshots-only" not in sys.argv:
    benchmark()
if "--benchmark-only" not in sys.argv:
    screenshots()
