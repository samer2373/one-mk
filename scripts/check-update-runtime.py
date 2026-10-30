#!/usr/bin/env python3
"""Exercise only the supplied debug binary and throwaway null-backend homes."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import tomllib

binary = Path(sys.argv[1]).resolve(strict=True)
processes = []


def command(home, *arguments, expected=0, executable=binary):
    result = subprocess.run([str(executable), "--home", str(home), *arguments],
                            env=environment(home), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
    print(f"$ one-mk {' '.join(arguments)} [exit {result.returncode}]\n{result.stdout}", flush=True)
    assert result.returncode == expected, result.stdout
    return result.stdout


def environment(home):
    runtime = home / "runtime"
    runtime.mkdir(mode=0o700, exist_ok=True)
    return dict(PATH=os.environ["PATH"], HOME=str(home), XDG_RUNTIME_DIR=str(runtime), ONE_MK_HOME=str(home))


def launch(home, port):
    log = open(home / "daemon.log", "a")
    process = subprocess.Popen([str(binary), "--home", str(home), "run", "--backend", "null",
                                "--no-discovery", "--no-notify", "--no-clipboard", "--port", str(port)],
                               env=environment(home), stdout=log, stderr=log)
    log.close()
    processes.append(process)
    for _ in range(100):
        assert process.poll() is None, (home / "daemon.log").read_text()
        if (home / "control.sock").exists():
            break
        time.sleep(0.05)
    return process


def sample(process):
    fields = Path(f"/proc/{process.pid}/stat").read_text().split()
    rss = next(line for line in Path(f"/proc/{process.pid}/status").read_text().splitlines() if line.startswith("VmRSS:"))
    return int(fields[13]) + int(fields[14]), rss


try:
    with tempfile.TemporaryDirectory(prefix="one-mk-portable-", dir=binary.parent) as portable_home:
        home = Path(portable_home)
        programs = home / ".local/bin"
        programs.mkdir(parents=True)
        portable = programs / "one-mk"
        os.link(binary, portable)
        previous = programs / "one-mk.old"
        previous.write_text("#!/bin/sh\necho 'one-mk 0.1.11'\n")
        previous.chmod(0o700)
        state = dict(sequence=42, digest="11" * 32, highest_version="0.1.13", last_attempt=100, available="0.1.13")
        (home / "update-state.json").write_text(json.dumps(state))
        command(home, "update", "--rollback", executable=portable)
        assert command(home, "--version", executable=portable).strip() == "one-mk 0.1.11"
        assert json.loads((home / "update-state.json").read_text()) == state
        assert not any(programs.glob(".one-mk-update-payload-*"))
        print("PASS: real CLI rollback only replaced its explicitly copied portable target; replay state retained.", flush=True)
    with tempfile.TemporaryDirectory(prefix="one-mk-update-runtime-") as temporary:
        homes = [Path(temporary) / name for name in ["on", "off"]]
        ports = []
        reservations = []
        for home in homes:
            home.mkdir()
            (home / "settings.toml").write_text('settings_version = 2\nnotify_files = false\n[clipboard]\ntext = false\n')
            (home / "devices.toml").write_text('[[device]]\nname = "previous-build fixture"\nfingerprint = "' + "11" * 32 + '"\nside = "left"\n')
            reservation = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            reservation.bind(("127.0.0.1", 0))
            ports.append(reservation.getsockname()[1])
            reservations.append(reservation)
        command(homes[0], "update", "--help")
        command(homes[0], "update", "--check", "--rollback", expected=2)
        for home, value in zip(homes, ["on", "off"]):
            original_devices = (home / "devices.toml").read_bytes()
            command(home, "update", "--check-on-start", value)
            settings = tomllib.loads((home / "settings.toml").read_text())
            assert settings["notify_files"] is False and settings["clipboard"]["text"] is False
            assert settings["update"]["check"] == (value == "on")
            assert (home / "devices.toml").read_bytes() == original_devices
            command(home, "update", "--check", expected=1)
            command(home, "update", expected=1)
            command(home, "update", "--rollback", expected=1)
        for reservation in reservations:
            reservation.close()
        for home, port in zip(homes, ports):
            launch(home, port)
            command(home, "status")
        before = [sample(process) for process in processes]
        time.sleep(5)
        for label, process, initial in zip(["ON", "OFF"], processes, before):
            after = sample(process)
            print(f"Idle {label}: {after[0] - initial[0]} CPU ticks / 5 seconds, {after[1]}; clock ticks/sec={os.sysconf('SC_CLK_TCK')}", flush=True)
        processes[0].send_signal(signal.SIGKILL)
        processes[0].wait(timeout=5)
        launch(homes[0], ports[0])
        command(homes[0], "status")
        for home in homes:
            command(home, "stop")
        for process in processes:
            process.wait(timeout=10)
        print("PASS: isolated CLI, legacy settings preservation, two null daemons, kill-9/restart, clean stop. No real release contacted.", flush=True)
finally:
    for process in processes:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
