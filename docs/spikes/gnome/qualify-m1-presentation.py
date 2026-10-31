#!/usr/bin/env python3
"""6D-M1 step A end-to-end qualification, exclusively inside the private Mutter harness.

Real processes only: the one-mk daemon (null input backend, laboratory build
with `wallpaper-m1-lab`) serves the presentation authority on the private
session bus; the private GNOME Shell spawns the real renderer (`one-mk-wall`
built with `m1-lab`, launched through a one-line exec wrapper that only adds
`--m1-lab` and a log redirect, keeping the PID and inherited Wayland socket).

Environment (all required):
  M1_BIN       directory holding the laboratory `one-mk` and `one-mk-wall`
  M1_LAB       a short, new private directory (Unix socket paths must fit)
  M1_CYCLES    start/stop cycles (default 100)

Never connects to the owner's desktop, X servers, clipboard or session bus.
The renderer creates no surface or decoder: "admitted"/"active" is intent.
"""
import json
import os
from pathlib import Path
import pwd
import random
import runpy
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
BIN = Path(os.environ['M1_BIN']).resolve()
LAB = Path(os.environ['M1_LAB']).resolve()
CYCLES = int(os.environ.get('M1_CYCLES', '100'))
SESSION = LAB / 's'
HOME = SESSION / 'one-mk'
RECORD_DIR = SESSION / 'run/one-mk-wallpaper-m1'
RECORD = RECORD_DIR / 'authority'
RENDERER_LOG = LAB / 'renderer.log'
UUID = 'one-mk-wallpaper@one-mk.org'
HARNESS = HERE / 'isolated-session.sh'
BRIDGE = f'Main.extensionManager.lookup("{UUID}").stateObj.bridge'
PEER = 'ab' * 32
MEDIA = '5e' * 32
# Two 1920x1080 monitors at x=0 and x=1920; this computer sits in the middle
# of a three-computer canvas whose left third (an absent computer) stays reserved.
CONFIG = {'peer': PEER, 'media': MEDIA, 'canvas': [-1920, 0, 5760, 1080], 'origin': [0, 0]}
EXIT_BOUND = 2.0
ENV = {'PATH': '/usr/bin:/bin', 'HOME': str(LAB / 'home'),
       'USER': pwd.getpwuid(os.getuid()).pw_name, 'LANG': 'C.UTF-8',
       'SPIKE': str(SESSION), 'FAKE_GDM': '1', 'PYTHONDONTWRITEBYTECODE': '1'}
REPORT = {'checks': [], 'production_authorization': False, 'surfaces_or_decoders_created': False}


def run(arguments, check=True, timeout=45):
    return subprocess.run([str(value) for value in arguments], env=ENV, cwd=LAB,
                          text=True, capture_output=True, timeout=timeout, check=check)


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


def wait(predicate, timeout, interval=0.02, what='condition'):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval)
    raise AssertionError(f'timed out after {timeout}s waiting for {what}')


def check(text):
    REPORT['checks'].append(text)
    print('PASS', text, flush=True)


# ---------------------------------------------------------------- processes

def state(pid):
    """'R'/'S'/... while alive, 'Z' once exited but unreaped, None once reaped."""
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError, IndexError):
        return None


def exited(pid):
    return state(pid) in (None, 'Z', 'X')


def lab_processes():
    """Every process carrying this laboratory's private runtime directory."""
    marker = f'XDG_RUNTIME_DIR={SESSION}/run'.encode()
    found = {}
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            if marker in (entry / 'environ').read_bytes().split(b'\0'):
                found[int(entry.name)] = (entry / 'comm').read_text().strip()
        except OSError:
            continue
    return found


def free_port():
    while True:
        port = random.randint(30000, 60000)
        try:
            for family, address in ((socket.AF_INET, '0.0.0.0'), (socket.AF_INET6, '::')):
                with socket.socket(family, socket.SOCK_DGRAM) as probe:
                    probe.bind((address, port))
            return port
        except OSError:
            continue


# ---------------------------------------------------------------- record

def encode_record(key, generation, pid, name):
    return (b'WAUT' + bytes([1]) + key + generation + pid.to_bytes(4, 'little') +
            bytes([len(name)]) + name.encode())


