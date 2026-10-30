#!/usr/bin/env python3
"""M1 STOP gate, not an importer or shipping runtime.

Only generated VP9 is decoded. Uses the original 6B private-bus headless
harness. Never connects to a live desktop or installs a user extension.
The host runtime is an explicitly UNQUALIFIED laboratory dependency.
"""

import ctypes
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import runpy
import shutil
import signal
import stat
import subprocess
import time
import traceback


HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
LAB = REPO / "target/wall-m1-lab"
ROOT = LAB / "wall-m1-session"
BINARY = REPO / "target/debug/one-mk-wall"
REPORT = LAB / "presentation.json"
HARNESS = HERE / "isolated-session.sh"
BRIDGE = 'Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj?.bridge'
PLUGINS = ["coreelements", "matroska", "vpx", "videoconvertscale", "waylandsink", "videotestsrc", "app"]


def shell_warnings(log):
    return [line for line in log.splitlines()
            if re.search(r'WARNING|CRITICAL|Gjs[- ](?:ERROR|error)|JS (?:ERROR|WARNING)', line)]


def warning_identity(line):
    line = re.sub(r':\d+\)', ':PID)', line)
    line = re.sub(r'\b\d{2}:\d{2}:\d{2}\.\d+\b', 'TIME', line)
    return re.sub(r'\b:1\.\d+\b', ':1.CONNECTION', line)


def new_warnings(log, baseline):
    identities = {warning_identity(record) for record in warning_records(baseline)}
    return [record for record in warning_records(log) if warning_identity(record) not in identities]


def warning_records(log):
    lines = log.splitlines()
    records = []
    for index, line in enumerate(lines):
        if not shell_warnings(line):
            continue
        record = [line]
        for following in lines[index + 1:]:
            if not following.strip():
                continue
            if following.startswith(('  ', '\t', 'Stack trace:')):
                record.append(following.rstrip())
            else:
                break
        records.append('\n'.join(record))
    return records


def clean_environment():
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": str(LAB / "home"),
        "XDG_RUNTIME_DIR": str(LAB / "run"),
        "ONE_MK_HOME": str(LAB / "one-mk"),
        "USER": pwd.getpwuid(os.getuid()).pw_name,
        "SPIKE": str(ROOT),
        "FAKE_GDM": "1",
        "PYTHONDONTWRITEBYTECODE": "1",
    }


