#!/usr/bin/env python3
"""6D-M1 step B: real playback, exclusively inside the private Mutter harness.

Real processes only. Locally generated media are imported by the real
importer (`one-mk wallpaper import`, bubblewrap/seccomp/cgroup sandbox). The
null-backend laboratory daemon publishes an authenticated plan naming one
imported item on a 5760x1080 canvas whose left third belongs to an absent
computer; the private GNOME Shell spawns the laboratory renderer, which plays
the item through the sandboxed playback worker on its own xdg surface. Shell
screenshots are compared with exact expected crops of the source frames.

Environment (all required unless a default is shown):
  M1_BIN      directory with laboratory `one-mk` (wallpaper-m1-lab),
              `one-mk-wall` (m1-lab) and `one-mk-wall-import`
  M1_LAB      a short, new private directory (Unix socket paths must fit)
  M1_TRUSTED  an existing directory whose path contains `target` and whose
              ancestors are not group/world writable (the audited runtime)
  M1_CGROUP   an explicitly delegated, empty cgroup v2 directory with memory
              and pids (and optionally cpu) in cgroup.subtree_control; the
              script creates `import` and `play` inside and removes them
  M1_CYCLES   real-playback start/stop cycles (default 100)
  M1_MEASURE  seconds of 1080p30 resource measurement (default 30)

Never connects to the owner's desktop, X servers, clipboard or session bus.
Production authorization stays false: playback needs both laboratory features
and private Eval authorization of this harness's bridge.
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

from PIL import Image, ImageChops, ImageStat

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import m1_playback_media as media  # noqa: E402

REPO = HERE.parents[2]
BIN = Path(os.environ['M1_BIN']).resolve()
LAB = Path(os.environ['M1_LAB']).resolve()
TRUSTED = Path(os.environ['M1_TRUSTED']).resolve()
CGROUP = Path(os.environ['M1_CGROUP'])
CYCLES = int(os.environ.get('M1_CYCLES', '100'))
MEASURE = int(os.environ.get('M1_MEASURE', '30'))
SESSION = LAB / 's'
HOME = SESSION / 'one-mk'
RECORD = SESSION / 'run/one-mk-wallpaper-m1/authority'
RENDERER_LOG = LAB / 'renderer.log'
SHOTS = LAB / 'shots'
LIBRARY = LAB / 'library'
UUID = 'one-mk-wallpaper@one-mk.org'
HARNESS = HERE / 'isolated-session.sh'
BRIDGE = f'Main.extensionManager.lookup("{UUID}").stateObj.bridge'
PEER = 'ab' * 32
CANVAS = [-1920, 0, 5760, 1080]
MONITORS = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)]
SLICES = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)]
ABSENT = (-1920, 0, 1920, 1080)
# The top bar sits on the primary monitor; compare rows below it on both.
PANEL_ROWS = 64
EXIT_BOUND = 2.0
ENV = {'PATH': '/usr/bin:/bin', 'HOME': str(LAB / 'home'),
       'USER': pwd.getpwuid(os.getuid()).pw_name, 'LANG': 'C.UTF-8',
       'SPIKE': str(SESSION), 'FAKE_GDM': '1', 'PYTHONDONTWRITEBYTECODE': '1'}
REPORT = {'checks': [], 'production_authorization': False, 'media': {}, 'screenshots': {}}

# name: (file, kind, source size, frames, lossless reference available)
ITEMS = {
    'gif': {'frames': 10, 'size': (576, 108), 'lossless': True},
    'apng': {'frames': 10, 'size': (576, 108), 'lossless': True},
    # The image crate composites animated-WebP frames after the first with
    # at most one level of rounding versus libwebp (checked independently):
    # a decoder property, not presentation, so WebP allows that one level.
    'webp': {'frames': 10, 'size': (576, 108), 'lossless': True, 'tolerance': 1},
    'still': {'frames': 1, 'size': (1152, 216), 'lossless': True},
    'vp9': {'frames': 30, 'size': (960, 180), 'lossless': False},
    'av1': {'frames': 30, 'size': (960, 180), 'lossless': False},
    'clip1080p': {'frames': 240, 'size': (1920, 1080), 'lossless': None},
}


def run(arguments, check=True, timeout=60, env=None):
    return subprocess.run([str(value) for value in arguments], env=env or ENV, cwd=LAB,
                          text=True, capture_output=True, timeout=timeout, check=check)


def private(arguments, check=True, timeout=60):
    return run([HARNESS, 'run', *arguments], check, timeout)


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
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError, IndexError):
        return None


def exited(pid):
    return state(pid) in (None, 'Z', 'X')


def lab_processes():
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


def play_groups():
    """Playback worker groups (the sandboxed decoders and their bubblewrap)."""
    groups = {}
    for entry in (CGROUP / 'play').iterdir():
        if entry.is_dir() and entry.name.startswith('one-mk-play-'):
            groups[entry.name] = [int(pid) for pid in (entry / 'cgroup.procs').read_text().split()]
    return groups


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


def cpu_ticks(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return int(fields[11]) + int(fields[12])


def status_kib(pid, name):
    for line in Path(f'/proc/{pid}/status').read_text().splitlines():
        if line.startswith(name + ':'):
            return int(line.split()[1])
    return 0


# ---------------------------------------------------------------- daemon

def write_settings(revision, enabled=True):
    HOME.mkdir(mode=0o700, exist_ok=True)
    (HOME / 'settings.toml').write_text(
        f'[wallpaper]\nversion = 1\nenabled = {"true" if enabled else "false"}\n\n'
        f'[wallpaper.grants]\n"{PEER}" = {revision}\n')


def decode_record(data):
    assert data[:5] == b'WAUT\x01', data[:5]
    pid = int.from_bytes(data[53:57], 'little')
    return {'pid': pid, 'name': data[58:58 + data[57]].decode()}


class Daemon:
    count = 0

    def __init__(self, item):
        Daemon.count += 1
        self.log_path = LAB / 'daemon.log'
        self.offset = self.log_path.stat().st_size if self.log_path.exists() else 0
        read, write = os.pipe()
        config = {'peer': PEER, 'media': REPORT['media'][item]['hash'], 'canvas': CANVAS, 'origin': [0, 0]}
        os.write(write, json.dumps(config).encode())
        os.close(write)
        self.log = self.log_path.open('a')
        self.log.write(f'--- daemon start {Daemon.count} ({item})\n')
        self.log.flush()
        self.process = subprocess.Popen(
            [str(HARNESS), 'run', 'env', f'ONE_MK_M1_LAB_FD={read}', str(BIN / 'one-mk'), 'run',
             '--backend', 'null', '--no-discovery', '--no-notify', '--no-clipboard', '--port', str(free_port())],
            env=ENV, cwd=LAB, stdout=self.log, stderr=self.log, pass_fds=(read,))
        os.close(read)
        self.pid = self.process.pid
        wait(self._ready, 15, what='daemon authority record')

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
    """Private Eval laboratory authorization only (never a production path).
    The first-frame and damage watchdogs stay in force: frames are real."""
    evaluate(f'(() => {{ const b = {BRIDGE}; b._recovery.attempts = []; b._recovery.exhausted = false; '
             f'b._authorized = true; b._coverage(); return true; }})()')


def disarm():
    evaluate(f'(() => {{ const b = {BRIDGE}; b._authorized = false; b._stop(); return true; }})()')
    wait(lambda: evaluate(f'{BRIDGE}._recovery.children.size') == 0, 5, 0.05, 'Shell child reaped')


def spawn(offset, timeout=8):
    arm()
    return wait(lambda: renderers_since(offset)[:1], timeout, what='renderer start')[0]


def await_line(pid, offset, needle, timeout=8):
    return wait(lambda: next((line for line in lines_for(pid, offset) if needle in line), None),
                timeout, what=f'renderer {pid}: {needle}')


def await_exit(pid, start, timeout=EXIT_BOUND):
    wait(lambda: exited(pid), timeout, 0.005, f'renderer {pid} exit')
    return time.monotonic() - start


def reaped(pid, timeout=5):
    wait(lambda: state(pid) is None, timeout, 0.02, f'renderer {pid} reaped')


def shell_view():
    return evaluate(f'(() => {{ const b = {BRIDGE}; return {{state: b._state, layer: b._layer.visible, '
                    f'fresh: b._renderer?.fresh === true, reasons: b._reasons, clones: b._clones.length, '
                    f'rect: b._renderer ? [b._renderer.window.get_buffer_rect().x, b._renderer.window.get_buffer_rect().y, '
                    f'b._renderer.window.get_buffer_rect().width, b._renderer.window.get_buffer_rect().height] : null, '
                    f'type: b._renderer?.window.get_window_type() ?? null}}; }})()')


def playing(pid, offset):
    line = await_line(pid, offset, 'event=playing', 8)
    view = wait(lambda: (lambda value: value if value['layer'] and value['fresh'] and value['state'] in (1, 4) else None)(shell_view()),
                3, 0.05, 'Shell reveals the presentation')
    assert view['rect'] == [0, 0, 3840, 1080] and view['clones'] == 2 and view['reasons'] == 0, view
    return line, view


# ---------------------------------------------------------------- screenshots

SHOT_COUNT = [0]


def screenshot(label):
    SHOT_COUNT[0] += 1
    path = SHOTS / f'{SHOT_COUNT[0]:04d}-{label}.png'
    before = time.monotonic()
    private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Screenshot', '-o', '/org/gnome/Shell/Screenshot',
             '-m', 'org.gnome.Shell.Screenshot.Screenshot', 'false', 'false', path])
    after = time.monotonic()
    image = Image.open(path).convert('RGB')
    assert image.size == (3840, 1080), image.size
    return image, (before + after) / 2, path


def sample(position, canvas, source, numerator, denominator):
    value = ((2 * position + 1) * denominator - (canvas * denominator - source * numerator)) // (2 * numerator)
    return min(max(value, 0), source - 1)


def mapping(size, slice_rect):
    """Per-axis (source index, centre output offset) pairs, as the renderer maps."""
    source_width, source_height = size
    canvas_x, canvas_y, canvas_width, canvas_height = CANVAS
    if canvas_width * source_height >= canvas_height * source_width:
        numerator, denominator = canvas_width, source_width
    else:
        numerator, denominator = canvas_height, source_height
    x, y, width, height = slice_rect

    def axis(origin, length, canvas, source):
        runs = {}
        for offset in range(length):
            runs.setdefault(sample(origin + offset, canvas, source, numerator, denominator), []).append(offset)
        return sorted((index, members[len(members) // 2], members) for index, members in runs.items())

    return (axis(x - canvas_x, width, canvas_width, source_width), axis(y - canvas_y, height, canvas_height, source_height))


def sampled(shot, monitor, columns, rows):
    """The screenshot at each source pixel's centre output pixel (below the panel)."""
    left = MONITORS[monitor][0]
    pixels = shot.load()
    kept_rows = [row for row in rows if row[1] >= PANEL_ROWS]
    data = bytearray()
    for _, centre_y, _ in kept_rows:
        for _, centre_x, _ in columns:
            data.extend(pixels[left + centre_x, centre_y])
    return Image.frombytes('RGB', (len(columns), len(kept_rows)), bytes(data)), kept_rows


