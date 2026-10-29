import base64
import os
import pathlib
import pty
import re
import select
import signal
import socket
import subprocess
import tempfile
import termios
import time


ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/one-mk"
PROBE = ROOT / "target/debug/examples/invite_probe"
CHILDREN = []


class Terminal:
    def __init__(self, home, arguments):
        self.master, slave = pty.openpty()
        settings = termios.tcgetattr(slave)
        settings[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, settings)
        self.process = subprocess.Popen(
            [str(BINARY), "--home", str(home), *arguments],
            stdin=slave, stdout=slave, stderr=slave,
        )
        os.close(slave)
        self.output = ""
        CHILDREN.append(self)

    def expect(self, pattern, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            match = re.search(pattern, self.output)
            if match:
                return match
            if select.select([self.master], [], [], 0.1)[0]:
                try:
                    chunk = os.read(self.master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                self.output += chunk.decode(errors="replace")
        redacted = re.sub(r"onemk://pair\S+", "<redacted-link>", self.output)
        raise AssertionError(f"missing {pattern!r}: {redacted[-2500:]}")

    def send(self, text):
        os.write(self.master, text.encode())

    def finish(self, success=True):
        status = self.process.wait(timeout=15)
        assert (status == 0) == success, (status, self.output[-1000:])

    def stop(self):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        os.close(self.master)


def invitation(home, address, extra=()):
    terminal = Terminal(home, ["invite", "--no-discovery", "--listen", f"{address}:0", *extra])
    link = terminal.expect(r"(onemk://pair\S+)").group(1)
    terminal.expect("Approval required|WARNING: --no-confirm")
    return terminal, link


def metadata(link):
    encoded = link.split("&d=", 1)[1]
    payload = base64.urlsafe_b64decode(encoded + "=" * (-len(encoded) % 4))
    fingerprint = payload[1:33].hex()
    expiry = int.from_bytes(payload[33:41], "big")
    offset = 45 + int.from_bytes(payload[43:45], "big")
    offset += 1
    size = int.from_bytes(payload[offset:offset + 2], "big")
    address = payload[offset + 2:offset + 2 + size].decode()
    return address, fingerprint, str(expiry)


def control(home):
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(2)
        stream.connect(str(home / "invitation/control.sock"))
        stream.sendall(b"invitation\n")
        response = bytearray()
        while chunk := stream.recv(4096):
            response.extend(chunk)
        return response.decode()


def join_terminal(home, link):
    terminal = Terminal(home, ["join"])
    terminal.expect("Paste the invitation link")
    terminal.send(link + "\n")
    code = terminal.expect(r"Your code: (\d{6})").group(1)
    terminal.expect(r"Trust .*\? \[y/N\]")
    return terminal, code


def devices(home):
    result = subprocess.run([str(BINARY), "--home", str(home), "-v", "devices"], capture_output=True, text=True, check=True, timeout=5)
    assert "No paired computers" not in result.stdout
    return result.stdout.strip()


def main():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as route:
        route.connect(("192.0.2.1", 9))
        address = route.getsockname()[0]
    assert not address.startswith("127.")
    print(f"Same-host unicast endpoint: {address} (loopback link literals intentionally rejected)", flush=True)
    with tempfile.TemporaryDirectory(prefix="one-mk-invite-security-") as directory:
        homes = [pathlib.Path(directory) / name for name in ["A", "B", "C", "script-A", "script-B", "refuse-A", "refuse-B", "expire-A", "expire-B"]]
        host, link = invitation(homes[0], address)
        assert link in control(homes[0])
        attack = subprocess.run([str(PROBE), *metadata(link)], capture_output=True, text=True, timeout=90, check=True)
        print(attack.stdout.strip(), flush=True)
        assert link in control(homes[0])
        time.sleep(1.05)
        joiner, code = join_terminal(homes[1], link)
        shown = host.expect(r"Does the other computer show (\d{6})\?").group(1)
        assert shown == code
        assert control(homes[0]) == "none\n"
        assert not (homes[0] / "devices.toml").exists()
        assert not (homes[1] / "devices.toml").exists()
        late = subprocess.run([str(BINARY), "--home", str(homes[2]), "join", "--yes"], input=link + "\n", capture_output=True, text=True, timeout=12)
        assert late.returncode != 0
        assert "already used or expired" in late.stderr, late.stderr
        print("Late join: explicit 'invitation already used or expired'; active UI link cleared.", flush=True)
        print("B: " + re.search(r"Your code: [^\r\n]+", joiner.output).group(0), flush=True)
        print("A: " + re.search(r"Does the other computer show [^\r\n]+", host.output).group(0) + " -> y", flush=True)
        print("B: " + re.search(r"Trust [^\r\n]+", joiner.output).group(0) + " -> y", flush=True)
        joiner.send("y\n")
        host.send("y\n")
        host.expect("Trusted ")
        joiner.expect("Trusted ")
        host.finish()
        joiner.finish()
        print("A devices:\n" + devices(homes[0]), flush=True)
        print("B devices:\n" + devices(homes[1]), flush=True)
        host, link = invitation(homes[3], address, ["--no-confirm"])
        scripted = subprocess.run([str(BINARY), "--home", str(homes[4]), "join", "--yes"], input=link + "\n", capture_output=True, text=True, timeout=15, check=True)
        host.expect("Trusted ")
        host.finish()
        assert "WARNING: --yes" in scripted.stdout
        assert "WARNING: --no-confirm skips comparing code" in host.output
        devices(homes[3])
        devices(homes[4])
        print("Scripted --yes / --no-confirm: both warnings printed; both devices saved.", flush=True)
        host, link = invitation(homes[5], address)
        joiner, code = join_terminal(homes[6], link)
        host.expect(r"Does the other computer show " + code)
        host.send("y\n")
        joiner.send("n\n")
        host.finish(success=False)
        joiner.finish(success=False)
        assert not (homes[5] / "devices.toml").exists()
        assert not (homes[6] / "devices.toml").exists()
        print("Joiner refusal: neither trust store created.", flush=True)
        host, link = invitation(homes[7], address, ["--expires", "2s"])
        joiner, code = join_terminal(homes[8], link)
        host.expect(r"Does the other computer show " + code)
        joiner.send("y\n")
        host.finish(success=False)
        joiner.finish(success=False)
        assert not (homes[7] / "devices.toml").exists()
        assert not (homes[8] / "devices.toml").exists()
        print("Expiry during approval: both commands fail; neither trust store created.", flush=True)
    print("All smoke checks passed; all started processes exited or are stopped in cleanup.", flush=True)


if __name__ == "__main__":
    try:
        main()
    finally:
        for child in reversed(CHILDREN):
            child.stop()
