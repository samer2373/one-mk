#!/usr/bin/env python3
"""Offline extension consent/setup, only in a disposable headless GNOME session.

Uses the normal release CLI, not laboratory authorization or Private Eval.
No renderer is authorized and no wallpaper setting is changed. The harness's
restart models logging out and back in; it is not a real GDM login test.
"""
import json
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
HARNESS = HERE / "isolated-session.sh"
UUID = "one-mk-wallpaper@one-mk.org"


def main():
    binary = Path(os.environ.get("SETUP_BINARY", REPO / "target/debug/one-mk")).resolve()
    output = REPO / "target/gnome-setup-qa"
    output.mkdir(parents=True, exist_ok=True)
    report = {"checks": [], "production_playback_qualified": False}
    root = Path(tempfile.mkdtemp(prefix="omk-setup-"))
    session = root / "s"
    sources = root / "extensions"
    sources.mkdir()
    shutil.copytree(HERE / "ext/spike-harness@one-mk.test", sources / "spike-harness@one-mk.test")
    environment = {"PATH": "/usr/bin:/bin", "HOME": str(root), "SPIKE": str(session),
                   "USER": pwd.getpwuid(os.getuid()).pw_name, "LANG": "C.UTF-8", "EXT_SRC": str(sources),
                   "FAKE_GDM": "1", "PYTHONDONTWRITEBYTECODE": "1"}

    def run(arguments, check=True):
        return subprocess.run([str(value) for value in arguments], env=environment, cwd=REPO,
                              capture_output=True, text=True, timeout=60, check=check)

    def private(arguments, check=True):
        return run([HARNESS, "run", *arguments], check)

    def cli(action, check=True):
        return private([binary, "wallpaper", "--json", "extension", action], check)

    def status():
        return json.loads(cli("status").stdout)

    def background():
        return private(["gsettings", "get", "org.gnome.desktop.background", "picture-uri"]).stdout

    try:
        run([HARNESS, "start", "2"])
        report["shell"] = run(["gnome-shell", "--version"]).stdout.strip()
        before = background()
        state = status()
        assert state["setup"]["stage"] == "not_installed", state
        assert state["setup"]["can_install"] and not state["enabled"], state
        assert cli("enable", check=False).returncode != 0
        report["checks"].append("no install or enable on status; enable before install refused")
        installed = json.loads(cli("install").stdout)
        assert "Log out and back in once to finish" in installed["message"], installed
        state = status()
        assert state["installed"] and not state["enabled"], state
        assert state["setup"]["stage"] == "log_out_required", state
        assert cli("enable", check=False).returncode != 0
        assert cli("install", check=False).returncode != 0
        assert background() == before
        report["checks"].append("bundled install offline; no auto-enable; logout required; duplicate install refused")
        run([HARNESS, "restart", "2"])
        state = status()
        assert state["setup"]["stage"] == "disabled" and state["setup"]["can_enable"], state
        cli("enable")
        deadline = time.monotonic() + 5
        while True:
            state = status()
            if state["setup"]["stage"] == "active":
                break
            assert time.monotonic() < deadline, state
            time.sleep(0.05)
        assert state["enabled"], state
        assert "not connected to the production service" in state["setup"]["message"]
        assert background() == before
        assert cli("uninstall", check=False).returncode != 0
        report["checks"].append("next private session discovers extension; explicit Enable becomes active; active uninstall refused")
        private(["gdbus", "call", "--session", "-d", "org.gnome.Shell.Extensions", "-o", "/org/gnome/Shell/Extensions",
                 "-m", "org.gnome.Shell.Extensions.DisableExtension", UUID])
        cli("uninstall")
        assert not status()["installed"]
        assert background() == before
        report["checks"].append("disable then managed uninstall; private wallpaper URI unchanged throughout")
        report["result"] = "passed"
    except Exception as error:
        report["result"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        log = session / "log/shell.log"
        if log.exists():
            shutil.copy2(log, output / "shell.log")
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        try:
            run([HARNESS, "stop"], check=False)
        finally:
            shutil.rmtree(root)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
