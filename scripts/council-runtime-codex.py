import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time


parser = argparse.ArgumentParser()
parser.add_argument("baseline", type=Path)
parser.add_argument("candidate", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
clock_ticks = os.sysconf("SC_CLK_TCK")
results = {"clock_ticks": clock_ticks, "build": "debug", "scenario": "visible static Desk, one offline peer, no running daemon", "samples": []}


def cpu_seconds(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / clock_ticks


def cleanup_tray(home):
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            command = (entry / "cmdline").read_bytes().split(b"\0")
            if command and Path(os.fsdecode(command[0])).name == "one-mk-tray" and os.fsencode(home) in command:
                os.kill(int(entry.name), signal.SIGTERM)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass


for label, binary in [("baseline", args.baseline), ("candidate", args.candidate)]:
    with tempfile.TemporaryDirectory(prefix=f"council-codex-{label}-", dir=args.candidate.resolve().parents[1]) as directory:
        home = Path(directory)
        status = home / "fixture.status"
        status.write_text("ok\nname\tCodex reference\nid\tabcd-ef01\nport\t24800\npeer\t1111-2222\tleft\t1\t0\tauto\tStudio PC\n")
        environment = dict(os.environ, ONE_MK_APP_NO_SETUP="1", ONE_MK_APP_STATUS=str(status), ONE_MK_APP_WHY="1", ONE_MK_APP_SIZE="1200x780")
        with (args.output / f"{label}-idle.log").open("w") as log:
            process = subprocess.Popen([str(binary.resolve()), "--home", str(home)], env=environment, stdout=log, stderr=log)
            try:
                time.sleep(12)
                started = time.monotonic()
                before = cpu_seconds(process.pid)
                time.sleep(60)
                elapsed = time.monotonic() - started
                consumed = cpu_seconds(process.pid) - before
                results["samples"].append({"name": label, "elapsed_s": round(elapsed, 3), "process_cpu_s": round(consumed, 3), "one_core_percent": round(consumed / elapsed * 100, 3)})
            finally:
                process.terminate()
                process.wait(timeout=10)
                cleanup_tray(home)
        for scenario, extra in [("moving", {}), ("static-blur", {"ONE_MK_APP_BENCH_HOVER": "1"})]:
            with (args.output / f"{label}-{scenario}.log").open("w") as log:
                subprocess.run([str(binary.resolve()), "--home", str(home)], env=dict(environment, ONE_MK_APP_BENCH="180", **extra), stdout=log, stderr=log, timeout=90, check=True)
            cleanup_tray(home)
        (args.output / "runtime.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(results["samples"][-1]), flush=True)
