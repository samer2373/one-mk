#!/usr/bin/env python3
"""Product-protocol conformance, exclusively inside the private Mutter harness."""
import json
import os
from pathlib import Path
import pwd
import runpy
import shutil
import subprocess
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
LAB = REPO / 'target/gnome-component-qa'
SESSION = LAB / 'session'
UUID = 'one-mk-wallpaper@one-mk.org'
HARNESS = HERE / 'isolated-session.sh'
BRIDGE = f'Main.extensionManager.lookup("{UUID}").stateObj.bridge'
ENV = {'PATH': '/usr/bin:/bin', 'HOME': str(LAB / 'home'),
       'USER': pwd.getpwuid(os.getuid()).pw_name, 'LANG': 'C.UTF-8',
       'SPIKE': str(SESSION), 'FAKE_GDM': '1', 'PYTHONDONTWRITEBYTECODE': '1'}


def run(arguments, check=True):
    return subprocess.run([str(value) for value in arguments], env=ENV, cwd=REPO,
                          text=True, capture_output=True, timeout=45, check=check)


def private(arguments, check=True):
    return run([HARNESS, 'run', *arguments], check)


def extension(action, uuid=UUID):
    return private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Extensions',
                    '-o', '/org/gnome/Shell/Extensions', '-m',
                    f'org.gnome.Shell.Extensions.{action}Extension', uuid]).stdout


def evaluate(source):
    result = private(['python3', HERE / 'client/shell_eval.py', source]).stdout
    if result.startswith('EVAL FAILED'):
        raise RuntimeError(result)
    return json.loads(result)


