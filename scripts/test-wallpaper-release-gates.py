#!/usr/bin/env python3
"""Require optimized builds to refuse the presentation laboratory features."""
import os
from pathlib import Path
import subprocess


def main():
    root = Path(__file__).resolve().parent.parent
    # A cold release-profile check compiles every dependency; on a loaded computer that took longer than the
    # old fixed 600 s, which failed the run without saying why. ONE_MK_GATE_TIMEOUT (seconds) overrides this.
    limit = int(os.environ.get("ONE_MK_GATE_TIMEOUT", "2400"))
    for package, feature in (("one-mk", "wallpaper-m1-lab"), ("one-mk-wall", "m1-lab")):
        try:
            result = subprocess.run(
                ["cargo", "check", "--locked", "--release", "-j", "2",
                 "-p", package, "--features", feature],
                cwd=root, env=os.environ.copy(), capture_output=True, text=True, timeout=limit,
            )
        except subprocess.TimeoutExpired:
            raise SystemExit(f"{package}/{feature}: cargo check did not finish in {limit} s (a loaded computer or a cold build); "
                             "run it again or raise ONE_MK_GATE_TIMEOUT")
        assert result.returncode != 0, f"{package}: laboratory feature compiled for release"
        assert "presentation laboratory features require debug assertions" in result.stderr, result.stderr
        print(f"PASS: {package}/{feature} refused in release")


if __name__ == "__main__":
    main()