def mean_error(first, second):
    return sum(ImageStat.Stat(ImageChops.difference(first, second)).mean) / 3


def reference(item, index):
    width, height = ITEMS[item]['size']
    if item == 'still':
        return media.still(width, height)
    return media.pattern(width, height, index, ITEMS[item]['frames'])


def analyse(item, shot):
    """Identifies the frame each monitor shows and proves it is that monitor's
    exact slice of the shared canvas, never the reserved absent slice."""
    size = ITEMS[item]['size']
    frames = ITEMS[item]['frames']
    result = {'monitors': []}
    absent_columns, _ = mapping(size, ABSENT)
    for monitor in range(2):
        columns, rows = mapping(size, SLICES[monitor])
        observed, kept_rows = sampled(shot, monitor, columns, rows)
        box = (columns[0][0], kept_rows[0][0], columns[-1][0] + 1, kept_rows[-1][0] + 1)
        errors = [mean_error(observed, reference(item, index).crop(box)) for index in range(frames)]
        best = min(range(frames), key=errors.__getitem__)
        absent_box = (absent_columns[0][0], box[1], absent_columns[-1][0] + 1, box[3])
        absent_error = mean_error(observed, reference(item, best).crop(absent_box))
        other = SLICES[1 - monitor]
        other_columns, _ = mapping(size, other)
        other_box = (other_columns[0][0], box[1], other_columns[-1][0] + 1, box[3])
        swapped_error = mean_error(observed, reference(item, best).crop(other_box))
        entry = {'frame': best, 'error': round(errors[best], 3), 'absent_slice_error': round(absent_error, 1),
                 'other_slice_error': round(swapped_error, 1), 'source_box': box,
                 'runner_up_error': round(sorted(errors)[1], 3) if frames > 1 else None}
        if ITEMS[item]['lossless']:
            # Every output pixel below the panel, not just block centres.
            source = reference(item, best)
            raw = source.tobytes()
            source_width = source.size[0]
            row_index = {offset: index for index, _, members in rows for offset in members}
            column_values = [index for index, _, members in columns for _ in members]
            lines = {}
            data = bytearray()
            for offset in range(PANEL_ROWS, 1080):
                index = row_index[offset]
                if index not in lines:
                    base = index * source_width * 3
                    lines[index] = b''.join(raw[base + column * 3:base + column * 3 + 3] for column in column_values)
                data.extend(lines[index])
            expected = Image.frombytes('RGB', (1920, 1080 - PANEL_ROWS), bytes(data))
            left = MONITORS[monitor][0]
            actual = shot.crop((left, PANEL_ROWS, left + 1920, 1080))
            difference = ImageChops.difference(actual, expected)
            entry['max_channel_error'] = max(high for _, high in difference.getextrema())
            entry['exact'] = entry['max_channel_error'] <= ITEMS[item].get('tolerance', 0)
        else:
            entry['psnr_db'] = psnr(observed, reference(item, best).crop(box))
        result['monitors'].append(entry)
    # Any red-dominant pixel would mean the absent computer's slice is drawn here.
    red = 0
    probe = shot.crop((0, PANEL_ROWS, 3840, 1080)).resize((384, 101), Image.NEAREST)
    for r, g, b in probe.getdata():
        if r >= 40 and r > g + 25 and r > b + 25 and not (r > 200 and g > 200):
            red += 1
    result['red_dominant_samples'] = red
    return result


