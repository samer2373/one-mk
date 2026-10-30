#!/usr/bin/env python3
import runpy
from pathlib import Path
import unittest


probe = runpy.run_path(str(Path(__file__).with_name('m1-risk-probe.py')))
warnings = probe['shell_warnings']


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

    def test_only_recorded_warning_identities_match(self):
        baseline = '(gnome-shell:123): Gjs-WARNING **: 12:34:56.123: environment'
        current = '(gnome-shell:456): Gjs-WARNING **: 12:35:01.789: environment'
        ours = '(gnome-shell:456): Gjs-WARNING **: 12:35:01.789: renderer failed'
        self.assertEqual(probe['new_warnings'](current + '\n' + ours, baseline), [ours])
        self.assertEqual(probe['new_warnings'](ours, ''), [ours])

    def test_matching_generic_header_does_not_hide_new_stack(self):
        header = 'Gjs-WARNING: Unhandled promise rejection. Stack trace:'
        baseline = header + '\n  loginManager@resource:///login.js:1:2\n'
        ours = header + '\n  renderer@file:///extension.js:1:2\n'
        self.assertEqual(probe['new_warnings'](ours, baseline), [ours.rstrip()])


if __name__ == '__main__':
    unittest.main()
