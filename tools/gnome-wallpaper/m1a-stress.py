#!/usr/bin/env python3
"""Rapid isolated bridge generations with optional forced GJS collection."""
import json
from pathlib import Path
import time


BRIDGE = 'Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge'


def run_stress(evaluate, private, snapshot, arguments, report, cycles, collect):
    measurements = report['measurements']
    measurements['cycles'] = []
    for index in range(cycles):
        if collect:
            evaluate('Main.extensionManager.lookup("spike-harness@one-mk.test").stateObj.collectGarbage()')
        introspection = private(['gdbus', 'introspect', '--session', '-d',
            'org.onemk.WallpaperBridgeSpike1', '-o', '/org/onemk/WallpaperBridgeSpike1'])
        if 'Hello(' not in introspection:
            measurements['failed_introspection'] = introspection
            raise RuntimeError(f'cycle {index + 1}: Hello absent before spawn')
        lookup = evaluate(f'(() => {{ const info = {BRIDGE}._dbus.get_info(); return {{methods: info.methods.map(method => method.name), lookup: info.lookup_method("Hello")?.name ?? null}}; }})()')
        if lookup['lookup'] != 'Hello':
            measurements['failed_lookup'] = lookup
            measurements['failed_introspection'] = introspection
            raise RuntimeError(f'cycle {index + 1}: interface lookup disagrees with introspection: {lookup}')
        before = time.monotonic()
        renderer = (evaluate(f'{BRIDGE}.startRenderer({json.dumps(arguments)})') if index == 0
                    else evaluate(f'{BRIDGE}._pid ?? null'))
        while time.monotonic() - before < 3:
            current = snapshot()
            if current['fresh'] and current['damage'] and current['buffer']:
                break
            time.sleep(0.01)
        else:
            raise RuntimeError(f'cycle {index + 1}: no first frame: {current}')
        renderer = current['pid']
        elapsed = (time.monotonic() - before) * 1000
        private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Extensions',
            '-o', '/org/gnome/Shell/Extensions', '-m',
            'org.gnome.Shell.Extensions.DisableExtension', 'wallpaper-spike@one-mk.test'])
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            windows = evaluate('global.display.list_all_windows().filter(w => w.get_title().includes("M1a isolated probe")).length')
            if not Path(f'/proc/{renderer}').exists() and windows == 0:
                break
            time.sleep(0.01)
        else:
            raise RuntimeError(f'cycle {index + 1}: renderer/window leaked')
        private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Extensions',
            '-o', '/org/gnome/Shell/Extensions', '-m',
            'org.gnome.Shell.Extensions.EnableExtension', 'wallpaper-spike@one-mk.test'])
        measurements['cycles'].append({'cycle': index + 1, 'first_damage_ms': round(elapsed, 3),
            'old_exited': True, 'disabled_window_count': windows})
