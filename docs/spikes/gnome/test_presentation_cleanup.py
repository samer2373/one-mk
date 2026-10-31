#!/usr/bin/env python3
"""Exercise qualification cleanup without starting GNOME, D-Bus or a renderer."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch


class PresentationCleanupTests(unittest.TestCase):
    def test_failed_or_timed_out_start_still_stops_the_private_session(self):
        for error in (subprocess.CalledProcessError(1, "private-start"),
                      subprocess.TimeoutExpired("private-start", 45)):
            with self.subTest(error=type(error).__name__), tempfile.TemporaryDirectory() as temporary:
                lab = Path(temporary) / "lab"
                source = Path(__file__).with_name("qualify-m1-presentation.py")
                spec = importlib.util.spec_from_file_location("presentation_qualification", source)
                module = importlib.util.module_from_spec(spec)
                with patch.dict(os.environ, {"M1_BIN": temporary, "M1_LAB": str(lab)}):
                    spec.loader.exec_module(module)
                allocated = False
                stopped = False

                def run(arguments, **_options):
                    nonlocal allocated, stopped
                    if arguments[0] == module.HARNESS and arguments[1] == "start":
                        allocated = True
                        raise error
                    if arguments[0] == module.HARNESS and arguments[1] == "stop":
                        stopped = True
                        allocated = False
                    return types.SimpleNamespace(stdout="test shell", returncode=3)

                with patch.object(module, "run", run), patch.object(
                    module, "lab_processes", lambda: {123: "private-bus"} if allocated else {}
                ), contextlib.redirect_stdout(io.StringIO()):
                    with self.assertRaises(type(error)):
                        module.main()
                self.assertTrue(stopped, "failed start left its private process group running")
                self.assertFalse(allocated)


if __name__ == "__main__":
    unittest.main()