def psnr(first, second):
    import math
    difference = ImageChops.difference(first, second)
    squares = ImageStat.Stat(difference).rms
    mse = sum(value * value for value in squares) / 3
    return round(99.0 if mse == 0 else 10 * math.log10(255 * 255 / mse), 1)


def assert_slices(item, analysis):
    first, second = analysis['monitors']
    assert first['frame'] == second['frame'], analysis
    lossless = ITEMS[item]['lossless']
    for entry in analysis['monitors']:
        if lossless:
            assert entry['exact'] and entry['error'] <= ITEMS[item].get('tolerance', 0), (item, entry)
        else:
            assert entry['error'] < 6 and entry['psnr_db'] >= 30, (item, entry)
        assert entry['absent_slice_error'] > 20 and entry['other_slice_error'] > 4, (item, entry)
        if entry['runner_up_error'] is not None:
            assert entry['runner_up_error'] > entry['error'] + 3, (item, entry)
    assert analysis['red_dominant_samples'] == 0, analysis


def background_like(shot, baseline):
    """The private Shell's native background (nothing of ours visible)."""
    region = (0, PANEL_ROWS, 3840, 1080)
    return mean_error(shot.crop(region), baseline.crop(region))


# ---------------------------------------------------------------- setup

def prepare_cgroups():
    controllers = (CGROUP / 'cgroup.subtree_control').read_text().split()
    assert {'memory', 'pids'} <= set(controllers), controllers
    assert not (CGROUP / 'cgroup.procs').read_text().split(), 'delegation root must be empty'
    for name in ('import', 'play'):
        group = CGROUP / name
        group.mkdir(mode=0o755)
        os.chmod(group, 0o755)
        (group / 'cgroup.subtree_control').write_text(
            ' '.join(f'+{controller}' for controller in controllers if controller in ('memory', 'pids', 'cpu')))
    REPORT['cgroup_controllers'] = controllers


