#!/usr/bin/env python3
"""Linux daemon wake-up regression; see docs/IDLE-WAKEUPS.md for budgets."""

import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time


def snapshot(process):
    if process.poll() is not None:
        raise RuntimeError(f"daemon exited with {process.returncode}")
    threads = {}
    for task in Path(f"/proc/{process.pid}/task").iterdir():
        try:
            status = dict(line.split(":", 1) for line in (task / "status").read_text().splitlines() if ":" in line)
            threads[task.name] = {
                "name": (task / "comm").read_text().strip(),
                "voluntary": int(status["voluntary_ctxt_switches"]),
                "involuntary": int(status["nonvoluntary_ctxt_switches"]),
                "stat": (task / "stat").read_text().strip(),
            }
        except FileNotFoundError:
            continue
    return threads


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reserved:
        reserved.bind(("127.0.0.1", 0))
        return reserved.getsockname()[1]


class Lab:
    def __init__(self, directory, binary):
        self.directory = directory
        self.binary = binary
        self.display = None
        self.cleanup = contextlib.ExitStack()

    def home(self, name):
        directory = self.directory / name
        directory.mkdir(mode=0o700, exist_ok=True)
        return directory

    def environment(self, home):
        environment = {
            "PATH": os.environ.get("PATH", os.defpath),
            "HOME": str(home),
            "XDG_RUNTIME_DIR": str(home),
            "ONE_MK_HOME": str(home),
        }
        if self.display:
            environment["DISPLAY"] = self.display
        return environment

    def spawn(self, arguments, home, **options):
        process = subprocess.Popen(arguments, env=self.environment(home), **options)
        self.cleanup.callback(self.stop, process)
        return process

    @staticmethod
    def stop(process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

    def xvfb(self):
        executable = shutil.which("Xvfb")
        if executable is None:
            print("SKIP x11: Xvfb is not installed; null scenarios still run", flush=True)
            return False
        home = self.home("xvfb")
        log = self.cleanup.enter_context((home / "stderr").open("w"))
        process = self.spawn(
            [executable, "-displayfd", "1", "-screen", "0", "800x600x24", "-nolisten", "tcp"],
            home, stdout=subprocess.PIPE, stderr=log,
        )
        self.cleanup.callback(process.stdout.close)
        import select
        if not select.select([process.stdout], [], [], 10)[0]:
            raise RuntimeError("private Xvfb did not supply a display within 10 seconds")
        display = process.stdout.readline().decode().strip()
        if not display.isdigit():
            raise RuntimeError(f"private Xvfb failed: {(home / 'stderr').read_text()}")
        self.display = ":" + display
        return True

    def daemon(self, home, backend, port, discovery):
        log = self.cleanup.enter_context((home / "daemon.log").open("a"))
        arguments = [self.binary, "--home", str(home), "run", "--backend", backend, "--port", str(port)]
        if not discovery:
            arguments.append("--no-discovery")
        process = self.spawn(arguments, home, stdout=log, stderr=log)
        deadline = time.monotonic() + 10
        while not (home / "control.sock").exists():
            if process.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError(f"daemon startup failed: {(home / 'daemon.log').read_text()}")
            time.sleep(.02)
        return process

    def identity(self, home):
        process = self.daemon(home, "null", 0, False)
        self.stop(process)
        (home / "control.sock").unlink(missing_ok=True)
        return hashlib.sha256((home / "identity.cert.der").read_bytes()).hexdigest()

    def status(self, home):
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(5)
            stream.connect(str(home / "control.sock"))
            stream.sendall(b"status\n")
            response = bytearray()
            while chunk := stream.recv(65536):
                response.extend(chunk)
            return response.decode()

    def run(self, seconds, settle, discovery):
        backends = ["null", "x11"] if self.xvfb() else ["null"]
        cases = []
        for backend in backends:
            for state in ["alone", "offline", "connected"]:
                name = f"{backend}-{state}"
                primary, peer = self.home(name), self.home(name + "-peer")
                listen, peer_listen = free_port(), free_port()
                peer_id = None
                if state != "alone":
                    primary_id, peer_id = self.identity(primary), self.identity(peer)
                    for home, fingerprint, port in [(primary, peer_id, peer_listen), (peer, primary_id, listen)]:
                        (home / "devices.toml").write_text(
                            f'[[device]]\nname = "idle-peer"\nfingerprint = "{fingerprint}"\n'
                            f'side = "right"\naddrs = ["127.0.0.1:{port}"]\n'
                        )
                if state == "connected":
                    self.daemon(peer, backend, peer_listen, False)
                process = self.daemon(primary, backend, listen, discovery)
                cases.append((name, state, primary, peer_id, process))
        time.sleep(settle)
        for name, state, home, peer_id, process in cases:
            status = self.status(home)
            short_id = f"{peer_id[:4]}-{peer_id[4:8]}" if peer_id else None
            online = any(line.startswith(f"peer\t{short_id}\t") and line.split("\t")[4] == "1" for line in status.splitlines())
            if state == "connected" and not online:
                raise AssertionError(f"{name} is not connected: {status}")
            if state == "offline" and online:
                raise AssertionError(f"{name} unexpectedly connected: {status}")
            if process.poll() is not None:
                raise AssertionError(f"{name} exited")
        before = {name: snapshot(process) for name, _, _, _, process in cases}
        began = time.monotonic()
        time.sleep(seconds)
        elapsed = time.monotonic() - began
        result = {"seconds": elapsed, "settle_seconds": settle, "discovery": discovery, "cases": {}}
        failures = []
        for name, state, home, peer_id, process in cases:
            after = snapshot(process)
            if state == "connected":
                short_id = f"{peer_id[:4]}-{peer_id[4:8]}"
                status = self.status(home)
                if not any(line.startswith(f"peer\t{short_id}\t") and line.split("\t")[4] == "1" for line in status.splitlines()):
                    failures.append(f"{name}: peer disconnected during measurement: {status}")
            threads = {}
            for tid, entry in after.items():
                prior = before[name].get(tid)
                if prior is None:
                    failures.append(f"{name}: new thread {tid} appeared during idle")
                    prior = {"voluntary": 0, "involuntary": 0, "stat": ""}
                voluntary = entry["voluntary"] - prior["voluntary"]
                involuntary = entry["involuntary"] - prior["involuntary"]
                threads[tid] = dict(entry, voluntary=voluntary, involuntary=involuntary,
                                   voluntary_per_second=voluntary / elapsed, stat_before=prior["stat"])
                if not discovery:
                    budget = (6 if state == "alone" or tid != str(process.pid) else {"offline": 64, "connected": 128}[state]) * seconds / 20
                    if voluntary > budget:
                        failures.append(f"{name}/{entry['name']} ({tid}): {voluntary} voluntary switches in {elapsed:.2f}s exceeds {budget:.0f}")
            disappeared = before[name].keys() - after.keys()
            if disappeared:
                failures.append(f"{name}: threads disappeared during idle: {sorted(disappeared)}")
            result["cases"][name] = threads
            summary = ", ".join(f"{entry['name']}[{tid}]={entry['voluntary_per_second']:.3f}/s voluntary ({entry['involuntary']} involuntary)" for tid, entry in threads.items())
            print(f"{name}: {summary}", flush=True)
        return result, failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=lambda value: str(Path(value).resolve()))
    parser.add_argument("--seconds", type=float, default=20)
    parser.add_argument("--settle", type=float, default=20)
    parser.add_argument("--discovery", action="store_true", help="measure LAN discovery without asserting traffic-dependent budgets")
    parser.add_argument("--report", type=Path)
    options = parser.parse_args()
    if options.seconds < 20 or options.settle < 20:
        parser.error("settle and sample windows must each be at least 20 seconds")
    if not Path("/proc/self/task").exists():
        print("SKIP idle wakeups: Linux /proc task counters are required")
        return
    with tempfile.TemporaryDirectory(prefix="one-mk-idle-") as directory:
        lab = Lab(Path(directory), options.binary)
        try:
            result, failures = lab.run(options.seconds, options.settle, options.discovery)
            if options.report:
                options.report.write_text(json.dumps(result, indent=2) + "\n")
            if failures:
                raise AssertionError("\n".join(failures))
        finally:
            lab.cleanup.close()


if __name__ == "__main__":
    main()
