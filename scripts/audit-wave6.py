#!/usr/bin/env python3
"""Disposable loopback CLI audit; run inside a private network namespace."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time


class Audit:
    def __init__(self, binary, root):
        self.binary = str(binary.resolve())
        self.root = root
        self.children = []
        self.records = []

    def home(self, name):
        home = self.root / name
        home.mkdir(mode=0o700)
        return home

    def env(self, home):
        return {"PATH": os.environ.get("PATH", os.defpath), "HOME": str(home),
                "ONE_MK_HOME": str(home), "XDG_RUNTIME_DIR": str(home)}

    def cli(self, home, *args, success=True, input=None):
        result = subprocess.run([self.binary, *args], env=self.env(home), input=input,
                                capture_output=True, text=True, timeout=25)
        self.records.append({"args": list(args), "exit": result.returncode})
        assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
        assert "panicked at" not in result.stderr, (args, result.stderr)
        return result.stdout

    def spawn(self, home, args, name):
        log = home / name
        with log.open("w") as output:
            process = subprocess.Popen([self.binary, *args], env=self.env(home),
                                       stdin=subprocess.PIPE, stdout=output, stderr=output, text=True)
        self.children.append(process)
        return process, log

    def wait(self, condition, label):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            value = condition()
            if value:
                return value
            time.sleep(0.025)
        raise AssertionError(label)

    def request(self, home, command="status"):
        try:
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(1)
                connection.connect(str(home / "control.sock"))
                connection.sendall((command + "\n").encode())
                chunks = []
                while chunk := connection.recv(65536):
                    chunks.append(chunk)
                return b"".join(chunks).decode()
        except (OSError, TimeoutError):
            return ""

    def daemon(self, home):
        process, log = self.spawn(home, ["run", "--backend", "null", "--port", "0",
                                        "--no-discovery", "--no-notify", "--no-clipboard",
                                        "--downloads", str(home / "downloads")], "daemon.log")
        self.wait(lambda: self.request(home).startswith("ok\n"), log.read_text)
        return process

    def help_tree(self, home, args=()):
        output = self.cli(home, *args, "--help")
        assert "Usage:" in output
        section = re.search(r"Commands:\n(.*?)(?:\n\n|\Z)", output, re.S)
        if section:
            for command in re.findall(r"^  ([a-z][a-z0-9-]*)\s", section.group(1), re.M):
                if command != "help":
                    self.help_tree(home, (*args, command))

    def pair(self, host, joiner):
        first, first_log = self.spawn(host, ["pair", "--machine", "--no-discovery"], "pair.log")
        header = self.wait(lambda: re.search(r"^me\t[^\n]+", first_log.read_text(), re.M), "pair host")
        port = header.group().split("\t")[3]
        second, second_log = self.spawn(joiner, ["pair", "--machine", "--no-discovery", "--connect",
                                                "127.0.0.1:" + port], "pair.log")
        codes = []
        for process, log in [(first, first_log), (second, second_log)]:
            match = self.wait(lambda: re.search(r"^code\t([^\n]+)", log.read_text(), re.M), "pair code")
            codes.append(match.group(1).split("\t")[-1])
        assert codes[0] == codes[1], "pairing codes disagree"
        for process in [first, second]:
            process.stdin.write("yes\n")
            process.stdin.flush()
        for process in [first, second]:
            assert process.wait(timeout=20) == 0
        for home in [host, joiner]:
            self.wait(lambda: re.search(r"^peer\t[^\n]+\t1\t", self.request(home), re.M), "peer connected")

    def exercise(self):
        first, second = self.home("a"), self.home("b")
        self.help_tree(first)
        for home in [first, second]:
            self.daemon(home)
        self.pair(first, second)
        getters = ["id", "devices", "status", "drag", "clipboard", "notify", "typing",
                   "broadcast", "edge", "gaming", "hotkeys", "lock-sync", "hooks", "power", "keymap",
                   "pointer", "pointer-overlay", "scroll", "autoscroll", "gestures", "buttons", "input-devices"]
        for command in getters:
            self.cli(first, command)
        for args in [("lock-pointer", "on"), ("lock-pointer", "off"), ("broadcast", "on"),
                     ("broadcast", "off"), ("switch", "next"), ("switch", "here"), ("takeback",),
                     ("edge", "--wrap", "all"), ("edge", "--wrap", "off"), ("gaming", "on"),
                     ("gaming", "off"), ("drag", "--on"), ("drag", "--off"),
                     ("clipboard", "--allow-fetch", "on"), ("clipboard", "--allow-fetch", "off"),
                     ("clipboard", "history", "on"), ("clipboard", "history", "off"),
                     ("profile", "save", "audit"), ("profile", "list"), ("profile", "use", "audit"),
                     ("profile", "auto", "audit", "--ssid", "private-audit"),
                     ("profile", "auto", "audit", "--off"), ("profile", "delete", "audit"),
                     ("autoscroll", "--on"), ("autoscroll", "--off"),
                     ("keymap", "add", "capslock", "escape", "--app", "editor"),
                     ("keymap", "check"), ("keymap", "remove", "capslock", "--app", "editor"),
                     ("keymap", "add", "capslock", "escape", "--device", "usb/1234:5678//Audit keyboard"),
                     ("keymap", "remove", "capslock", "--device", "usb/1234:5678//Audit keyboard"),
                     ("keymap", "local", "off"), ("keymap", "clear"),
                     ("pointer", "--speed", "1.5", "--os", "linux"), ("pointer", "--reset", "--os", "linux"),
                     ("scroll", "--speed", "1.25", "--app", "editor"), ("scroll", "--reset", "--app", "editor"),
                     ("buttons", "map", "button4", "escape"), ("buttons", "unmap", "button4"),
                     ("buttons", "preset", "side-back-forward"), ("buttons", "clear"),
                     ("gestures", "add", "button4", "Left", "ctrl+alt+left"),
                     ("gestures", "remove", "button4", "Left"), ("gestures", "preset", "side-swipe"),
                     ("gestures", "clear"), ("hotkeys", "--off"), ("hotkeys", "--on"),
                     ("lock-sync", "on"), ("lock-sync", "off"), ("lock-all",),
                     ("power", "--block-screensaver", "on"), ("power", "--block-screensaver", "off"),
                     ("power", "--sleep-sync", "on"), ("power", "--sleep-sync", "off"),
                     ("pointer-overlay", "never"), ("pointer-overlay", "auto"),
                     ("typing", "--keys", "physical"), ("install",), ("uninstall",),
                     ("integrate",), ("integrate", "--remove"), ("notify", "off")]:
            self.cli(first, *args)
        marker = first / "leave-hook-ran"
        self.cli(first, "hooks", "--on-leave", "touch " + str(marker))
        self.cli(first, "switch", "next")
        self.wait(marker.exists, "real pointer-leave hook")
        self.cli(first, "switch", "here")
        self.cli(first, "hooks", "--off")
        peer = self.cli(second, "id").split("fingerprint ")[1].strip()
        self.cli(first, "layout", peer, "right", "--offset", "12,20")
        self.wait(lambda: "at -12,-20" in self.cli(second, "devices"), "mirrored layout offset")
        self.cli(first, "layout", peer, "right", "--offset", "none")
        self.wait(lambda: "at -12,-20" not in self.cli(second, "devices"), "cleared peer layout offset")
        for args in [("clip-get",), ("clip-send",), ("clipboard", "history", "list"),
                     ("switch", "missing-peer"), ("forget", "missing-peer"),
                     ("profile", "use", "missing"), ("layout", "missing-peer", "left"),
                     ("keymap", "add", "not-a-key", "a"), ("drag", "--zone", "0"),
                     ("clipboard", "--max-mb", "4294967295"), ("autoscroll", "--speed", "NaN"),
                     ("pointer", "--speed", "1", "--device", "nonexistent-mouse"),
                     ("gestures", "add", "left", "up", "escape"),
                     ("autoscroll", "--device", "nonexistent-mouse", "--on"),
                     ("join", "--yes", "invalid-invitation"), ("update", "--check")]:
            self.cli(first, *args, success=False)
        doctor = json.loads(self.cli(first, "doctor", "--json"))
        assert doctor
        payload = first / "payload.txt"
        payload.write_bytes(b"wave6 audit\n" * 8192)
        self.cli(first, "send", str(payload))
        received = self.wait(lambda: list((second / "downloads").rglob("payload.txt")), "received file")
        assert hashlib.sha256(received[0].read_bytes()).digest() == hashlib.sha256(payload.read_bytes()).digest()
        self.cli(first, "bench", "--count", "100", "--rate", "1000")
        self.cli(first, "stop")
        self.cli(second, "stop")
        for args in [("switch", "next"), ("takeback",), ("broadcast", "on"), ("clip-get",),
                     ("lock-pointer", "on"), ("lock-all",)]:
            self.cli(first, *args, success=False)
        self.cli(first, "forget", self.cli(second, "id").split("fingerprint ")[1].strip())
        assert "No paired computers" in self.cli(first, "devices")

    def close(self):
        for process in reversed(self.children):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=4)
            if process.stdin:
                process.stdin.close()


def main():
    assert os.environ.get("ONE_MK_ISOLATED_AUDIT") == "1", "use scripts/audit-wave6-isolated.sh"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    options = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="w6-") as directory:
        audit = Audit(options.binary, Path(directory))
        try:
            audit.exercise()
        finally:
            audit.close()
            options.report.write_text(json.dumps(audit.records, indent=2) + "\n")
    print(f"PASS {len(audit.records)} CLI invocations; matching-code pairing, two null daemons, file hash verified")


if __name__ == "__main__":
    main()
