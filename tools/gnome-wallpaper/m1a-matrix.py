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
    here = Path(__file__).resolve().parent

    def extension(action, uuid=UUID):
        if args_baseline and uuid == UUID:
            return 'wallpaper extension disabled for baseline'
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
        if args_baseline:
            time.sleep(0.1)
            return {'pid': shell, 'first_damage_ms': None, 'snapshot': snapshot()}
        before = time.monotonic()
        configured = evaluate(f'!!{BRIDGE}._recovery.argv')
        if not configured:
            evaluate(f'{BRIDGE}.startRenderer({__import__("json").dumps(arguments)})')
        current = wait_frame()
        return {'pid': current['pid'], 'first_damage_ms': round((time.monotonic() - before) * 1000, 3),
                'snapshot': current}

    def automatic(previous_pid):
        before = time.monotonic()
        while time.monotonic() - before < 7:
            current = snapshot()
            pid = evaluate(f'{BRIDGE}?._pid ?? null')
            if pid and pid != previous_pid and current['fresh'] and current['buffer']:
                return {'pid': pid, 'observed_ms': round((time.monotonic() - before) * 1000, 2), 'snapshot': current}
            time.sleep(0.025)
        raise RuntimeError('automatic recovery did not produce fresh owned damage')

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

    args_baseline = report.get('baseline', False)
    leader = int((root / 'leader.pid').read_text())
    shell = next(pid for pid in descendants(leader) if owned_shell(Path(f'/proc/{pid}'), root))
    measurements['shell_pid'] = shell

    def measure_cpu(pid, seconds=3):
        wall_start = time.time()
        start = time.monotonic()
        before = cpu(pid), cpu(shell)
        time.sleep(seconds)
        elapsed = time.monotonic() - start
        after = cpu(pid), cpu(shell)
        return {'seconds': round(elapsed, 3), 'wall_start': wall_start, 'wall_end': time.time(),
            'before_ticks': before, 'after_ticks': after,
            'renderer_percent_one_core': round(
            100 * (after[0] - before[0]) / os.sysconf('SC_CLK_TCK') / elapsed, 2),
            'shell_percent_one_core': round(100 * (after[1] - before[1]) /
                                          os.sysconf('SC_CLK_TCK') / elapsed, 2)}

    def burst(label, action):
        origin = evaluate('Main.extensionManager.lookup("spike-harness@one-mk.test").stateObj.timeUs()')
        action()
        time.sleep(2)
        return compositor_timing(origin)

    def compositor_timing(origin):
        events = evaluate('Main.extensionManager.lookup("spike-harness@one-mk.test").stateObj.events')
        damage = [] if args_baseline else evaluate(
            'Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj._recovery.damageTimes')
        events = [event for event in events if event['us'] >= origin]
        frames = [frame for frame in damage if frame['us'] >= origin]
        return {'origin_us': origin, 'shell_events': events, 'damage': frames,
                'snapshot': snapshot(), 'interpretation': 'compositor damage, not physical scanout'}

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
    for count, scale in ([] if report.get('idle_only') else
                        [(count, scale) for scale in (1, 2, 1.25) for count in (1, 2, 3)]):
        try:
            layout(count, scale)
            time.sleep(0.5)
            current = snapshot()
            measurements['layouts'].append({'monitors': count, 'scale': scale, 'snapshot': current})
        except RuntimeError as error:
            measurements['layouts'].append({'monitors': count, 'scale': scale, 'unavailable': str(error)})
    layout(2)
    time.sleep(0.5)
    if not report.get('idle_only'):
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
        measurements['covered'] = ({} if args_baseline else evaluate(f'({{paused: {BRIDGE}._paused, reason: {BRIDGE}._reason}})'))
    finally:
        cover.terminate()
        cover_second.terminate()
        cover.wait(timeout=5)
        cover_second.wait(timeout=5)
    time.sleep(0.5)
    if report.get('idle_only'):
        measurements['resumed'] = snapshot()
        measurements['qualification_failures'] = []
        if not measurements['covered']['paused']:
            measurements['qualification_failures'].append('coverage did not pause renderer')
        if measurements['resumed']['damage'] <= measurements['paused_after']['damage']:
            measurements['qualification_failures'].append('event-driven resume produced no new damage')
        if any(measurements['paused_after'][source] for source in ('watchdog', 'restart', 'coverage')):
            measurements['qualification_failures'].append('settled paused extension retains a source')
        extension('Disable')
        deadline = time.monotonic() + 3
        while Path(f'/proc/{pid}').exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        measurements['off_renderer_exists'] = Path(f'/proc/{pid}').exists()
        if measurements['off_renderer_exists']:
            measurements['qualification_failures'].append('disabled renderer was not reaped')
        return measurements
    if args_baseline:
        measurements['renderer_faults'] = 'not applicable: extension disabled, no renderer'
    else:
        renderer_faults(evaluate, snapshot, automatic, burst, measurements, pid)
    pre_lock_pid = None if args_baseline else evaluate(f'{BRIDGE}._pid')
    measurements['lock_shots'] = burst('lock', lambda: evaluate('Main.screenShield.lock(false); "locked"'))
    measurements['locked_state'] = evaluate('({locked: Main.screenShield.locked, mode: Main.sessionMode.currentMode})')
    measurements['unlock_shots'] = burst('unlock', lambda: evaluate('Main.screenShield.deactivate(false); "unlock"'))
    measurements['unlock_restart'] = start() if args_baseline else automatic(pre_lock_pid)
    measurements['unlock_compositor'] = compositor_timing(measurements['unlock_shots']['origin_us'])
    measurements['cycles'] = []
    for index in range(100):
        current_pid = None if args_baseline else evaluate(f'{BRIDGE}._pid')
        extension('Disable')
        gone = args_baseline
        deadline = time.monotonic() + 3
        disabled_windows = None
        while time.monotonic() < deadline:
            try:
                status = Path(f'/proc/{current_pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
                gone = status == 'Z'
            except FileNotFoundError:
                gone = True
            disabled_windows = evaluate('global.display.list_all_windows().filter(w => w.get_title().includes("M1a isolated probe")).length')
            if gone and disabled_windows == 0:
                break
            time.sleep(0.02)
        extension('Enable')
        fresh = start()
        measurements['cycles'].append({'cycle': index + 1, 'old_pid': current_pid,
            'old_exited': gone, 'first_damage_ms': fresh['first_damage_ms'], 'pid': fresh['pid'],
            'disabled_window_count': disabled_windows,
            'window_count': evaluate('global.display.list_all_windows().filter(w => w.get_title().includes("M1a isolated probe")).length'),
            'windows': evaluate('global.display.list_all_windows().filter(w => w.get_title().includes("M1a isolated probe")).map(w => ({id: w.get_stable_sequence(), pid: w.get_pid(), actor: !!w.get_compositor_private(), hidden: w.is_hidden(), type: w.get_window_type()}))')})
    if not args_baseline:
        evaluate(f'{BRIDGE}._recovery.attempts = []')
        measurements['restart_cap'] = []
        for index in range(4):
            killed = evaluate(f'{BRIDGE}._pid')
            os.kill(killed, signal.SIGKILL)
            if index < 3:
                recovered = automatic(killed)
            else:
                time.sleep(1)
                recovered = evaluate(f'({{exhausted: {BRIDGE}._recovery.exhausted, pid: {BRIDGE}._pid, timer: {BRIDGE}._restartId}})')
            measurements['restart_cap'].append(recovered)
        extension('Disable')
        extension('Enable')
        time.sleep(1)
        measurements['cap_after_reenable'] = evaluate(f'({{exhausted: {BRIDGE}._recovery.exhausted, pid: {BRIDGE}._pid ?? null, timer: {BRIDGE}._restartId}})')
    extension('Disable')
    time.sleep(1)
    measurements['ding_state'] = evaluate('({state: Main.extensionManager.lookup("ding@rastersoft.com")?.state, '
                                        'errors: Main.extensionManager.lookup("ding@rastersoft.com")?.errors})')
    measurements['final_leaks'] = evaluate((here / 'js/leaks.js').read_text())
    measurements['final_managed_windows'] = evaluate('global.display.list_all_windows().map(w => w.get_title())')
    if args_baseline:
        measurements['qualification_failures'] = []
        return measurements
    failures = []
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
    if len(measurements['cycles']) != 100 or not all(cycle['old_exited'] and cycle['window_count'] == 1 and cycle['disabled_window_count'] == 0 for cycle in measurements['cycles']):
        failures.append('100-cycle process/window invariant failed')
    if not measurements['covered']['paused']:
        failures.append('all-monitor coverage did not pause playback')
    if (len(measurements['restart_cap']) != 4
            or not all(entry.get('snapshot', {}).get('fresh') for entry in measurements['restart_cap'][:3])
            or measurements['restart_cap'][-1] != {'exhausted': True, 'pid': None, 'timer': 0}):
        failures.append('bounded restart did not give up without timers')
    if measurements['cap_after_reenable'] != {'exhausted': True, 'pid': None, 'timer': 0}:
        failures.append('extension re-enable reset exhausted restart budget')
    if measurements['paused_after']['watchdog'] or measurements['paused_after']['restart']:
        failures.append('paused extension has a periodic/restart source')
    measurements['qualification_failures'] = failures
    return measurements


def renderer_faults(evaluate, snapshot, automatic, burst, measurements, pid):
    os.kill(pid, signal.SIGSTOP)
    before = time.monotonic()
    while time.monotonic() - before < 5 and not snapshot()['stale']:
        time.sleep(0.05)
    measurements['watchdog'] = {'observed_ms': round((time.monotonic() - before) * 1000, 2), 'snapshot': snapshot()}
    recovered = automatic(pid)
    measurements['post_hang_restart'] = recovered
    pid = recovered['pid']
    current = recovered['snapshot']
    measurements['kill9_before'] = current
    measurements['kill9_shots'] = burst('kill9', lambda: os.kill(pid, signal.SIGKILL))
    measurements['post_kill_restart'] = automatic(pid)


def owned_shell(path, root):
    try:
        return ((path / 'comm').read_text().strip() == 'gnome-shell'
                and f'XDG_RUNTIME_DIR={root}/run'.encode() in (path / 'environ').read_bytes().split(b'\0'))
    except (PermissionError, FileNotFoundError, ProcessLookupError):
        return False
