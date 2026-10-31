#!/usr/bin/env python3
"""Require optimized builds to refuse the presentation laboratory features."""
import os
from pathlib import Path
import subprocess


def main():
    root = Path(__file__).resolve().parent.parent
    for package, feature in (("one-mk", "wallpaper-m1-lab"), ("one-mk-wall", "m1-lab")):
        result = subprocess.run(
            ["cargo", "check", "--locked", "--release", "-j", "2",
             "-p", package, "--features", feature],
            cwd=root, env=os.environ.copy(), capture_output=True, text=True, timeout=600,
        )
        assert result.returncode != 0, f"{package}: laboratory feature compiled for release"
        assert "presentation laboratory features require debug assertions" in result.stderr, result.stderr
        print(f"PASS: {package}/{feature} refused in release")


if __name__ == "__main__":
    main()
