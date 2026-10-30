#!/usr/bin/env python3
"""Negative bridge checks inside the supervisor's isolated headless Shell."""
import json
import subprocess
import time


BRIDGE = 'Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge'


def run_security(evaluate, private, arguments, root, here, harness, environment, report):
    def extension(action):
        return private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Extensions',
                        '-o', '/org/gnome/Shell/Extensions', '-m',
                        f'org.gnome.Shell.Extensions.{action}Extension', 'wallpaper-spike@one-mk.test'])

    squatter = subprocess.Popen([str(harness), 'run', 'python3',
                                str(here / 'client/bridge_squatter.py'), str(root)], env=environment)
    checks = {}
    report['measurements'] = checks
    try:
        deadline = time.monotonic() + 5
        while not (root / 'log/squatter-ready').exists():
            if squatter.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('squatter fixture did not acquire private bridge name')
            time.sleep(0.025)
        extension('Enable')
        deadline = time.monotonic() + 5
        while not evaluate(f'{BRIDGE}._bridgeFailure'):
            if time.monotonic() > deadline:
                raise RuntimeError('extension did not fail closed on name squatting')
            time.sleep(0.025)
        checks['name_squatting'] = evaluate(f'''(() => {{
            const bridge = {BRIDGE};
            let refused = false;
            try {{ bridge.startRenderer({json.dumps(arguments)}); }} catch (_) {{ refused = true; }}
            return {{failure: bridge._bridgeFailure, ownsName: bridge._ownsName,
                children: bridge._recovery.children.size, restart: bridge._restartId, refused}};
        }})()''')
        assert checks['name_squatting'] == {'failure': 'bridge-name-unavailable',
            'ownsName': False, 'children': 0, 'restart': 0, 'refused': True}
        evaluate(f'''(() => {{ const bridge = {BRIDGE};
            bridge._ownsName = true; bridge._bridgeReady = true; bridge._bridgeFailure = null;
            bridge._recovery.argv = {json.dumps(arguments)};
            bridge._spawnRenderer(); bridge._ownsName = false; bridge._bridgeReady = false;
            bridge._bridgeFailure = 'bridge-name-unavailable'; return true; }})()''')
        deadline = time.monotonic() + 5
        while evaluate(f'{BRIDGE}._recovery.children.size'):
            if time.monotonic() > deadline:
                raise RuntimeError('renderer did not refuse foreign bridge unique-name/PID')
            time.sleep(0.025)
        assert not (root / 'log/squatter-hello').exists()
        log = (root / 'log/shell.log').read_text(errors='replace')
        assert 'bridge unique owner is not the authenticated private Wayland peer' in log
        checks['renderer_refused_before_hello'] = True
        extension('Disable')
    finally:
        squatter.terminate()
        try:
            squatter.wait(timeout=3)
        except subprocess.TimeoutExpired:
            squatter.kill()
            squatter.wait()
    extension('Enable')
    interface = private(['gdbus', 'introspect', '--session', '-d', 'org.onemk.WallpaperBridgeSpike1',
                         '-o', '/org/onemk/WallpaperBridgeSpike1'])
    assert 'FrameReady' not in interface
    checks['frame_file_method_absent'] = True
    evaluate(f'{BRIDGE}.startRenderer({json.dumps(["/usr/bin/python3", str(here / "client/replacement_client.py")])})')
    deadline = time.monotonic() + 5
    while not evaluate(f'{BRIDGE}._retiring.size'):
        if time.monotonic() > deadline:
            raise RuntimeError('replacement fixture did not retain old window')
        time.sleep(0.025)
    checks['retired_window_rehidden'] = evaluate('''(() => {
        const actor = global.get_window_actors().find(a => a.meta_window.get_title() === 'M1 replacement old');
        actor.show(); return !actor.visible; })()''')
    assert checks['retired_window_rehidden']
    deadline = time.monotonic() + 5
    while evaluate(f'{BRIDGE}._recovery.children.size'):
        if time.monotonic() > deadline:
            raise RuntimeError('uncooperative replaced window was not terminated')
        time.sleep(0.025)
    assert evaluate(f'{BRIDGE}._retiring.size') == 0
    checks['uncooperative_replacement_reaped'] = True
    checks['shell_responsive_after_negative_tests'] = evaluate('1 + 1') == 2
    extension('Disable')
    return checks