def remove_cgroups():
    for name in ('play', 'import'):
        group = CGROUP / name
        if group.exists():
            for child in group.iterdir():
                if child.is_dir():
                    child.rmdir()
            group.rmdir()


def prepare_runtime():
    runtime = TRUSTED / 'runtime'
    if not runtime.exists():
        run(['python3', '-I', REPO / 'scripts/wallpaper-lab-runtime.py', runtime, '--worker', BIN / 'one-mk-wall-import'])
    return runtime


def generate_media():
    directory = LAB / 'media'
    directory.mkdir()
    paths = media.save_images(directory, 'pattern', *ITEMS['gif']['size'], ITEMS['gif']['frames'], 100)
    files = {'gif': paths['gif'], 'apng': paths['png'], 'webp': paths['webp'],
             'still': media.save_still(directory, *ITEMS['still']['size']),
             'vp9': media.encode_pattern_video(directory / 'pattern-vp9.webm', *ITEMS['vp9']['size'], 30, 30, 'vp9'),
             'av1': media.encode_pattern_video(directory / 'pattern-av1.webm', *ITEMS['av1']['size'], 30, 30, 'av1'),
             'clip1080p': media.encode_clip_1080p(directory / 'clip-1080p30.webm')}
    # The lossless encodings must decode to exactly the generated frames.
    for item in ('gif', 'apng', 'webp'):
        frames = media.decoded_frames(files[item])
        assert len(frames) == ITEMS[item]['frames']
        assert all(frame.tobytes() == reference(item, index).tobytes() for index, frame in enumerate(frames)), item
    assert media.decoded_frames(files['still'])[0].tobytes() == reference('still', 0).tobytes()
    return files


def import_media(files, runtime):
    LIBRARY.mkdir(mode=0o700)
    for item, path in files.items():
        result = run([BIN / 'one-mk', 'wallpaper', 'import', path, '--runtime', runtime, '--output', LIBRARY,
                      '--cgroup-root', CGROUP / 'import', '--json'], check=False, timeout=600)
        assert result.returncode == 0, (item, result.stdout, result.stderr)
        record = json.loads(result.stdout)
        expected = {'gif': 'gif', 'apng': 'apng', 'webp': 'webp', 'still': 'still_png', 'vp9': 'vp9_webm',
                    'av1': 'av1_webm', 'clip1080p': 'vp9_webm'}[item]
        assert record['media']['format'] == expected, record
        assert [record['media']['width'], record['media']['height']] == list(ITEMS[item]['size']), record
        assert record['media']['frames'] == ITEMS[item]['frames'], record
        REPORT['media'][item] = {'hash': record['source_blake3'], 'bytes': record['source_bytes'], **record['media']}
    assert not [entry for entry in (CGROUP / 'import').iterdir() if entry.is_dir()], 'import cgroup left behind'
    check('all seven generated items imported by the real sandboxed importer (one-mk wallpaper import, '
          'bubblewrap/seccomp, delegated cgroup v2): ' + ', '.join(f'{item}={value["format"]} {value["width"]}x{value["height"]}'
                                                                     f'/{value["frames"]}f' for item, value in REPORT['media'].items()))


def write_playback_config(runtime):
    worker = BIN / 'one-mk-wall-import'
    config = LAB / 'playback.json'
    config.write_text(json.dumps({'library': str(LIBRARY), 'runtime': str(runtime),
                                  'cgroup_root': str(CGROUP / 'play'), 'worker': str(worker)}))
    config.chmod(0o600)
    return config


# ---------------------------------------------------------------- scenarios

def seams(pid, offset):
    values = []
    for line in lines_for(pid, offset):
        if line.startswith('event=loop '):
            fields = dict(part.split('=', 1) for part in line.split()[1:])
            values.append({key: int(value) for key, value in fields.items()})
    return values