def main():
    LAB.mkdir(parents=True, exist_ok=True)
    (LAB / 'home').mkdir(exist_ok=True)
    report = {'checks': [], 'production_playback_qualified': False}
    squat = None
    sources = LAB / 'extensions'
    if sources.exists():
        shutil.rmtree(sources)
    sources.mkdir()
    destination = sources / UUID
    shutil.copytree(REPO / 'extensions/gnome' / UUID, destination)
    shutil.copytree(HERE / 'ext/spike-harness@one-mk.test', sources / 'spike-harness@one-mk.test')
    (destination / 'libexec').mkdir()
    shutil.copy2(REPO / 'target/debug/examples/bridge-conformance', destination / 'libexec/one-mk-wall')
    ENV['EXT_SRC'] = str(sources)
    try:
        report['shell'] = run(['gnome-shell', '--version']).stdout.strip()
        metadata = json.loads((destination / 'metadata.json').read_text())
        assert report['shell'].removeprefix('GNOME Shell ') in metadata['one-mk-qualified-shell-versions']
        run([HARNESS, 'start', '2'])
        extension('Enable', 'spike-harness@one-mk.test')
        evaluate('Main.overview.hide(); true')
        cli = REPO / 'target/debug/one-mk'
        cli_data = SESSION / 'cli-data'
        cli_command = ['env', f'XDG_DATA_HOME={cli_data}', cli, 'wallpaper', '--json', 'extension']
        installed = json.loads(private([*cli_command, 'install', '--renderer',
                                      REPO / 'target/debug/examples/bridge-conformance']).stdout)
        assert installed['ok'] and 'Not enabled' in installed['message']
        status = json.loads(private([*cli_command, 'status']).stdout)
        assert status['installed'] and not status['enabled'] and status['qualified'], status
        private([*cli_command, 'uninstall'])
        assert not (cli_data / 'gnome-shell/extensions' / UUID).exists()
        report['checks'].append('CLI install/status/uninstall in private XDG data; never auto-enabled')
        extension('Enable')
        assert evaluate(f'Boolean({BRIDGE}._ownsName)')
        assert evaluate(f'{BRIDGE}._pid === null && {BRIDGE}._authorized === false')
        report['checks'].append('default dormant, no renderer or authorization')
        refused = private(['gdbus', 'call', '--session', '-d', 'org.onemk.WallpaperBridge1',
                           '-o', '/org/onemk/WallpaperBridge1', '-m',
                           'org.onemk.WallpaperBridge1.Hello', '1'], check=False)
        assert refused.returncode and 'AccessDenied' in refused.stderr, refused
        report['checks'].append('unspawned caller refused before Hello')
        evaluate(f'{BRIDGE}._authorized = true; {BRIDGE}._coverage(); true')
        for _ in range(100):
            log = (SESSION / 'log/shell.log').read_text()
            if 'production bridge awaiting layout' in log:
                break
            time.sleep(0.1)
        assert 'production bridge awaiting layout' in log, log[-6000:]
        evaluate('Main.layoutManager.emit("monitors-changed"); true')
        for _ in range(50):
            log = (SESSION / 'log/shell.log').read_text()
            if 'production bridge awaiting reasons' in log:
                break
            time.sleep(0.1)
        assert 'production bridge awaiting reasons' in log, log[-6000:]
        evaluate(f'{BRIDGE}._emit("PauseHint", new imports.gi.GLib.Variant("(u)", [15])); '
                 f'{BRIDGE}._emit("PauseHint", new imports.gi.GLib.Variant("(u)", [16])); true')
        for _ in range(50):
            log = (SESSION / 'log/shell.log').read_text()
            if 'production bridge conformance:' in log:
                break
            time.sleep(0.1)
        assert 'production bridge conformance:' in log, log[-6000:]
        report['checks'].append('Rust Hello/snapshot, layout serials, unicast isolation, pause masks, state/type/version/rate negatives')
        evaluate(f'{BRIDGE}._authorized = false; {BRIDGE}._stop(); true')
        for _ in range(20):
            if evaluate(f'{BRIDGE}._recovery.children.size') == 0:
                break
            time.sleep(0.1)
        for _ in range(20):
            extension('Disable')
            extension('Enable')
            assert evaluate(f'Boolean({BRIDGE}._ownsName)')
            evaluate('Main.extensionManager.lookup("spike-harness@one-mk.test").stateObj.collectGarbage(); true')
        report['checks'].append('20 enable/disable + GC cycles, no renderer children')
        extension('Disable')
        code = ('from gi.repository import Gio, GLib; '
                'bus=Gio.bus_get_sync(Gio.BusType.SESSION); '
                'Gio.bus_own_name_on_connection(bus,"org.onemk.WallpaperBridge1",'
                'Gio.BusNameOwnerFlags.DO_NOT_QUEUE,lambda *args: print("ready",flush=True),None); '
                'GLib.MainLoop().run()')
        with (LAB / 'squatter.log').open('w') as output:
            squat = subprocess.Popen([str(HARNESS), 'run', 'python3', '-c', code],
                                     cwd=REPO, env=ENV, stdout=output, stderr=output)
        for _ in range(30):
            if 'ready' in (LAB / 'squatter.log').read_text():
                break
            time.sleep(0.1)
        assert 'ready' in (LAB / 'squatter.log').read_text()
        extension('Enable')
        time.sleep(0.2)
        assert evaluate(f'!{BRIDGE}._ownsName && {BRIDGE}._pid === null && '
                        f'{BRIDGE}.reason.includes("name unavailable")')
        extension('Disable')
        squat.terminate()
        squat.wait(timeout=5)
        squat = None
        extension('Enable')
        report['checks'].append('bus-name squatting refused, no renderer start')
        renderer = SESSION / 'data/gnome-shell/extensions' / UUID / 'libexec/one-mk-wall'
        renderer.rename(renderer.with_suffix('.held'))
        evaluate(f'{BRIDGE}._recovery.attempts = []; {BRIDGE}._authorized = true; {BRIDGE}._restart(); true')
        for _ in range(110):
            if evaluate(f'{BRIDGE}._recovery.exhausted'):
                break
            time.sleep(0.1)
        assert evaluate(f'{BRIDGE}._recovery.exhausted && {BRIDGE}._recovery.attempts.length === 3 && '
                        f'{BRIDGE}._recovery.children.size === 0 && {BRIDGE}._sources.size === 0')
        state = json.loads((SESSION / 'run/one-mk-wallpaper/status.json').read_text())
        assert state['restart_limit_reached'] and 'restart limit reached' in state['reason']
        doctor = private([cli, 'wallpaper', 'doctor']).stdout
        assert 'restart limit reached' in doctor, doctor
        status = json.loads(private([cli, 'wallpaper', '--json', 'extension', 'status']).stdout)
        assert status['enabled'] and status['qualified'] and status['restart_limit_reached'], status
        report['checks'].append('actual 1/2/4-second failed-spawn cap, no remaining timer/child, doctor-visible restart limit')
        extension('Disable')
        log = (SESSION / 'log/shell.log').read_text()
        gate = runpy.run_path(str(HERE / 'm1-risk-probe.py'))
        baseline = (HERE / 'm1a2-evidence/baseline-shell.log').read_text()
        report['new_warnings'] = gate['new_warnings'](log, baseline)
        assert not report['new_warnings'], report['new_warnings']
        (LAB / 'shell.log').write_text(log)
        evaluate(f'Main.extensionManager.lookup("{UUID}").stateObj.metadata["one-mk-qualified-shell-versions"] = []; true')
        extension('Enable')
        state = json.loads((SESSION / 'run/one-mk-wallpaper/status.json').read_text())
        assert 'unqualified GNOME Shell 50.1' in state['reason'], state
        report['checks'].append('exact-version guard refuses lab Shell when absent from qualified list (separate expected-error log)')
        report['passed'] = True
    except BaseException as error:
        report['passed'] = False
        report['failure'] = str(error)
        raise
    finally:
        if squat is not None:
            squat.terminate()
            squat.wait(timeout=5)
        if (SESSION / 'log/shell.log').exists():
            shutil.copy2(SESSION / 'log/shell.log', LAB / ('version-negative-shell.log' if report.get('passed') else 'shell.log'))
        run([HARNESS, 'stop'], check=False)
        shutil.rmtree(sources)
        (LAB / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
