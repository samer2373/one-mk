#!/usr/bin/env python3
"""Presentation-only measurements; imported by the isolated M1 supervisor."""
import os
from pathlib import Path
import signal
import time


BRIDGE = 'Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge'
UUID = 'wallpaper-spike@one-mk.test'


def run_matrix(evaluate, private, snapshot, arguments, root, lab, report):
    measurements = {}
    report['measurements'] = measurements
    shots = lab / ('m1a-shots-' + report['date_utc'].replace(':', '-'))
    shots.mkdir(exist_ok=True)
    here = Path(__file__).resolve().parent

    def extension(action, uuid=UUID):
        return private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Extensions',
                        '-o', '/org/gnome/Shell/Extensions', '-m',
                        f'org.gnome.Shell.Extensions.{action}Extension', uuid])

    def wait_frame(deadline=5):
        start = time.monotonic()
        while time.monotonic() - start < deadline:
            current = snapshot()
            if current['damage'] and current['buffer']:
                return current
            time.sleep(0.025)
        raise RuntimeError('no real renderer damage within deadline')

    def start():
        evaluate(f'{BRIDGE}.stats.damage = 0')
        before = time.monotonic()
        pid = evaluate(f'{BRIDGE}.startRenderer({__import__("json").dumps(arguments)})')
        current = wait_frame()
        return {'pid': pid, 'first_damage_ms': round((time.monotonic() - before) * 1000, 3),
                'snapshot': current}

    def cpu(pid):
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return int(fields[11]) + int(fields[12])

    def descendants(pid):
        children = Path(f'/proc/{pid}/task/{pid}/children').read_text().split()
        result = []
        for child in children:
            result.append(int(child))
            try:
                result.extend(descendants(int(child)))
            except FileNotFoundError:
                pass
        return result

    leader = int((root / 'leader.pid').read_text())
    shell = next(pid for pid in descendants(leader) if owned_shell(Path(f'/proc/{pid}'), root))
    measurements['shell_pid'] = shell

    def measure_cpu(pid, seconds=3):
        start = time.monotonic()
        before = cpu(pid), cpu(shell)
        time.sleep(seconds)
        elapsed = time.monotonic() - start
        after = cpu(pid), cpu(shell)
        return {'seconds': round(elapsed, 3), 'before_ticks': before, 'after_ticks': after,
            'renderer_percent_one_core': round(
            100 * (after[0] - before[0]) / os.sysconf('SC_CLK_TCK') / elapsed, 2),
            'shell_percent_one_core': round(100 * (after[1] - before[1]) /
                                          os.sysconf('SC_CLK_TCK') / elapsed, 2)}

    def burst(label, action):
        before = time.monotonic()
        action()
        frames = []
        for index in range(9):
            target = before + index * 0.25
            time.sleep(max(0, target - time.monotonic()))
            shot = shots / f'{label}-{index:02}.png'
            captured = time.monotonic()
            reply = private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Screenshot',
                '-o', '/org/gnome/Shell/Screenshot', '-m', 'org.gnome.Shell.Screenshot.Screenshot',
                'false', 'false', str(shot)])
            frames.append({'scheduled_ms': index * 250, 'requested_ms': round((captured - before) * 1000, 2),
                'completed_ms': round((time.monotonic() - before) * 1000, 2),
                'file': str(shot.relative_to(lab)), 'reply': reply, 'snapshot': snapshot()})
        return frames

    def layout(count, scale=1):
        private(['python3', str(here / 'client/displayconfig.py'), 'apply', str(count), str(scale)])

    ding = Path('/usr/share/gnome-shell/extensions/ding@rastersoft.com')
    if ding.exists():
        import shutil
        shutil.copytree(ding, root / 'data/gnome-shell/extensions/ding@rastersoft.com')
        measurements['ding_enable'] = extension('Enable', 'ding@rastersoft.com')
    else:
        measurements['ding_enable'] = 'UNAVAILABLE'
    evaluate('Main.overview.hide(); "hidden"')
    time.sleep(1)
    measurements['startup'] = start()
    pid = measurements['startup']['pid']
    measurements['cpu_on'] = measure_cpu(pid)
    measurements['layouts'] = []
    for count, scale in [(1, 1), (2, 1), (3, 1), (1, 2), (2, 2), (3, 2), (3, 1.25)]:
        try:
            layout(count, scale)
            time.sleep(0.5)
            current = snapshot()
            measurements['layouts'].append({'monitors': count, 'scale': scale, 'snapshot': current})
        except RuntimeError as error:
            measurements['layouts'].append({'monitors': count, 'scale': scale, 'unavailable': str(error)})
    layout(2)
    time.sleep(0.5)
    measurements['unplug_shots'] = burst('unplug', lambda: layout(1))
    measurements['replug_shots'] = burst('replug', lambda: layout(2))
    measurements['paused_before'] = snapshot()
    evaluate('Main.overview.show(); "overview"')
    time.sleep(1)
    measurements['cpu_paused'] = measure_cpu(pid)
    measurements['paused_after'] = snapshot()
    evaluate('Main.overview.hide(); "hidden"')
    time.sleep(1)
    from subprocess import Popen
    from runpy import run_path
    helper = run_path(str(here / 'm1-risk-probe.py'))
    with open(root / 'log/cover.log', 'w') as log:
        cover = Popen([str(here / 'isolated-session.sh'), 'run', 'python3',
            str(here / 'client/test_window.py'), 'M1a coverage', '1920', '1080'],
            env=helper['clean_environment'](), stdout=log,
            stderr=__import__('subprocess').STDOUT)
        cover_second = Popen([str(here / 'isolated-session.sh'), 'run', 'python3',
            str(here / 'client/test_window.py'), 'M1a coverage second', '1920', '1080'],
            env=helper['clean_environment'](), stdout=log,
            stderr=__import__('subprocess').STDOUT)
    try:
        time.sleep(1)
        for monitor, title in enumerate(['M1a coverage', 'M1a coverage second']):
            evaluate('(() => { const win = global.get_window_actors().find(a => '
                     f'a.meta_window.get_title() === "{title}").meta_window; '
                     f'win.move_to_monitor({monitor}); win.make_fullscreen(); return "covered"; }})()')
        time.sleep(1)
        measurements['cpu_covered'] = measure_cpu(pid)
        measurements['covered'] = evaluate(f'({{paused: {BRIDGE}._paused, reason: {BRIDGE}._reason}})')
    finally:
        cover.terminate()
        cover_second.terminate()
        cover.wait(timeout=5)
        cover_second.wait(timeout=5)
    time.sleep(0.5)
    os.kill(pid, signal.SIGSTOP)
    before = time.monotonic()
    while time.monotonic() - before < 5 and not snapshot()['stale']:
        time.sleep(0.05)
    measurements['watchdog'] = {'observed_ms': round((time.monotonic() - before) * 1000, 2), 'snapshot': snapshot()}
    old_damage = snapshot()['damage']
    os.kill(pid, signal.SIGCONT)
    before = time.monotonic()
    while time.monotonic() - before < 65:
        current = snapshot()
        if current['damage'] > old_damage and current['cloneCount'] and not current['stale']:
            break
        time.sleep(0.05)
    else:
        raise RuntimeError('renderer did not recover after SIGCONT within watchdog probe backoff')
    measurements['kill9_before'] = current
    measurements['kill9_shots'] = burst('kill9', lambda: os.kill(pid, signal.SIGKILL))
    measurements['post_kill_restart'] = start()
    measurements['lock_shots'] = burst('lock', lambda: evaluate('Main.screenShield.lock(false); "locked"'))
    measurements['locked_state'] = evaluate('({locked: Main.screenShield.locked, mode: Main.sessionMode.currentMode})')
    measurements['unlock_shots'] = burst('unlock', lambda: evaluate('Main.screenShield.deactivate(false); "unlock"'))
    measurements['unlock_restart'] = start()
    measurements['restart_limit'] = 'NOT IMPLEMENTED by the 6B bridge; harness starts are manual, not a production cap'
    measurements['cycles'] = []
    for index in range(100):
        current_pid = evaluate(f'{BRIDGE}._pid')
        extension('Disable')
        gone = False
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            try:
                status = Path(f'/proc/{current_pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
                gone = status == 'Z'
            except FileNotFoundError:
                gone = True
            if gone:
                break
            time.sleep(0.02)
        extension('Enable')
        fresh = start()
        measurements['cycles'].append({'cycle': index + 1, 'old_pid': current_pid,
            'old_exited': gone, 'first_damage_ms': fresh['first_damage_ms'], 'pid': fresh['pid'],
            'window_count': evaluate('global.display.list_all_windows().filter(w => w.get_title().includes("M1a isolated probe")).length')})
    extension('Disable')
    time.sleep(1)
    measurements['ding_state'] = evaluate('({state: Main.extensionManager.lookup("ding@rastersoft.com")?.state, '
                                        'errors: Main.extensionManager.lookup("ding@rastersoft.com")?.errors})')
    measurements['final_leaks'] = evaluate((here / 'js/leaks.js').read_text())
    measurements['final_managed_windows'] = evaluate('global.display.list_all_windows().map(w => w.get_title())')
    failures = ['6B restart budget not implemented; cascade qualification unavailable']
    for layout_result in measurements['layouts']:
        if 'unavailable' in layout_result:
            failures.append(f'layout unavailable: {layout_result}')
            continue
        current = layout_result['snapshot']
        count, scale = layout_result['monitors'], layout_result['scale']
        expected = [0, 0, round(1920 / scale) * count, round(1080 / scale)]
        if current['buffer'] != expected or current['cloneCount'] != count:
            failures.append(f'layout extent/clone mismatch: {count} monitors at scale {scale}')
    if measurements['watchdog']['observed_ms'] > 3000 or not measurements['watchdog']['snapshot']['stale']:
        failures.append('3-second watchdog deadline missed')
    if any('M1a isolated probe' in title for title in measurements['final_managed_windows']):
        failures.append('managed renderer windows retained after disable')
    if not all(cycle['old_exited'] and cycle['window_count'] == 1 for cycle in measurements['cycles']):
        failures.append('100-cycle process/window invariant failed')
    if not measurements['covered']['paused']:
        failures.append('all-monitor coverage did not pause playback')
    measurements['qualification_failures'] = failures
    return measurements


def owned_shell(path, root):
    try:
        return ((path / 'comm').read_text().strip() == 'gnome-shell'
                and f'XDG_RUNTIME_DIR={root}/run'.encode() in (path / 'environ').read_bytes().split(b'\0'))
    except (PermissionError, FileNotFoundError, ProcessLookupError):
        return False