def scenario(item, baseline):
    write_settings(200 + list(ITEMS).index(item))
    daemon = Daemon(item)
    offset = log_size()
    record = {}
    try:
        pid = spawn(offset)
        line, view = playing(pid, offset)
        start_line = await_line(pid, offset, 'event=playback-start')
        record['start'] = start_line
        record['first_frame'] = await_line(pid, offset, 'event=first-frame')
        frames = ITEMS[item]['frames']
        if item == 'clip1080p':
            record.update(clip(pid, offset, baseline))
        else:
            series = 1 if item == 'still' else 10
            captured = []
            for index in range(series):
                before = time.monotonic()
                shot, moment, path = screenshot(f'{item}-{index}')
                captured.append((shot, before, time.monotonic(), path))
                time.sleep(0.05 + 0.037 * index)
            shots = []
            for shot, before, after, path in captured:
                analysis = analyse(item, shot)
                assert_slices(item, analysis)
                shots.append({'before': round(before, 3), 'after': round(after, 3), 'frame': analysis['monitors'][0]['frame'],
                              'path': path.name, 'monitors': analysis['monitors'],
                              'red_dominant_samples': analysis['red_dominant_samples']})
            record['shots'] = shots
            indices = [shot['frame'] for shot in shots]
            if item != 'still':
                # Real-time progress: each step between screenshots advanced by
                # as many frames as the elapsed time allows (modulo the loop).
                duration = 1.0 / (ITEMS[item]['frames'])
                for first, second in zip(shots, shots[1:]):
                    delta = (second['frame'] - first['frame']) % frames
                    low = int((second['before'] - first['after']) / duration) - 1
                    high = int((second['after'] - first['before']) / duration) + 2
                    assert any(low <= delta + loop * frames <= high for loop in range(0, 50)), (first, second, low, high)
            if item == 'still':
                # Longer than the Shell's 3 s damage watchdog: the still must
                # stay, unchanged, with no decoder running.
                time.sleep(4.5)
                again, _, path = screenshot(f'{item}-later')
                assert ImageChops.difference(again, Image.open(SHOTS / shots[0]['path']).convert('RGB')).crop(
                    (0, PANEL_ROWS, 3840, 1080)).getbbox() is None
                assert any('event=still-presented worker=stopped cleanup=ok' in line for line in lines_for(pid, offset))
                assert not any(group for group in play_groups().values()), play_groups()
                view = shell_view()
                assert not exited(pid) and view['layer'] and view['state'] == 4, view  # holding (step C)
                record['stable_after_s'] = 4.5
                check(f'still PNG: both monitors show their exact slices (every pixel below the panel), identical '
                      f'4.5 s later and still revealed, decode worker gone right after the one frame')
            else:
                assert len(set(indices)) >= 4, indices
                wraps = sum(1 for previous, current in zip(indices, indices[1:]) if current < previous)
                assert wraps >= 1, indices
                record['frame_sequence'] = indices
                time.sleep(max(0.0, 2.6 - (time.monotonic() - shots[0]['before'])))
                loop_seams = seams(pid, offset)
                assert len(loop_seams) >= 2, loop_seams
                nominal = loop_seams[0]['nominal_us']
                worst = max(seam['seam_us'] for seam in loop_seams)
                assert worst <= nominal + 17_000, loop_seams
                record['seams_us'] = [seam['seam_us'] for seam in loop_seams]
                record['nominal_us'] = nominal
                errors = [round(entry['error'], 2) for shot in shots for entry in shot['monitors']]
                kind = ('exact (every pixel below the panel)' if not ITEMS[item].get('tolerance') else
                        f'every pixel below the panel within {ITEMS[item]["tolerance"]} level (decoder rounding)') \
                    if ITEMS[item]['lossless'] else \
                    f'PSNR >= {min(entry["psnr_db"] for shot in shots for entry in shot["monitors"])} dB'
                check(f'{item}: {len(shots)} screenshots, both monitors show the same frame as its own slice, {kind}; '
                      f'frames advance {indices} and wrap; reserved absent slice never drawn; '
                      f'loop seams {record["seams_us"]} us vs nominal {nominal} us (max error {max(errors)})')
        start = time.monotonic()
        private([BIN / 'one-mk', 'stop'])
        elapsed = await_exit(pid, start)
        await_line(pid, offset, 'authority connection lost')
        reaped(pid)
        disarm()
        record['exit_ms'] = round(elapsed * 1000, 1)
        assert not play_groups(), play_groups()
    finally:
        daemon.stop()
    REPORT['screenshots'][item] = record
    return record


