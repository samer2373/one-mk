#!/usr/bin/env python3
"""Small deliberate behavioral mutations, applied/reverted using apply_patch.
Run only in this spike checkout with no other writer; never formats source.
"""
import difflib
from pathlib import Path
import subprocess

SOURCE = Path("crates/wall/src/lib.rs")
original = SOURCE.read_text()


def patch(before, after):
    changes = list(difflib.unified_diff(before.splitlines(True), after.splitlines(True), n=3))[2:]
    changes = ["@@\n" if line.startswith("@@") else line for line in changes]
    subprocess.run(["apply_patch", "*** Begin Patch\n*** Update File: " + str(SOURCE) + "\n" + "".join(changes) + "*** End Patch\n"], check=True, stdout=subprocess.DEVNULL)


mutations = [
    ("offset sign", "let offset = ((self.t2 - self.t1) + (self.t3 - self.t4)) * 0.5;", "let offset = -((self.t2 - self.t1) + (self.t3 - self.t4)) * 0.5;"),
    ("highest RTT instead of lowest", "left.0.2.total_cmp(&right.0.2)", "right.0.2.total_cmp(&left.0.2)"),
    ("rate correction sign", "1.0 + (error / 4.0)", "1.0 - (error / 4.0)"),
    ("remove hysteresis", "(!self.correcting && error.abs() <= 0.008)", "(error.abs() <= 0.008)"),
    ("drift sign", "(offset - anchor.offset) / (local - anchor.local)", "(anchor.offset - offset) / (local - anchor.local)"),
]
subprocess.run(["cargo", "test", "-j", "2", "-p", "one-mk-wall-sync"], check=True)
for name, old, new in mutations:
    assert original.count(old) == 1, name
    mutated = original.replace(old, new)
    patch(original, mutated)
    try:
        result = subprocess.run(["cargo", "test", "-j", "2", "-p", "one-mk-wall-sync", "--test", "core"], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(f"=== {name}: exit {result.returncode} ===", flush=True)
        print(result.stdout, flush=True)
        assert result.returncode != 0, "SURVIVED: " + name
        assert "test result: FAILED" in result.stdout, "compile failure is not a killed mutation"
    finally:
        patch(mutated, original)
assert SOURCE.read_text() == original
subprocess.run(["cargo", "test", "-j", "2", "-p", "one-mk-wall-sync"], check=True)