def decode_record(data):
    assert data[:5] == b'WAUT\x01', data[:5]
    key, generation = data[5:37], data[37:53]
    pid = int.from_bytes(data[53:57], 'little')
    name = data[58:58 + data[57]].decode()
    return {'key': key, 'generation': generation, 'pid': pid, 'name': name}


def write_record(data):
    staging = RECORD_DIR / 'authority.test'
    descriptor = os.open(staging, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as output:
        output.write(data)
    os.replace(staging, RECORD)


# ---------------------------------------------------------------- daemon

def write_settings(revision, enabled=True):
    HOME.mkdir(mode=0o700, exist_ok=True)
    (HOME / 'settings.toml').write_text(
        f'[wallpaper]\nversion = 1\nenabled = {"true" if enabled else "false"}\n\n'
        f'[wallpaper.grants]\n"{PEER}" = {revision}\n')


class Daemon:
    count = 0

    def __init__(self):
        Daemon.count += 1
        self.log_path = LAB / 'daemon.log'
        self.offset = self.log_path.stat().st_size if self.log_path.exists() else 0
        read, write = os.pipe()
        os.write(write, json.dumps(CONFIG).encode())
        os.close(write)
        self.log = self.log_path.open('a')
        self.log.write(f'--- daemon start {Daemon.count}\n')
        self.log.flush()
        self.process = subprocess.Popen(
            [str(HARNESS), 'run', 'env', f'ONE_MK_M1_LAB_FD={read}', str(BIN / 'one-mk'), 'run',
             '--backend', 'null', '--no-discovery', '--no-notify', '--no-clipboard', '--port', str(free_port())],
            env=ENV, cwd=LAB, stdout=self.log, stderr=self.log, pass_fds=(read,))
        os.close(read)
        self.pid = self.process.pid
        self.record = wait(self._ready, 15, what='daemon authority record')

    def _ready(self):
        if self.process.poll() is not None:
            raise AssertionError(f'daemon exited early: {self.output()[-2000:]}')
        try:
            record = decode_record(RECORD.read_bytes())
        except (FileNotFoundError, AssertionError, IndexError):
            return None
        return record if record['pid'] == self.pid and 'authority: ready' in self.output() else None

    def output(self):
        with self.log_path.open() as log:
            log.seek(self.offset)
            return log.read()

    def directive(self, text):
        (HOME / 'wallpaper-m1-lab-directive').write_text(text + '\n')
        os.kill(self.pid, signal.SIGUSR1)

    def stop(self):
        if self.process.poll() is None:
            private([BIN / 'one-mk', 'stop'], check=False)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.log.close()


# ---------------------------------------------------------------- renderer

def plan_fields(revision):
    """How the renderer logs the plan this laboratory's daemon must publish."""
    return (f'revision={revision} media={MEDIA[:16]} canvas=-1920,0,5760x1080 '
            'slices=monitor-0@0,0,1920x1080;monitor-1@1920,0,1920x1080')


def log_size():
    return RENDERER_LOG.stat().st_size if RENDERER_LOG.exists() else 0


def log_since(offset):
    if not RENDERER_LOG.exists():
        return ''
    with RENDERER_LOG.open() as log:
        log.seek(offset)
        return log.read()


def lines_for(pid, offset=0):
    tag = f'wallpaper-m1-renderer[{pid}]:'
    return [line.split(tag, 1)[1].strip() for line in log_since(offset).splitlines() if tag in line]


def renderers_since(offset):
    pids = []
    for line in log_since(offset).splitlines():
        if line.startswith('wallpaper-m1-renderer['):
            pid = int(line.split('[', 1)[1].split(']', 1)[0])
            if pid not in pids:
                pids.append(pid)
    return pids


def arm():
    """Private Eval laboratory authorization (never a production path). The
    renderer draws no surface in step A, so the extension's first-frame
    watchdog is disabled for this bridge instance only."""
    evaluate(f'(() => {{ const b = {BRIDGE}; if (!b._m1Lab) {{ const later = b._later.bind(b); '
             f'b._later = (name, ms, cb) => name === "first-frame" ? undefined : later(name, ms, cb); '
             f'b._m1Lab = true; }} b._recovery.attempts = []; b._recovery.exhausted = false; '
             f'b._authorized = true; b._coverage(); return true; }})()')


def disarm():
    evaluate(f'(() => {{ const b = {BRIDGE}; b._authorized = false; b._stop(); return true; }})()')
    wait(lambda: evaluate(f'{BRIDGE}._recovery.children.size') == 0, 5, 0.05, 'Shell child reaped')


def spawn(offset, timeout=8):
    """Arms the bridge and returns the first renderer PID that logs."""
    arm()
    return wait(lambda: renderers_since(offset)[:1], timeout, what='renderer start')[0]


def await_line(pid, offset, needle, timeout=5):
    return wait(lambda: next((line for line in lines_for(pid, offset) if needle in line), None),
                timeout, what=f'renderer {pid}: {needle}')


def await_exit(pid, start, timeout=EXIT_BOUND):
    wait(lambda: exited(pid), timeout, 0.005, f'renderer {pid} exit')
    return time.monotonic() - start


def reaped(pid, timeout=5):
    wait(lambda: state(pid) is None, timeout, 0.02, f'renderer {pid} reaped')


# ---------------------------------------------------------------- scenarios

def authentication():
    if RECORD.exists():
        RECORD.unlink()
    # Absent: no daemon, no record.
    offset = log_size()
    pid = spawn(offset)
    await_line(pid, offset, 'exit no usable authority record')
    reaped(pid)
    disarm()
    assert not any('event=accepted' in line for line in lines_for(pid, offset))
    check('absent credential: renderer refuses without a daemon record, admits nothing, exits')

    # Stale after SIGKILL: the record stays but its name has no owner.
    write_settings(11)
    first = Daemon()
    stale = RECORD.read_bytes()
    first.process.send_signal(signal.SIGKILL)
    first.process.wait(timeout=5)
    first.stop()
    assert RECORD.read_bytes() == stale, 'SIGKILL left no stale record to test'
    offset = log_size()
    pid = spawn(offset)
    await_line(pid, offset, 'stale authority record')
    reaped(pid)
    disarm()
    check('stale credential: record left by a SIGKILLed daemon is refused (name has no owner)')

    daemon = Daemon()
    genuine = RECORD.read_bytes()
    live = decode_record(genuine)
    old = decode_record(stale)
    assert live['key'] != old['key'] and live['generation'] != old['generation'] and live['name'] != old['name']
    cases = [
        ('stale record replayed while a new daemon runs', stale, 'stale authority record'),
        ('stale key on the live daemon name', encode_record(old['key'], live['generation'], live['pid'], live['name']),
         'authority refused this renderer (credential refused)'),
        ('wrong key on the live daemon name', encode_record(os.urandom(32), live['generation'], live['pid'], live['name']),
         'authority refused this renderer (credential refused)'),
        ('live name with another PID', encode_record(live['key'], live['generation'], live['pid'] + 1, live['name']),
         'authority record does not match'),
    ]
    for label, data, needle in cases:
        write_record(data)
        offset = log_size()
        pid = spawn(offset)
        await_line(pid, offset, needle)
        reaped(pid)
        disarm()
        assert not any('event=accepted' in line for line in lines_for(pid, offset)), label
        check(f'{label}: refused, nothing admitted, renderer exited')
    assert 'credential refused' in daemon.output()

    # An impostor serving the object path with its own (unknown to it: random) key.
    impostor_log = LAB / 'impostor.log'
    code = ('import os,sys; from gi.repository import Gio, GLib\n'
            'X="""<node><interface name=\'org.onemk.WallpaperAuthority1\'><method name=\'Plan\'>'
            '<arg type=\'ay\' direction=\'in\'/><arg type=\'u\' direction=\'in\'/><arg type=\'a(iiiid)\' direction=\'in\'/>'
            '<arg type=\'ay\' direction=\'in\'/><arg type=\'b\' direction=\'out\'/><arg type=\'t\' direction=\'out\'/>'
            '<arg type=\'ay\' direction=\'out\'/><arg type=\'ay\' direction=\'out\'/></method></interface></node>"""\n'
            'bus=Gio.bus_get_sync(Gio.BusType.SESSION)\n'
            'info=Gio.DBusNodeInfo.new_for_xml(X).interfaces[0]\n'
            'def call(c,s,p,i,m,a,inv): inv.return_value(GLib.Variant("(btayay)",(True,11,b"WPLN"+bytes(200),os.urandom(32))))\n'
            'bus.register_object("/org/onemk/WallpaperAuthority1",info,call,None,None)\n'
            'print("impostor",bus.get_unique_name(),os.getpid(),flush=True)\n'
            'GLib.MainLoop().run()\n')
    with impostor_log.open('w') as output:
        impostor = subprocess.Popen([str(HARNESS), 'run', 'python3', '-c', code], cwd=LAB, env=ENV,
                                    stdout=output, stderr=output)
    try:
        _, name, ipid = wait(lambda: next((line.split() for line in impostor_log.read_text().splitlines()
                                           if line.startswith('impostor ')), None), 10, what='impostor ready')
        write_record(encode_record(os.urandom(32), os.urandom(16), int(ipid), name))
        offset = log_size()
        pid = spawn(offset)
        await_line(pid, offset, 'presentation torn down: Authentication')
        reaped(pid)
        disarm()
        assert not any('event=accepted' in line for line in lines_for(pid, offset))
        check('impostor authority with a valid-looking record but no key: forged reply refused, exited')
    finally:
        impostor.terminate()
        impostor.wait(timeout=5)

    # The daemon publishes nothing to a caller that cannot prove the key.
    probe = ('import os; from gi.repository import Gio, GLib\n'
             'bus=Gio.bus_get_sync(Gio.BusType.SESSION)\n'
             'try:\n'
             f' bus.call_sync("{live["name"]}","/org/onemk/WallpaperAuthority1","org.onemk.WallpaperAuthority1","Plan",'
             'GLib.Variant("(ayua(iiiid)ay)",(os.urandom(32),1,[(0,0,1920,1080,1.0)],os.urandom(32))),None,0,2000,None)\n'
             ' print("ANSWERED")\n'
             'except GLib.Error as e: print("REFUSED", e.message)\n')
    answer = private(['python3', '-c', probe]).stdout
    assert answer.startswith('REFUSED') and 'credential refused' in answer, answer
    check('daemon refuses an unauthenticated bus caller (AccessDenied, no plan)')

    write_record(genuine)
    return daemon


def plans(daemon):
    offset = log_size()
    pid = spawn(offset)
    accepted = await_line(pid, offset, 'event=accepted sequence=1')
    assert 'paused=false active=true' in accepted and plan_fields(11) in accepted, accepted
    check(f'genuine plan admitted via Gate::accept_encoded: {accepted}')
    steps = [
        ('pause', 'event=accepted sequence=2', 'paused=true'),
        ('replay', 'event=refused reason=Replay', 'admitted_sequence=2 paused=true'),
        ('stale-sequence', 'event=refused reason=Replay', 'admitted_sequence=2 paused=true'),
        ('stale-revision', 'event=refused reason=Consent', 'admitted_sequence=2 paused=true'),
        ('oversize', 'event=refused reason=Wire', 'admitted_sequence=2 paused=true'),
        ('oversize-envelope', 'event=refused reason=Envelope', 'admitted_sequence=2 paused=true'),
        ('malformed', 'event=refused reason=Wire', 'admitted_sequence=2 paused=true'),
        ('play', 'event=accepted sequence=3', 'paused=false active=true'),
    ]
    for directive, needle, expect in steps:
        seen = len(lines_for(pid, offset))
        daemon.directive(directive)
        line = wait(lambda: next((line for line in lines_for(pid, offset)[seen:] if needle in line), None),
                    5, what=f'{directive} outcome')
        assert expect in line, (directive, line)
        assert not exited(pid)
        check(f'{directive}: {line}')
    seen = len(lines_for(pid, offset))
    evaluate('Main.layoutManager.emit("monitors-changed"); true')
    line = wait(lambda: next((line for line in lines_for(pid, offset)[seen:] if 'event=accepted sequence=4' in line), None),
                5, what='fresh plan after layout change')
    assert any(line.startswith('event=layout serial=') and line.endswith('admitted_sequence=none active=false')
               for line in lines_for(pid, offset)[seen:]), lines_for(pid, offset)[seen:]
    check(f'layout change discards the admitted plan until a fresh one: {line}')
    start = time.monotonic()
    daemon.directive('forge')
    elapsed = await_exit(pid, start)
    await_line(pid, offset, 'presentation torn down: Authentication')
    reaped(pid)
    disarm()
    check(f'mid-session forged reply: presentation torn down, renderer exited in {elapsed * 1000:.0f} ms')


TEARDOWNS = ['revoke', 'disable', 'stop', 'sigterm', 'sigkill']
EXPECTED = {
    'revoke': 'presentation torn down: Revoked', 'disable': 'presentation torn down: Revoked',
    'stop': 'authority connection lost', 'sigterm': 'authority connection lost',
    'sigkill': 'authority connection lost',
}


def teardown(daemon, method):
    if method == 'revoke':
        private([BIN / 'one-mk', 'wallpaper', 'consent', 'revoke', PEER])
    elif method == 'disable':
        private([BIN / 'one-mk', 'wallpaper', 'disable'])
    elif method == 'stop':
        private([BIN / 'one-mk', 'stop'])
    elif method == 'sigterm':
        daemon.process.send_signal(signal.SIGTERM)
    elif method == 'sigkill':
        daemon.process.send_signal(signal.SIGKILL)


def cycle(index, method, latencies, baseline):
    write_settings(1000 + index)
    daemon = Daemon()
    offset = log_size()
    try:
        pid = spawn(offset)
        accepted = await_line(pid, offset, 'event=accepted sequence=1')
        assert 'paused=false active=true' in accepted and plan_fields(1000 + index) in accepted, accepted
        start = time.monotonic()
        teardown(daemon, method)
        latencies.setdefault(method, []).append(await_exit(pid, start))
        await_line(pid, offset, EXPECTED[method])
        reaped(pid)
        disarm()
        if method in ('stop', 'sigterm', 'sigkill'):
            daemon.process.wait(timeout=10)
            if method != 'sigkill':
                assert not RECORD.exists(), 'clean daemon exit left its record'
    finally:
        daemon.stop()
    for renderer in renderers_since(offset):
        reaped(renderer)
    assert state(daemon.pid) is None
    leftovers = {pid: comm for pid, comm in lab_processes().items() if pid not in baseline}
    assert not leftovers, f'cycle {index}: leftover processes {leftovers}'
    evaluate('Main.extensionManager.lookup("spike-harness@one-mk.test").stateObj.collectGarbage(); true')
    return renderers_since(offset)


def teardown_proofs(baseline):
    latencies = {}
    for index, method in enumerate(TEARDOWNS):
        write_settings(500 + index)
        daemon = Daemon()
        offset = log_size()
        try:
            pid = spawn(offset)
            await_line(pid, offset, 'event=accepted sequence=1')
            start = time.monotonic()
            teardown(daemon, method)
            elapsed = await_exit(pid, start)
            await_line(pid, offset, EXPECTED[method])
            reaped(pid)
            # The Shell restarts the renderer (still authorized in this lab); the
            # fresh renderer must also refuse while consent is gone / daemon dead.
            restart = wait(lambda: [p for p in renderers_since(offset) if p != pid][:1], 6,
                           what='Shell restart attempt')[0]
            refusal = 'presentation torn down: Revoked' if method in ('revoke', 'disable') else 'no usable authority record' \
                if method != 'sigkill' else 'stale authority record'
            await_line(restart, offset, refusal)
            reaped(restart)
            disarm()
            assert not any('event=accepted' in line for line in lines_for(restart, offset))
        finally:
            daemon.stop()
        for renderer in renderers_since(offset):
            reaped(renderer)
        leftovers = {pid: comm for pid, comm in lab_processes().items() if pid not in baseline}
        assert not leftovers, leftovers
        latencies[method] = elapsed
        check(f'{method}: renderer exited {elapsed * 1000:.0f} ms after the action ({EXPECTED[method]}); '
              f'Shell restart refused ({refusal}); no leftover processes')
    return latencies


def main():
    LAB.mkdir(mode=0o700)
    (LAB / 'home').mkdir()
    sources = LAB / 'extensions'
    sources.mkdir()
    destination = sources / UUID
    shutil.copytree(REPO / 'extensions/gnome' / UUID, destination)
    shutil.copytree(HERE / 'ext/spike-harness@one-mk.test', sources / 'spike-harness@one-mk.test')
    (destination / 'libexec').mkdir()
    wrapper = destination / 'libexec/one-mk-wall'
    wrapper.write_text(f'#!/bin/sh\nexec >>"{RENDERER_LOG}" 2>&1\nexec "{BIN / "one-mk-wall"}" --present=gnome --m1-lab\n')
    wrapper.chmod(0o700)
    ENV['EXT_SRC'] = str(sources)
    started = False
    try:
        REPORT['shell'] = run(['gnome-shell', '--version']).stdout.strip()
        assert run([BIN / 'one-mk-wall', '--present=gnome'], check=False).returncode == 3
        run([HARNESS, 'start', '2'])
        started = True
        extension('Enable', 'spike-harness@one-mk.test')
        evaluate('Main.overview.hide(); true')
        extension('Enable')
        wait(lambda: evaluate(f'Boolean({BRIDGE}._ownsName && {BRIDGE}._ready)'), 5, 0.1, 'bridge ready')
        assert evaluate(f'{BRIDGE}._pid === null && {BRIDGE}._authorized === false')
        check('production extension dormant and unauthorized by default; renderer without --m1-lab refuses')
        daemon = authentication()
        plans(daemon)
        daemon.stop()
        assert not RECORD.exists()
        baseline = lab_processes()
        REPORT['baseline_processes'] = sorted(set(baseline.values()))
        REPORT['teardown_latency_ms'] = {key: round(value * 1000, 1) for key, value in teardown_proofs(baseline).items()}
        latencies = {}
        renderers = 0
        for index in range(CYCLES):
            renderers += len(cycle(index, TEARDOWNS[index % len(TEARDOWNS)], latencies, baseline))
            if index % 10 == 9:
                print(f'cycle {index + 1}/{CYCLES}', flush=True)
        REPORT['cycles'] = CYCLES
        REPORT['cycle_renderer_processes'] = renderers
        REPORT['cycle_exit_latency_ms'] = {
            method: {'count': len(values), 'median': round(statistics.median(values) * 1000, 1),
                     'max': round(max(values) * 1000, 1)} for method, values in latencies.items()}
        assert all(max(values) <= EXIT_BOUND for values in latencies.values())
        check(f'{CYCLES} real daemon+renderer start/authenticate/admit/teardown cycles '
              f'(rotating {", ".join(TEARDOWNS)}), every renderer and daemon reaped, no leftover processes')
        extension('Disable')
        log = (SESSION / 'log/shell.log').read_text()
        gate = runpy.run_path(str(HERE / 'm1-risk-probe.py'))
        baseline_log = (HERE / 'm1a2-evidence/baseline-shell.log').read_text()
        REPORT['new_warnings'] = gate['new_warnings'](log, baseline_log)
        REPORT['warning_records'] = len(gate['warning_records'](log))
        assert not REPORT['new_warnings'], REPORT['new_warnings']
        check(f'zero new Mutter/Shell warning records versus the M1a-2 baseline ({REPORT["warning_records"]} total, all known)')
        REPORT['passed'] = True
    except BaseException as error:
        REPORT['passed'] = False
        REPORT['failure'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        if (SESSION / 'log/shell.log').exists():
            shutil.copy2(SESSION / 'log/shell.log', LAB / 'shell.log')
        if started:
            run([HARNESS, 'stop'], check=False)
        REPORT['remaining_lab_processes'] = lab_processes()
        shutil.rmtree(sources, ignore_errors=True)
        (LAB / 'result.json').write_text(json.dumps(REPORT, indent=2) + '\n')
        print(json.dumps({key: value for key, value in REPORT.items() if key != 'checks'}, indent=2))


if __name__ == '__main__':
    sys.exit(main())