def clip(pid, offset, baseline):
    """1080p30 VP9: frames change, both monitors show media, and resources."""
    result = {}
    shots = []
    for index in range(4):
        shot, moment, path = screenshot(f'clip1080p-{index}')
        shots.append(shot)
        assert background_like(shot, baseline) > 20
        time.sleep(0.4)
    changes = [round(mean_error(first.crop((0, PANEL_ROWS, 3840, 1080)), second.crop((0, PANEL_ROWS, 3840, 1080))), 2)
               for first, second in zip(shots, shots[1:])]
    assert all(change > 0.5 for change in changes), changes
    # Seam continuity: the two monitors' facing columns are adjacent canvas
    # columns of the same frame.
    seam = mean_error(shots[0].crop((1919, PANEL_ROWS, 1920, 1080)), shots[0].crop((1920, PANEL_ROWS, 1921, 1080)))
    far = mean_error(shots[0].crop((0, PANEL_ROWS, 1, 1080)), shots[0].crop((1920, PANEL_ROWS, 1921, 1080)))
    result['frame_changes'] = changes
    result['seam_column_error'] = round(seam, 2)
    result['far_column_error'] = round(far, 2)
    result['resources'] = measure(pid, offset)
    check(f'1080p30 VP9 clip: frames change between screenshots {changes}; seam columns continuous '
          f'({result["seam_column_error"]} vs {result["far_column_error"]} for unrelated columns); '
          f'resources {json.dumps(result["resources"]["summary"])}')
    return result


def worker_pids():
    pids = []
    for members in play_groups().values():
        pids.extend(members)
    return pids


def shell_pid():
    return next(pid for pid, comm in lab_processes().items() if comm == 'gnome-shell')


def group_dir():
    names = list(play_groups())
    assert len(names) == 1, names
    return CGROUP / 'play' / names[0]


