#!/usr/bin/env python3
import ast
from pathlib import Path
import statistics
import tempfile
from types import SimpleNamespace
import unittest


class PlaybackMeasurement(unittest.TestCase):
    def measure(self, baseline):
        path = Path(__file__).with_name('qualify-m1-playback.py')
        functions = [node for node in ast.parse(path.read_text()).body
                     if isinstance(node, ast.FunctionDef) and node.name == 'measure']
        clock = [0.0]

        def sleep(seconds):
            clock[0] += seconds

        current = {'presented': 961, 'late': 0, 'starved': 0, 'seam_us': 33000}
        with tempfile.TemporaryDirectory() as directory:
            group = Path(directory)
            for name, value in {'cpu.stat': 'usage_usec 100\nnr_throttled 0\n',
                                'cpu.max': '200000 100000', 'memory.current': '1024',
                                'memory.peak': '1024'}.items():
                (group / name).write_text(value)
            scope = {'time': SimpleNamespace(sleep=sleep, monotonic=lambda: clock[0]),
                     'os': SimpleNamespace(sysconf=lambda _: 100), 'statistics': statistics,
                     'group_dir': lambda: group, 'shell_pid': lambda: 2,
                     'cpu_ticks': lambda _: 0, 'status_kib': lambda *_: 1,
                     'worker_pids': lambda: [3], 'MEASURE': 1,
                     'seams': lambda *_: baseline if clock[0] <= 3 else baseline + [current]}
            exec(compile(ast.Module(body=functions, type_ignores=[]), str(path), 'exec'), scope)
            return scope['measure'](1, 0)['summary']

    def test_missing_baseline_is_not_a_zero_window_counter(self):
        summary = self.measure([])
        self.assertNotIn('presented_in_window', summary)
        self.assertIsNone(summary['presented_between_seam_samples'])
        self.assertIsNone(summary['late_between_seam_samples'])
        self.assertEqual(summary['presented_total'], 961)

    def test_existing_seam_baseline_is_explicitly_sampled(self):
        summary = self.measure([{'presented': 100, 'late': 0, 'starved': 0}])
        self.assertEqual(summary['presented_between_seam_samples'], 861)


if __name__ == '__main__':
    unittest.main()