def command(arguments, environment, timeout=40):
    completed = subprocess.run(arguments, env=environment, cwd=REPO, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    if completed.returncode:
        raise RuntimeError(f"{arguments!r} exit={completed.returncode}\n{completed.stdout}\n{completed.stderr}")
    return completed.stdout.strip()


def private(arguments):
    return command([str(HARNESS), "run", *arguments], clean_environment())


def evaluate(source):
    result = private(["python3", str(HERE / "client/shell_eval.py"), source])
    if result.startswith("EVAL FAILED"):
        raise RuntimeError(result)
    return json.loads(result)


def snapshot():
    return evaluate(f"""(() => {{
        const bridge = {BRIDGE};
        const renderer = bridge?._renderer;
        const frame = renderer?.win.get_frame_rect();
        const buffer = renderer?.win.get_buffer_rect();
        return {{monitors: bridge?._monitorsVariant() ?? [],
            frame: frame ? [frame.x, frame.y, frame.width, frame.height] : null,
            buffer: buffer ? [buffer.x, buffer.y, buffer.width, buffer.height] : null,
            damage: bridge?.stats.damage ?? 0, stale: bridge?._stale ?? false,
            fresh: renderer?.firstDamage ?? false,
            watchdog: bridge?._watchdogId ?? 0,
            restart: bridge?._restartId ?? 0,
            cloneCount: bridge?._clones.length ?? 0,
            events: bridge?.stats.events.slice(-15) ?? [],
            focus: global.display.focus_window?.get_title() ?? null}};
    }})()""")


def direct_children(process_id):
    try:
        return [int(value) for value in Path(f"/proc/{process_id}/task/{process_id}/children").read_text().split()]
    except (FileNotFoundError, PermissionError):
        return []


def descendants(process_id):
    result = []
    for child in direct_children(process_id):
        result.extend(descendants(child))
        result.append(child)
    return result


def cleanup():
    bus_path = None
    bus_record = ROOT / "session-bus"
    if bus_record.exists():
        address = bus_record.read_text().strip().split(",", 1)[0]
        if address.startswith("unix:path=/tmp/dbus-"):
            bus_path = Path(address.removeprefix("unix:path="))
    for termination in [signal.SIGTERM, signal.SIGKILL]:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            for child in descendants(os.getpid()):
                try:
                    os.kill(child, termination)
                except (ProcessLookupError, PermissionError):
                    pass
            try:
                while os.waitpid(-1, os.WNOHANG)[0]:
                    pass
            except ChildProcessError:
                break
            time.sleep(0.02)
    remaining = direct_children(os.getpid())
    if remaining:
        raise RuntimeError(f"owned children remain after cleanup: {remaining}")
    if bus_path and bus_path.exists():
        metadata = bus_path.lstat()
        if stat.S_ISSOCK(metadata.st_mode) and metadata.st_uid == os.getuid():
            bus_path.unlink()
    log = (ROOT / 'log/shell.log').read_text(errors='replace') if (ROOT / 'log/shell.log').exists() else ''
    if ROOT.exists():
        shutil.rmtree(ROOT)
    return log


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--normal-window', action='store_true')
    parser.add_argument('--trace', action='store_true')
    parser.add_argument('--label', default='presentation')
    parser.add_argument('--sink', choices=['wayland', 'own'], default='own')
    parser.add_argument('--matrix', action='store_true')
    parser.add_argument('--baseline', action='store_true')
    parser.add_argument('--baseline-log', type=Path)
    parser.add_argument('--binary', type=Path, default=BINARY)
    args = parser.parse_args()
    args.binary = args.binary.resolve(strict=True)
    if args.baseline and not args.matrix:
        parser.error('--baseline requires the lifecycle matrix')
    if args.baseline and args.baseline_log:
        parser.error('a baseline cannot compare against another baseline')
    if ROOT.exists():
        raise RuntimeError("private session already exists; refusing to reuse or kill another run")
    if ctypes.CDLL(None).prctl(36, 1, 0, 0, 0) != 0:
        raise RuntimeError("cannot become child subreaper; refusing unreapable headless session")
    for directory in [LAB, LAB / "home", LAB / "run", LAB / "one-mk"]:
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        directory.chmod(0o700)
    report = {"date_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "baseline": args.baseline,
              "status": "failed", "runtime": "host GStreamer: UNQUALIFIED, trusted generated fixture only",
              "stages": [], "measurements": {}}
    report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    if args.baseline_log:
        report['baseline_log'] = {'file': str(args.baseline_log),
                                 'sha256': hashlib.sha256(args.baseline_log.read_bytes()).hexdigest()}
    started = False
    try:
        started = True
        report["harness_start"] = command([str(HARNESS), "start", "3" if args.matrix else "2"], clean_environment())
        ROOT.chmod(0o700)
        plugins = ROOT / "plugins"
        plugins.mkdir(mode=0o700)
        for plugin in PLUGINS:
            source = Path(f"/usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgst{plugin}.so")
            shutil.copyfile(source, plugins / source.name)
        media_environment = clean_environment() | {
            "ONE_MK_WALL_RISK_SINK": args.sink,
            "ONE_MK_WALL_RISK_ROOT": str(ROOT),
            "GST_PLUGIN_PATH": str(plugins), "GST_PLUGIN_PATH_1_0": str(plugins),
            "GST_PLUGIN_SYSTEM_PATH": "", "GST_PLUGIN_SYSTEM_PATH_1_0": "",
            "GST_REGISTRY_1_0": str(ROOT / "cache/registry.bin"), "GST_REGISTRY_FORK": "no",
        }
        if args.trace:
            media_environment['WAYLAND_DEBUG'] = 'client'
        report["runtime_version"] = command(["gst-inspect-1.0", "--version"], media_environment)
        report["native_dependencies"] = {
            plugin: command(["ldd", str(plugins / f"libgst{plugin}.so")], clean_environment())
            for plugin in PLUGINS
        }
        for dependencies in report["native_dependencies"].values():
            if any(forbidden in dependencies.lower() for forbidden in ["libavcodec", "libavformat", "libx264", "libx265", "libmpv"]):
                raise RuntimeError("forbidden dependency in the selected laboratory plugin closure")
        command(["gst-launch-1.0", "-q", "videotestsrc", "num-buffers=1200", "pattern=ball", "!",
                 "video/x-raw,format=I420,width=3840,height=1080,framerate=30/1", "!", "vp9enc",
                 "deadline=1", "cpu-used=8", "threads=2", "!", "webmmux", "!", "filesink",
                 f"location={ROOT}/fixture.webm"], media_environment, timeout=120)
        fixture_hash = command([str(args.binary), "--risk-hash-fixture"], media_environment)
        report["fixture"] = {"blake3": fixture_hash, "bytes": (ROOT / "fixture.webm").stat().st_size,
                             "dimensions": [3840, 1080], "frames": 1200, "fps": 30}
        for extension in ["spike-harness@one-mk.test"] + ([] if args.baseline else ["wallpaper-spike@one-mk.test"]):
            private(["gdbus", "call", "--session", "-d", "org.gnome.Shell.Extensions", "-o", "/org/gnome/Shell/Extensions",
                     "-m", "org.gnome.Shell.Extensions.EnableExtension", extension])
        evaluate('Main.overview.hide(); "hidden"')
        if args.normal_window:
            evaluate(f'{BRIDGE}._setType = false')
        arguments = ["/usr/bin/env", *[f"{name}={value}" for name, value in media_environment.items()
                      if name.startswith(("GST_", "ONE_MK_WALL_")) or name == 'WAYLAND_DEBUG'], f"ONE_MK_WALL_RISK_HASH={fixture_hash}",
                     str(args.binary), "--present=gnome", "--risk-probe"]
        if args.matrix:
            report['measurements'] = runpy.run_path(str(HERE / 'm1a-matrix.py'))['run_matrix'](
                evaluate, private, snapshot, arguments, ROOT, LAB, report)
            report['status'] = 'partial-risk-pass-not-M1-acceptance'
            failures = report['measurements'].get('qualification_failures', [])
            if failures:
                raise RuntimeError('STOP: ' + '; '.join(failures))
        else:
            before = time.monotonic()
            report["renderer_pid"] = evaluate(f"{BRIDGE}.startRenderer({json.dumps(arguments)})")
            while time.monotonic() - before < 5:
                current = snapshot()
                if current["damage"]:
                    break
                time.sleep(0.05)
            report["measurements"]["first_damage_observed_ms"] = round((time.monotonic() - before) * 1000, 3)
            report["measurements"]["initial"] = current
            if not current["damage"] or current["buffer"] != [0, 0, 3840, 1080]:
                raise RuntimeError("STOP: Rust/GStreamer initial surface failed first-frame/extent presentation gate")
            report["stages"].append("initial first frame and two-monitor extent passed")
            initial_damage = current["damage"]
            time.sleep(2)
            report["measurements"]["active"] = snapshot()
            if report["measurements"]["active"]["damage"] <= initial_damage:
                raise RuntimeError("STOP: hidden Rust/GStreamer window did not continue producing damage")
            private(["python3", str(HERE / "client/displayconfig.py"), "apply", "1"])
            time.sleep(1)
            report["measurements"]["unplug"] = snapshot()
            private(["python3", str(HERE / "client/displayconfig.py"), "apply", "2"])
            time.sleep(2)
            report["measurements"]["replug"] = snapshot()
            if report["measurements"]["replug"]["buffer"] != [0, 0, 3840, 1080]:
                raise RuntimeError("STOP: hot-plug did not restore the required two-monitor Rust/GStreamer surface within 2 seconds")
            report["stages"].append("hot-plug extent restored; visible gap NOT measured")
            report["status"] = "partial-risk-pass-not-M1-acceptance"
    except Exception as error:
        report["failure"] = str(error)
        report['status'] = 'failed'
    finally:
        if started:
            log = (ROOT / 'log/shell.log').read_text(errors='replace') if (ROOT / 'log/shell.log').exists() else ''
            try:
                log = cleanup()
                report["cleanup"] = "all owned children reaped; session directory and generated media removed"
            except Exception as error:
                report["cleanup"] = str(error)
                report["cleanup_traceback"] = traceback.format_exc()
                report["status"] = "failed"
            if log:
                (LAB / f"{args.label}-shell.log").write_text(log)
                report["shell_log_tail"] = log.splitlines()[-45:]
                violations = [line for line in log.splitlines() if "Buggy client (one-mk-wall)" in line]
                report['shell_warnings'] = shell_warnings(log)
                report['new_warnings'] = (new_warnings(log, args.baseline_log.read_text())
                                          if args.baseline_log else report['shell_warnings'])
                if report['new_warnings'] and not args.baseline:
                    report['status'] = 'failed'
                    report['warning_gate'] = 'FAILED: warnings absent from recorded baseline'
                if violations:
                    report["presentation_gate"] = "FAILED: compositor required a protocol workaround for the Rust client"
                    report["protocol_violations"] = violations
                    report["status"] = "failed"
        (LAB / f'{args.label}.json').write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 1 if report["status"] == "failed" else 0


if __name__ == "__main__":
    raise SystemExit(main())