def measure(pid, offset):
    time.sleep(3)
    hertz = os.sysconf('SC_CLK_TCK')
    group = group_dir()
    shell = shell_pid()
    def stat(name):
        return int(next(line.split()[1] for line in (group / 'cpu.stat').read_text().splitlines() if line.split()[0] == name))

    def usage():
        return stat('usage_usec')
    start = time.monotonic()
    before = {'renderer': cpu_ticks(pid), 'shell': cpu_ticks(shell), 'worker_usec': usage(), 'throttled': stat('nr_throttled')}
    late_before = seams(pid, offset)[-1] if seams(pid, offset) else {}
    seams_before = len(seams(pid, offset))
    samples = []
    while time.monotonic() - start < MEASURE:
        samples.append({'renderer_rss': status_kib(pid, 'VmRSS'), 'renderer_anon': status_kib(pid, 'RssAnon'),
                        'renderer_shmem': status_kib(pid, 'RssShmem'), 'renderer_file': status_kib(pid, 'RssFile'),
                        'worker_rss': sum(status_kib(member, 'VmRSS') for member in worker_pids()),
                        'worker_cgroup': int((group / 'memory.current').read_text()) // 1024})
        time.sleep(1)
    elapsed = time.monotonic() - start
    after = {'renderer': cpu_ticks(pid), 'shell': cpu_ticks(shell), 'worker_usec': usage(), 'throttled': stat('nr_throttled')}
    loop_seams = seams(pid, offset)[seams_before:]
    last = loop_seams[-1] if loop_seams else {}
    peak = int((group / 'memory.peak').read_text()) // 1024
    summary = {
        'seconds': round(elapsed, 1),
        'renderer_cpu_percent_of_one_core': round(100 * (after['renderer'] - before['renderer']) / hertz / elapsed, 1),
        'worker_cpu_percent_of_one_core': round(100 * (after['worker_usec'] - before['worker_usec']) / 1e6 / elapsed, 1),
        'gnome_shell_cpu_percent_of_one_core': round(100 * (after['shell'] - before['shell']) / hertz / elapsed, 1),
        'renderer_rss_mib_max': round(max(sample['renderer_rss'] for sample in samples) / 1024, 1),
        'renderer_rss_mib_mean': round(statistics.mean(sample['renderer_rss'] for sample in samples) / 1024, 1),
        'renderer_shm_mib_max': round(max(sample['renderer_shmem'] for sample in samples) / 1024, 1),
        'renderer_anon_mib_max': round(max(sample['renderer_anon'] for sample in samples) / 1024, 1),
        'worker_rss_mib_max': round(max(sample['worker_rss'] for sample in samples) / 1024, 1),
        'worker_cgroup_mib_max': round(max(sample['worker_cgroup'] for sample in samples) / 1024, 1),
        'worker_cgroup_peak_mib': round(peak / 1024, 1),
        'renderer_vmhwm_mib': round(status_kib(pid, 'VmHWM') / 1024, 1),
        'loops_in_window': len(loop_seams),
        'presented_in_window': last.get('presented', 0) - late_before.get('presented', 0),
        'late_in_window': last.get('late', 0) - late_before.get('late', 0),
        'starved_in_window': last.get('starved', 0) - late_before.get('starved', 0),
        'late_total': last.get('late'), 'starved_total': last.get('starved'), 'presented_total': last.get('presented'),
        'worker_cpu_max': (group / 'cpu.max').read_text().strip(),
        'worker_throttled_periods': after['throttled'] - before['throttled'],
        'seams_us': [seam['seam_us'] for seam in loop_seams],
    }
    return {'summary': summary, 'samples': samples}


def pause_proof(baseline):
    item = 'gif'
    write_settings(300)
    daemon = Daemon(item)
    offset = log_size()
    try:
        pid = spawn(offset)
        playing(pid, offset)
        shot, _, _ = screenshot('pause-before')
        assert_slices(item, analyse(item, shot))
        assert len(play_groups()) == 1
        seen = len(lines_for(pid, offset))
        daemon.directive('pause')
        stopped = wait(lambda: next((line for line in lines_for(pid, offset)[seen:] if line.startswith('event=stopped')), None),
                       3, what='renderer stops on the paused plan')
        assert 'paused=true' in stopped and 'active=false' in stopped, stopped
        view = wait(lambda: (lambda value: value if value['state'] == 2 and not value['layer'] and value['rect'] is None
                             else None)(shell_view()), 2, 0.05, 'Shell hides the presentation, surface destroyed')
        wait(lambda: not play_groups(), 2, 0.05, 'decode worker gone')
        cpu_before = cpu_ticks(pid)
        paused_shots = []
        for index in range(3):
            shot, _, _ = screenshot(f'paused-{index}')
            paused_shots.append(round(background_like(shot, baseline), 3))
            time.sleep(1.0)
        cpu_paused = cpu_ticks(pid) - cpu_before
        assert all(value < 0.5 for value in paused_shots), paused_shots
        assert not exited(pid) and not play_groups() and view['rect'] is None, view
        seen = len(lines_for(pid, offset))
        daemon.directive('play')
        wait(lambda: any(line.startswith('event=playing') for line in lines_for(pid, offset)[seen:]), 5, what='resume')
        wait(lambda: (lambda value: value if value['layer'] and value['state'] in (1, 4) else None)(shell_view()), 3, 0.05, 'revealed')
        indices = []
        for index in range(4):
            shot, _, _ = screenshot(f'resumed-{index}')
            analysis = analyse(item, shot)
            assert_slices(item, analysis)
            indices.append(analysis['monitors'][0]['frame'])
            time.sleep(0.23)
        assert len(set(indices)) >= 3, indices
        REPORT['pause'] = {'paused_background_error': paused_shots, 'renderer_cpu_ticks_paused_3s': cpu_paused,
                           'resumed_frames': indices, 'shell_view_paused': view}
        check(f'pause in the plan: renderer reports Paused, Shell hides the layer, worker and surface destroyed '
              f'(no play cgroup), screen equals the native background for 3 s ({paused_shots}), renderer used '
              f'{cpu_paused} CPU ticks while paused; play resumes real frames {indices}')
        private([BIN / 'one-mk', 'stop'])
        await_exit(pid, time.monotonic())
        reaped(pid)
        disarm()
    finally:
        daemon.stop()


def kill_proof():
    """The Shell force-exits (SIGKILLs) a playing renderer: the sandboxed
    worker dies with it and the next playback reclaims the stale group."""
    item = 'vp9'
    write_settings(400)
    daemon = Daemon(item)
    offset = log_size()
    try:
        pid = spawn(offset)
        playing(pid, offset)
        workers = worker_pids()
        groups = list(play_groups())
        assert workers and len(groups) == 1
        evaluate(f'(() => {{ const b = {BRIDGE}; b._authorized = false; b._stop(); return true; }})()')
        wait(lambda: exited(pid), 2, 0.005, 'renderer killed')
        wait(lambda: all(state(worker) is None for worker in workers), 3, 0.01, 'worker dies with the renderer')
        assert list(play_groups()) == groups and not play_groups()[groups[0]], play_groups()
        wait(lambda: evaluate(f'{BRIDGE}._recovery.children.size') == 0, 5, 0.05, 'Shell child reaped')
        offset = log_size()
        pid = spawn(offset)
        playing(pid, offset)
        assert groups[0] not in play_groups(), play_groups()
        private([BIN / 'one-mk', 'stop'])
        await_exit(pid, time.monotonic())
        reaped(pid)
        disarm()
        assert not play_groups()
        check(f'renderer SIGKILLed by the Shell while playing: {len(workers)} sandboxed worker process(es) died with it; '
              f'the empty stale group was reclaimed by the next playback; no group left after the clean exit')
    finally:
        daemon.stop()


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


def cycle(index, item, method, baseline, background, latencies, results):
    write_settings(1000 + index)
    daemon = Daemon(item)
    offset = log_size()
    try:
        pid = spawn(offset)
        playing(pid, offset)
        shot, _, _ = screenshot(f'cycle{index:03d}-{item}') if index % 10 == 0 else (None, None, None)
        if shot is None:
            shot = Image.open(private_shot()).convert('RGB')
        if item == 'clip1080p':
            assert background_like(shot, background) > 20
            frame = None
        else:
            analysis = analyse(item, shot)
            assert_slices(item, analysis)
            frame = analysis['monitors'][0]['frame']
        workers = worker_pids()
        assert workers or item == 'still'
        start = time.monotonic()
        teardown(daemon, method)
        latencies.setdefault(method, []).append(await_exit(pid, start))
        await_line(pid, offset, EXPECTED[method])
        reaped(pid)
        wait(lambda: all(state(worker) is None for worker in workers), 2, 0.01, 'workers gone')
        disarm()
        if method in ('stop', 'sigterm', 'sigkill'):
            daemon.process.wait(timeout=10)
    finally:
        daemon.stop()
    for renderer in renderers_since(offset):
        reaped(renderer)
    assert not play_groups(), f'cycle {index}: playback group left {play_groups()}'
    leftovers = {pid: comm for pid, comm in lab_processes().items() if pid not in baseline}
    assert not leftovers, f'cycle {index}: leftover processes {leftovers}'
    evaluate('Main.extensionManager.lookup("spike-harness@one-mk.test").stateObj.collectGarbage(); true')
    results.append({'cycle': index, 'item': item, 'teardown': method, 'frame': frame})


def private_shot():
    path = SHOTS / 'cycle-latest.png'
    private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Screenshot', '-o', '/org/gnome/Shell/Screenshot',
             '-m', 'org.gnome.Shell.Screenshot.Screenshot', 'false', 'false', path])
    return path


