#!/usr/bin/env python3
import runpy
from pathlib import Path
import unittest


warnings = runpy.run_path(str(Path(__file__).with_name('m1-risk-probe.py')))['shell_warnings']


class WarningGate(unittest.TestCase):
    def test_every_warning_is_fatal_including_unrelated_startup(self):
        lines = [
            'libmutter-WARNING: unknown experimental feature',
            'GNOME Shell-CRITICAL: no private logind proxy',
            'Gjs-WARNING: property type mismatch',
            'DING: JS ERROR: file monitor unavailable',
            'Clutter-WARNING: actor needs allocation',
            'libmutter-WARNING: Buggy client committed initial non-empty content',
        ]
        self.assertEqual(warnings('\n'.join(lines)), lines)

    def test_messages_are_not_warnings(self):
        self.assertEqual(warnings('libmutter-Message: virtual monitor added\nrenderer first frame'), [])


if __name__ == '__main__':
    unittest.main()