def main():
    LAB.mkdir(mode=0o700)
    (LAB / 'home').mkdir()
    SHOTS.mkdir()
    sources = LAB / 'extensions'
    sources.mkdir()
    destination = sources / UUID
    started = False
    cgroups = False
    try:
        for binary in ('one-mk', 'one-mk-wall', 'one-mk-wall-import'):
            os.chmod(BIN / binary, 0o755)
        REPORT['source_sha256'] = {binary: __import__('hashlib').sha256((BIN / binary).read_bytes()).hexdigest()
                                   for binary in ('one-mk', 'one-mk-wall', 'one-mk-wall-import')}
        prepare_cgroups()
        cgroups = True
        runtime = prepare_runtime()
        files = generate_media()
        import_media(files, runtime)
        config = write_playback_config(runtime)
        shutil.copytree(REPO / 'extensions/gnome' / UUID, destination)
        shutil.copytree(HERE / 'ext/spike-harness@one-mk.test', sources / 'spike-harness@one-mk.test')
        (destination / 'libexec').mkdir()
        wrapper = destination / 'libexec/one-mk-wall'
        wrapper.write_text(f'#!/bin/sh\nexport ONE_MK_WALL_M1_PLAYBACK="{config}"\nexec >>"{RENDERER_LOG}" 2>&1\n'
                           f'exec "{BIN / "one-mk-wall"}" --present=gnome --m1-lab\n')
        wrapper.chmod(0o700)
        ENV['EXT_SRC'] = str(sources)
        REPORT['shell'] = run(['gnome-shell', '--version']).stdout.strip()
        assert run([BIN / 'one-mk-wall', '--present=gnome'], check=False).returncode == 3
        run([HARNESS, 'start', '2'])
        started = True
        extension('Enable', 'spike-harness@one-mk.test')
        evaluate('Main.overview.hide(); true')
        extension('Enable')
        wait(lambda: evaluate(f'Boolean({BRIDGE}._ownsName && {BRIDGE}._ready)'), 5, 0.1, 'bridge ready')
        assert evaluate(f'{BRIDGE}._pid === null && {BRIDGE}._authorized === false')
        # The harness's own "unsafe mode" banner would cover part of a monitor.
        evaluate('Main.messageTray.getSources().forEach(source => source.destroy()); true')
        time.sleep(1.5)
        assert evaluate('Main.messageTray.getSources().length') == 0
        baseline, _, _ = screenshot('native-background')
        check('production extension dormant and unauthorized by default; renderer without --m1-lab refuses; '
              'native background captured as the baseline')
        for item in ITEMS:
            scenario(item, baseline)
        pause_proof(baseline)
        kill_proof()
        baseline_processes = lab_processes()
        latencies = {}
        results = []
        items = list(ITEMS)
        for index in range(CYCLES):
            cycle(index, items[index % len(items)], TEARDOWNS[index % len(TEARDOWNS)], baseline_processes, baseline, latencies, results)
            if index % 10 == 9:
                print(f'cycle {index + 1}/{CYCLES}', flush=True)
        REPORT['cycles'] = {'count': CYCLES, 'results': results,
                            'exit_latency_ms': {method: {'count': len(values), 'median': round(statistics.median(values) * 1000, 1),
                                                         'max': round(max(values) * 1000, 1)} for method, values in latencies.items()}}
        assert all(max(values) <= EXIT_BOUND for values in latencies.values())
        check(f'{CYCLES} real-playback cycles (rotating {len(items)} imported items x {len(TEARDOWNS)} teardowns): '
              f'every cycle presented verified frames, then the renderer exited, every worker died, no playback '
              f'group or process left')
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
        if cgroups:
            try:
                REPORT['remaining_play_groups'] = play_groups()
                remove_cgroups()
            except OSError as error:
                REPORT['cgroup_cleanup_error'] = str(error)
        shutil.rmtree(sources, ignore_errors=True)
        (LAB / 'result.json').write_text(json.dumps(REPORT, indent=2, default=str) + '\n')
        print(json.dumps({key: value for key, value in REPORT.items() if key not in ('checks', 'screenshots', 'cycles')},
                         indent=2, default=str))


if __name__ == '__main__':
    sys.exit(main())
