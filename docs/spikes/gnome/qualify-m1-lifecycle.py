#!/usr/bin/env python3
"""6D-M1 step C: lifecycle and restoration, exclusively inside the private
Mutter harness.

Builds on step B (`qualify-m1-playback.py`, imported for media, import,
daemon and process helpers): real imported media play through the laboratory
renderer while the private GNOME Shell 50.1 goes through layout changes and
hot-plug (add, remove, scale, fractional scale, reposition, rotation, a layout
outside the committed canvas), lock/unlock through the real screen shield
(fake GDM on the private system bus), suspend/resume through logind's
PrepareForSleep (a stand-in logind on the private system bus), renderer and
daemon kills, extension disable/uninstall and GNOME Shell restarts. Every
claim is checked with Shell screenshots against exact expected slices or
against the user's own wallpaper, plus process and cgroup accounting.

Environment: as `qualify-m1-playback.py` (M1_BIN, M1_LAB, M1_TRUSTED,
M1_CGROUP), plus
  M1_SEQUENCES  randomized lifecycle sequences (default 100)
  M1_SEED       random seed (default 20261031)
  M1_ONLY       comma-separated directed proofs to run (default: all; a
                qualifying run uses all of them)

Never connects to the owner's desktop, X servers, clipboard or session bus.
Production authorization stays false: playback needs both laboratory
features and private Eval authorization of this harness's bridge. When the
Shell re-enables the extension (unlock, re-enable, restart) a private Eval
hook re-authorizes the new bridge instance while the laboratory is armed: a
stand-in for the production authorization channel (step D), not part of it.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time
import traceback

from PIL import Image, ImageChops, ImageDraw, ImageStat

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
_spec = importlib.util.spec_from_file_location('qualify_m1_playback', HERE / 'qualify-m1-playback.py')
pb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pb)
media = pb.media

SEQUENCES = int(os.environ.get('M1_SEQUENCES', '100'))
SEED = int(os.environ.get('M1_SEED', '20261031'))
LAB, SESSION, SHOTS, BIN, CGROUP = pb.LAB, pb.SESSION, pb.SHOTS, pb.BIN, pb.CGROUP
UUID = pb.UUID
HARNESS = pb.HARNESS
BRIDGE = f'Main.extensionManager.lookup("{UUID}")?.stateObj?.bridge'
private, evaluate, wait, state, exited = pb.private, pb.evaluate, pb.wait, pb.state, pb.exited

# One shared canvas: an absent computer's 1920-wide slice on the left, then
# room for every committed local layout (up to three 1920x1080 monitors side
# by side, two stacked, one rotated). This computer's (0, 0) sits at canvas
# (0, 540), so even a small top-row monitor (scale 2) shows the media's
# frame-identifying bands.
CANVAS = [-1920, 0, 7680, 2700]
ORIGIN = (0, 540)
ABSENT = (-1920, 0, 1920, 2700)
pb.CANVAS = CANVAS
# Rows covered by the top bar (logical units, every monitor's top).
PANEL = 64
PRIMARY = 'Meta-0'
# Logical monitors: connector, x, y, scale, transform (1 = 90 degrees).
LAYOUTS = {
    'two': [('Meta-0', 0, 0, 1.0, 0), ('Meta-1', 1920, 0, 1.0, 0)],
    'three': [('Meta-0', 0, 0, 1.0, 0), ('Meta-1', 1920, 0, 1.0, 0), ('Meta-2', 3840, 0, 1.0, 0)],
    'one': [('Meta-0', 0, 0, 1.0, 0)],
    'two@2': [('Meta-0', 0, 0, 2.0, 0), ('Meta-1', 960, 0, 2.0, 0)],
    'two@1.25': [('Meta-0', 0, 0, 1.25, 0), ('Meta-1', 1536, 0, 1.25, 0)],
    'swapped': [('Meta-0', 1920, 0, 1.0, 0), ('Meta-1', 0, 0, 1.0, 0)],
    'rotated': [('Meta-0', 0, 0, 1.0, 1), ('Meta-1', 1080, 0, 1.0, 0)],
    'stacked': [('Meta-0', 0, 0, 1.0, 0), ('Meta-1', 0, 1080, 1.0, 0)],
    # Outside the committed canvas (bottom edge 3240 > 2160): static, by design.
    'column3': [('Meta-0', 0, 0, 1.0, 0), ('Meta-1', 0, 1080, 1.0, 0), ('Meta-2', 0, 2160, 1.0, 0)],
}
DESCRIPTION = {'two': 'two monitors', 'three': 'monitor added (three)', 'one': 'monitors removed (one)',
               'two@2': 'scale 2 (logical resize)', 'two@1.25': 'fractional scale 1.25', 'swapped': 'reposition (order swapped)',
               'rotated': 'rotation (1080x1920 resize)', 'stacked': 'reposition (stacked vertically)',
               'column3': 'layout outside the committed canvas'}
ITEMS = dict(pb.ITEMS)
# A GIF whose long frames (2.6 s) outlast the Shell's 3 s damage watchdog tick.
ITEMS['longgif'] = {'frames': 4, 'size': (576, 108), 'lossless': True, 'durations': [400, 2600, 400, 2600]}
pb.ITEMS.update(ITEMS)
LOG = LAB / 'lifecycle.log'
REPORT = {'checks': [], 'production_authorization': False, 'timings': {}, 'events': {}, 'sequences': [],
          'shell_sessions': 1, 'media': pb.REPORT['media']}
WALLPAPER_URI = None
EXIT_BOUND = 2.5        # SIGTERM path; SIGKILL escalation is at 2 s
LAYOUT_BOUND = 2.5      # design: replacement within 2 s, else static
RESUME_BOUND = 5.0      # design: <= 5 s after unlock/wake


def note(text):
    with LOG.open('a') as log:
        log.write(f'{time.strftime("%H:%M:%S")} {text}\n')


def check(text):
    REPORT['checks'].append(text)
    note('PASS ' + text)
    print('PASS', text, flush=True)


def timing(name, seconds):
    REPORT['timings'].setdefault(name, []).append(round(seconds * 1000, 1))


def count(event):
    REPORT['events'][event] = REPORT['events'].get(event, 0) + 1


# ---------------------------------------------------------------- geometry

def logical(name):
    """The Shell's monitors for a layout: (x, y, width, height, scale) in
    Mutter's monitor order (connector order)."""
    monitors = []
    for connector, x, y, scale, transform in sorted(LAYOUTS[name]):
        width, height = (1080, 1920) if transform % 2 else (1920, 1080)
        monitors.append((x, y, round(width / scale), round(height / scale), scale))
    return monitors


def extent(monitors):
    left = min(m[0] for m in monitors)
    top = min(m[1] for m in monitors)
    return [left, top, max(m[0] + m[2] for m in monitors) - left, max(m[1] + m[3] for m in monitors) - top]


def fits(name):
    x0, y0, width, height = CANVAS
    return all(x0 <= x + ORIGIN[0] and y0 <= y + ORIGIN[1] and x + ORIGIN[0] + w <= x0 + width
               and y + ORIGIN[1] + h <= y0 + height for x, y, w, h, _ in logical(name))


def physical_size(name):
    monitors = logical(name)
    scale = monitors[0][4]
    left, top, width, height = extent(monitors)
    return (round(width * scale), round(height * scale))


# ---------------------------------------------------------------- Shell

def shell_monitors():
    return [tuple(value) for value in evaluate(
        'JSON.parse(JSON.stringify(Main.layoutManager.monitors.map(m => '
        '[m.x, m.y, m.width, m.height, global.display.get_monitor_scale(m.index)])))')]


def view():
    return evaluate(f'(() => {{ const b = {BRIDGE}; if (!b) return null; const r = b._renderer; '
                    f'return {{state: b._state, layer: b._layer.visible, fresh: r?.fresh === true, reasons: b._reasons, '
                    f'clones: b._clones.length, rect: r ? (g => [g.x, g.y, g.width, g.height])(r.window.get_buffer_rect()) : null, '
                    f'pid: b._pid, authorized: b._authorized, sources: [...b._sources.keys()], '
                    f'attempts: b._recovery.attempts.length, starts: b._recovery.starts.length, '
                    f'exhausted: b._recovery.exhausted, children: b._recovery.children.size, '
                    f'escalations: b._recovery.escalations.size, serial: b._serial, monitors: b._monitors, lastStop: b.lastStop ?? null}}; }})()')


def install_lab_hook():
    """Private Eval: re-authorize each new bridge instance while armed (the
    production authorization channel is step D)."""
    evaluate(f'(() => {{ if (globalThis.__omkLabHook) return true; globalThis.__omkLabArmed = false; '
             f'globalThis.__omkLabHook = Main.extensionManager.connect("extension-state-changed", (_m, extension) => {{ '
             f'const b = extension.uuid === "{UUID}" ? extension.stateObj?.bridge : null; '
             f'if (b && globalThis.__omkLabArmed && !b._authorized) {{ b._authorized = true; b._coverage(); }} }}); '
             f'return true; }})()')


def arm(reset_budget=True):
    reset = 'b._recovery.attempts = []; b._recovery.starts = []; b._recovery.exhausted = false; ' if reset_budget else ''
    evaluate(f'(() => {{ globalThis.__omkLabArmed = true; const b = {BRIDGE}; {reset}'
             f'b._authorized = true; b._coverage(); return true; }})()')


def disarm():
    evaluate(f'(() => {{ globalThis.__omkLabArmed = false; const b = {BRIDGE}; if (b) {{ b._authorized = false; b._stop(); }} '
             f'return true; }})()')
    wait(lambda: (lambda v: v is None or v['children'] == 0)(view()), 5, 0.05, 'Shell children reaped')


def extension(action):
    return pb.extension(action)


def extension_state():
    return evaluate(f'Main.extensionManager.lookup("{UUID}")?.state ?? null')


def lock():
    evaluate('Main.screenShield.lock(false); true')


def unlock():
    evaluate('Main.screenShield.deactivate(false); true')


def locked():
    return evaluate('Main.screenShield.locked')


def set_lock_on_suspend(value):
    evaluate('(() => { new imports.gi.Gio.Settings({schema_id: "org.gnome.desktop.screensaver"})'
             f'.set_boolean("ubuntu-lock-on-suspend", {"true" if value else "false"}); return true; }})()')


def simulate_sleep(about_to_sleep):
    private(['gdbus', 'call', '--system', '-d', 'org.freedesktop.login1', '-o', '/org/freedesktop/login1', '-m',
             'org.onemk.Test.Sleep.Simulate', 'true' if about_to_sleep else 'false'])


def background_settings():
    return evaluate('(() => { const s = new imports.gi.Gio.Settings({schema_id: "org.gnome.desktop.background"}); '
                    'return [s.get_string("picture-uri"), s.get_string("picture-uri-dark"), s.get_string("picture-options")]; })()')


def set_user_wallpaper(uri):
    evaluate('(() => { const s = new imports.gi.Gio.Settings({schema_id: "org.gnome.desktop.background"}); '
             f's.set_string("picture-uri", "{uri}"); s.set_string("picture-uri-dark", "{uri}"); '
             's.set_string("picture-options", "zoom"); return true; })()')


def apply_layout(name, settle=True):
    private(['python3', HERE / 'client/displayconfig.py', 'layout', json.dumps(LAYOUTS[name])])
    expected = logical(name)
    wait(lambda: shell_monitors() == expected, 5, 0.05, f'Shell layout {name}')
    if settle:
        wait(lambda: (lambda v: v is None or [tuple(m) for m in v['monitors']] == expected)(view()), 3, 0.05,
             f'bridge layout {name}')
    CURRENT['layout'] = name


CURRENT = {'layout': 'three'}


# ---------------------------------------------------------------- screenshots

SHOT_COUNT = [0]


def screenshot(label, keep=False):
    SHOT_COUNT[0] += 1
    path = SHOTS / (f'{SHOT_COUNT[0]:05d}-{label}.png' if keep else 'latest.png')
    private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Screenshot', '-o', '/org/gnome/Shell/Screenshot',
             '-m', 'org.gnome.Shell.Screenshot.Screenshot', 'false', 'false', path])
    with Image.open(path) as image:
        return image.convert('RGB')


def regions(name):
    """Physical screenshot rectangles of each monitor below its top rows (the
    top bar's clock changes). Stage areas outside every monitor (for example
    below the unrotated monitor beside a rotated one) hold undefined pixels
    and are never compared."""
    boxes = []
    for x, y, width, height, scale in logical(name):
        boxes.append((round(x * scale), round((y + PANEL) * scale), round((x + width) * scale), round((y + height) * scale)))
    return boxes


def body(image, name):
    """Only the monitors' pixels below the top rows; the rest is black."""
    masked = Image.new('RGB', image.size)
    for box in regions(name):
        masked.paste(image.crop(box), box[:2])
    return masked


def native_error(shot, name):
    baseline = BASELINES[name]
    if shot.size != baseline.size:
        return 255.0
    total = weight = 0
    for box in regions(name):
        area = (box[2] - box[0]) * (box[3] - box[1])
        total += pb.mean_error(shot.crop(box), baseline.crop(box)) * area
        weight += area
    return total / weight


BASELINES = {}


def analyse(item, shot, monitors):
    """Identifies the frame each monitor shows and proves it is exactly that
    monitor's slice of the shared canvas (never the absent computer's)."""
    size = ITEMS[item]['size']
    frames = ITEMS[item]['frames']
    absent_columns, _ = pb.mapping(size, ABSENT)
    pixels = shot.load()
    result = []
    for x, y, width, height, scale in monitors:
        columns, rows = pb.mapping(size, (x + ORIGIN[0], y + ORIGIN[1], width, height))
        kept = [row for row in rows if row[1] >= PANEL]
        data = bytearray()
        for _, centre_y, _ in kept:
            for _, centre_x, _ in columns:
                data.extend(pixels[int((x + centre_x + 0.5) * scale), int((y + centre_y + 0.5) * scale)])
        observed = Image.frombytes('RGB', (len(columns), len(kept)), bytes(data))
        box = (columns[0][0], kept[0][0], columns[-1][0] + 1, kept[-1][0] + 1)
        errors = [pb.mean_error(observed, pb.reference(item, index).crop(box)) for index in range(frames)]
        best = min(range(frames), key=errors.__getitem__)
        absent_box = (absent_columns[0][0], box[1], absent_columns[-1][0] + 1, box[3])
        entry = {'monitor': [x, y, width, height, scale], 'frame': best, 'error': round(errors[best], 3),
                 'absent_slice_error': round(pb.mean_error(observed, pb.reference(item, best).crop(absent_box)), 1),
                 'runner_up_error': round(sorted(errors)[1], 3) if frames > 1 else None}
        if ITEMS[item]['lossless'] and scale == 1:
            # Every output pixel below the top bar, not just block centres.
            source = pb.reference(item, best)
            raw = source.tobytes()
            source_width = source.size[0]
            row_index = {offset: index for index, _, members in rows for offset in members}
            column_values = [index for index, _, members in columns for _ in members]
            lines = {}
            data = bytearray()
            for offset in range(PANEL, height):
                index = row_index[offset]
                if index not in lines:
                    start = index * source_width * 3
                    lines[index] = b''.join(raw[start + column * 3:start + column * 3 + 3] for column in column_values)
                data.extend(lines[index])
            expected = Image.frombytes('RGB', (width, height - PANEL), bytes(data))
            actual = shot.crop((x, y + PANEL, x + width, y + height))
            entry['max_channel_error'] = max(high for _, high in ImageChops.difference(actual, expected).getextrema())
            entry['every_pixel'] = True
        elif ITEMS[item]['lossless']:
            entry['max_channel_error'] = max(high for _, high in ImageChops.difference(
                observed, pb.reference(item, best).crop(box)).getextrema())
            entry['every_pixel'] = False
        else:
            entry['psnr_db'] = pb.psnr(observed, pb.reference(item, best).crop(box))
        result.append(entry)
    return result


def assert_frames(item, analysis):
    assert len({entry['frame'] for entry in analysis}) == 1, analysis
    for entry in analysis:
        if ITEMS[item]['lossless']:
            assert entry['max_channel_error'] <= ITEMS[item].get('tolerance', 0), (item, entry)
        else:
            assert entry['error'] < 6 and entry['psnr_db'] >= 30, (item, entry)
        assert entry['absent_slice_error'] > 20, (item, entry)
        if entry['runner_up_error'] is not None:
            if ITEMS[item]['lossless']:
                assert entry['runner_up_error'] > entry['error'] + 3, (item, entry)
            else:
                # Lossy video (PSNR >= 30 dB checked above): on small or scaled
                # monitors fewer identifying rows are visible, so the next-best
                # frame is closer; it must still be clearly worse.
                assert entry['runner_up_error'] >= 1.5 * entry['error'] and \
                    entry['runner_up_error'] >= entry['error'] + 0.5, (item, entry)


def media_on_screen(item, shot, name):
    """Classifies a screenshot: 'native', 'media' (exact slices for this
    layout) or 'other'."""
    if shot.size != physical_size(name):
        return 'other'
    if native_error(shot, name) < 0.5:
        return 'native'
    if item == 'clip1080p':
        return 'media' if native_error(shot, name) > 20 else 'other'
    try:
        assert_frames(item, analyse(item, shot, logical(name)))
        return 'media'
    except AssertionError:
        return 'other'


# ---------------------------------------------------------------- processes

def lab_processes():
    return pb.lab_processes()


def renderer_alive(pid):
    return pid is not None and not exited(pid)


OURS = {str(BIN / name) for name in ('one-mk', 'one-mk-wall', 'one-mk-wall-import')}
CGROUP_PREFIX = '0::/' + str(CGROUP.resolve().relative_to('/sys/fs/cgroup'))


def our_processes():
    """Every live process of ours: by executable (daemon, renderer, importer)
    or by membership in this run's delegated cgroup (sandboxed workers and
    their bubblewrap), independent of environment markers."""
    found = {}
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            exe = os.readlink(entry / 'exe').removesuffix(' (deleted)')
            group = (entry / 'cgroup').read_text().strip()
            comm = (entry / 'comm').read_text().strip()
        except OSError:
            continue
        if (exe in OURS or group.startswith(CGROUP_PREFIX + '/')) and not exited(int(entry.name)):
            found[int(entry.name)] = comm
    return found


def leaks(allowed):
    return {pid: comm for pid, comm in our_processes().items() if pid not in allowed}


class Context:
    def __init__(self, label):
        self.label = label
        self.daemon = None
        self.item = None
        self.paused = False
        self.pid = None
        self.revealed = None
        self.offset = pb.log_size()

    def static_expected(self):
        return self.paused or not fits(CURRENT['layout'])


def baseline_allowed(ctx):
    allowed = set()
    if ctx.daemon and ctx.daemon.process.poll() is None:
        allowed.add(ctx.daemon.pid)
    view_now = view()
    if view_now and view_now['pid']:
        allowed.add(view_now['pid'])
    return allowed


# ---------------------------------------------------------------- settled states

def settled(ctx, previous_pid=None, timeout=RESUME_BOUND, label='settled', keep=False):
    """Waits until the presentation is what the plan and layout say it must
    be, then proves it on screen. Returns (view, seconds)."""
    start = time.monotonic()
    name = CURRENT['layout']
    monitors = logical(name)
    if ctx.static_expected():
        current = wait(lambda: (lambda v: v if v and v['pid'] and v['pid'] != previous_pid and v['state'] == 2
                                and not v['layer'] and v['rect'] is None else None)(view()),
                       timeout, 0.03, f'{ctx.label}: static renderer (paused plan or layout outside the canvas)')
        elapsed = time.monotonic() - start
        ctx.pid = current['pid']
        wait(lambda: any('event=accepted' in line and 'paused=true' in line
                         for line in pb.lines_for(ctx.pid, ctx.offset)), 3, 0.05, f'{ctx.label}: paused plan admitted')
        assert not pb.play_groups(), pb.play_groups()
        shot = screenshot(f'{label}-static', keep)
        error = native_error(shot, name)
        assert error < 0.5, (ctx.label, 'static screen is not the native background', error)
        return current, elapsed
    expected = [0, 0, *extent(monitors)[2:]]
    current = wait(lambda: (lambda v: v if v and v['pid'] and v['pid'] != previous_pid and v['layer'] and v['fresh']
                            and v['state'] in (1, 4) and v['reasons'] == 0 and v['rect'] == expected
                            and v['clones'] == len(monitors) else None)(view()),
                   timeout, 0.02, f'{ctx.label}: presentation revealed for {name}')
    elapsed = time.monotonic() - start
    ctx.revealed = time.monotonic()
    ctx.pid = current['pid']
    verify(ctx, label, keep)
    return current, elapsed


def verify(ctx, label, keep=False):
    """Screenshots prove each monitor shows its exact slice and that frames
    advance (or, for the still, stay)."""
    name = CURRENT['layout']
    monitors = logical(name)
    first = screenshot(f'{label}-{ctx.item}-{name}', keep)
    assert first.size == physical_size(name), (first.size, name)
    if ctx.item == 'clip1080p':
        assert native_error(first, name) > 20
        time.sleep(0.3)
        second = screenshot(f'{label}-again')
        assert pb.mean_error(body(first, name), body(second, name)) > 0.5, 'clip frames do not change'
        return {'frames': None}
    analysis = analyse(ctx.item, first, monitors)
    assert_frames(ctx.item, analysis)
    frames = [analysis[0]['frame']]
    if ctx.item == 'still':
        time.sleep(0.4)
        again = screenshot(f'{label}-still-again')
        assert ImageChops.difference(body(first, name), body(again, name)).getbbox() is None
        return {'frames': frames}
    # A later screenshot shows a later frame: the content is live, not stale.
    deadline = time.monotonic() + (3.5 if ctx.item == 'longgif' else 1.5)
    while time.monotonic() < deadline:
        time.sleep(0.15)
        later = analyse(ctx.item, screenshot(f'{label}-later'), monitors)
        assert_frames(ctx.item, later)
        frames.append(later[0]['frame'])
        if frames[-1] != frames[0]:
            return {'frames': frames}
    raise AssertionError(f'{ctx.label}: frames do not advance {frames}')


def renderer_ended(pid, start, how, ctx):
    """The renderer exits within the bound; for Shell-initiated stops via
    the clean SIGTERM path (its worker and cgroup removed by itself)."""
    elapsed = pb.await_exit(pid, start, EXIT_BOUND)
    needle = {'sigterm': 'event=terminated signal=SIGTERM', 'owner': 'authority connection lost',
              'revoked': 'presentation torn down: Revoked', 'wayland': 'exit'}.get(how)
    if needle:
        pb.await_line(pid, ctx.offset, needle, 2)
    pb.reaped(pid)
    wait(lambda: not pb.play_groups(), 2, 0.02, f'{ctx.label}: playback group removed')
    return elapsed


# ---------------------------------------------------------------- events

def event_layout(ctx, target, burst=False):
    count('layout')
    source = CURRENT['layout']
    previous = ctx.pid
    groups_before = list(pb.play_groups())
    start = time.monotonic()
    apply_layout(target, settle=False)
    # Immediately after the change: the layer is hidden, or shows a fresh
    # window of the new extent; never the old window or old slicing.
    current = view()
    new_extent = [0, 0, *extent(logical(target))[2:]]
    assert not current['layer'] or (current['fresh'] and current['rect'] == new_extent), \
        (ctx.label, f'{source}->{target}: stale presentation visible', current)
    shot = screenshot(f'{ctx.label}-transition')
    kind = media_on_screen(ctx.item, shot, target)
    REPORT.setdefault('transition_screens', {}).setdefault(kind, 0)
    REPORT['transition_screens'][kind] += 1
    if kind == 'other':
        shot.save(SHOTS / f'transition-other-{SHOT_COUNT[0]:05d}.png')
    current = view()
    assert not current['layer'] or (current['fresh'] and current['rect'] == new_extent), current
    current, elapsed = settled(ctx, previous_pid=None, timeout=LAYOUT_BOUND + 2, label=f'{ctx.label}-layout')
    total = time.monotonic() - start
    if not ctx.static_expected():
        reveal = ctx.revealed - start
        timing('layout_to_reveal_s', reveal)
        assert reveal <= LAYOUT_BOUND, (ctx.label, f'{source}->{target} revealed after {reveal:.2f}s')
    assert current['pid'] == previous, (ctx.label, 'layout change must not restart the renderer', previous, current['pid'])
    if not ctx.static_expected():
        assert total <= LAYOUT_BOUND + 1.0, (ctx.label, f'{source}->{target} took {total:.2f}s')
        timing('layout_to_verified_s', total)
        assert all(group not in pb.play_groups() for group in groups_before) or not groups_before
    else:
        timing('layout_to_static_s', total)
        # Past the Shell's 2 s replacement deadline: still alive, still static.
        time.sleep(2.2)
        current = view()
        assert current['pid'] == previous and renderer_alive(previous) and current['state'] == 2 and not current['layer'], current
        assert native_error(screenshot(f'{ctx.label}-static-later'), target) < 0.5
    note(f'{ctx.label}: layout {source} -> {target} {"static" if ctx.static_expected() else "verified"} in {total:.2f}s')
    return (ctx.revealed - start) if not ctx.static_expected() else total


def event_layout_burst(ctx):
    """Three changes 150 ms apart (re-plug storm): converges, no crash."""
    count('layout-burst')
    choices = [name for name in LAYOUTS if fits(name) and name != CURRENT['layout']]
    sequence = random.sample(choices, 3)
    for name in sequence:
        private(['python3', HERE / 'client/displayconfig.py', 'layout', json.dumps(LAYOUTS[name])])
        time.sleep(0.15)
    final = sequence[-1]
    wait(lambda: shell_monitors() == logical(final), 5, 0.05, 'burst layout')
    CURRENT['layout'] = final
    start = time.monotonic()
    # The Shell may replace the renderer if a stale-sized window raced the
    # burst; either way it must converge to verified frames.
    current, _ = settled(ctx, previous_pid=None, timeout=8, label=f'{ctx.label}-burst')
    timing('burst_converged_s', time.monotonic() - start)
    note(f'{ctx.label}: burst {sequence} converged pid {ctx.pid}')


def event_lock(ctx):
    count('lock')
    pid = ctx.pid
    workers = pb.worker_pids()
    start = time.monotonic()
    lock()
    elapsed = renderer_ended(pid, start, 'sigterm', ctx)
    timing('lock_to_renderer_exit_s', elapsed)
    wait(lambda: all(state(worker) is None for worker in workers), 2, 0.02, 'workers gone at lock')
    wait(lambda: locked(), 3, 0.05, 'screen locked')
    # While locked: the extension is disabled by the unlock-dialog session
    # mode (no bridge, no timers), no renderer, no decoder, no group.
    time.sleep(1.0)
    assert view() is None, view()
    assert not pb.play_groups()
    stray = leaks(baseline_allowed(ctx))
    assert not stray, stray
    locked_shot = screenshot(f'{ctx.label}-locked')
    # (The moving 1080p clip has no exact reference; its absence is proven by the process checks.)
    assert ctx.item == 'clip1080p' or media_on_screen(ctx.item, locked_shot, CURRENT['layout']) != 'media'
    start = time.monotonic()
    unlock()
    _, _ = settled(ctx, previous_pid=pid, timeout=RESUME_BOUND + 1, label=f'{ctx.label}-unlock')
    timing('unlock_to_settled_s', time.monotonic() - start)


def event_suspend(ctx, with_lock):
    count('suspend-lock' if with_lock else 'suspend')
    set_lock_on_suspend(with_lock)
    try:
        pid = ctx.pid
        workers = pb.worker_pids()
        start = time.monotonic()
        simulate_sleep(True)
        elapsed = renderer_ended(pid, start, 'sigterm', ctx)
        timing('sleep_to_renderer_exit_s', elapsed)
        wait(lambda: all(state(worker) is None for worker in workers), 2, 0.02, 'workers gone at sleep')
        time.sleep(0.8)
        if with_lock:
            assert locked() and view() is None
        else:
            current = view()
            assert current and current['reasons'] & 8 and not current['layer'] and current['pid'] is None, current
            assert not [source for source in current['sources'] if source in ('restart', 'watchdog', 'hello', 'first-frame')], current
            # Asleep: the screen shows the user's wallpaper, never a frame.
            assert native_error(screenshot(f'{ctx.label}-asleep'), CURRENT['layout']) < 0.5
        assert not pb.play_groups()
        stray = leaks(baseline_allowed(ctx))
        assert not stray, stray
        start = time.monotonic()
        simulate_sleep(False)
        if with_lock:
            time.sleep(0.3)
            assert locked() and view() is None
            unlock()
        settled(ctx, previous_pid=pid, timeout=RESUME_BOUND + 1, label=f'{ctx.label}-wake')
        timing('wake_to_settled_s', time.monotonic() - start)
    finally:
        set_lock_on_suspend(True)


def event_renderer_kill(ctx):
    count('renderer-kill')
    pid = ctx.pid
    workers = pb.worker_pids()
    groups = list(pb.play_groups())
    os.kill(pid, signal.SIGKILL)
    wait(lambda: exited(pid), 2, 0.005, 'renderer killed')
    wait(lambda: all(state(worker) is None for worker in workers), 3, 0.01, 'worker dies with the renderer')
    # The Shell hides the layer and restarts after its 1 s failure delay.
    current, _ = settled(ctx, previous_pid=pid, timeout=8, label=f'{ctx.label}-respawn')
    assert not any(group in pb.play_groups() for group in groups), (groups, pb.play_groups())
    pb.reaped(pid)


def event_pause(ctx):
    count('pause')
    pid = ctx.pid
    seen = len(pb.lines_for(pid, ctx.offset))
    ctx.daemon.directive('pause')
    ctx.paused = True
    if pid and renderer_alive(pid) and view()['state'] != 2:
        wait(lambda: any(line.startswith('event=stopped') for line in pb.lines_for(pid, ctx.offset)[seen:]), 3, 0.02,
             'renderer stops on the paused plan')
    wait(lambda: (lambda v: v and v['state'] == 2 and not v['layer'] and v['rect'] is None)(view()), 3, 0.05, 'paused view')
    wait(lambda: not pb.play_groups(), 2, 0.02, 'paused: no decoder')
    assert native_error(screenshot(f'{ctx.label}-paused'), CURRENT['layout']) < 0.5
    ctx.daemon.directive('play')
    ctx.paused = False
    settled(ctx, previous_pid=None, timeout=RESUME_BOUND, label=f'{ctx.label}-play')
    assert ctx.pid == pid


def event_extension_cycle(ctx):
    count('extension-cycle')
    pid = ctx.pid
    start = time.monotonic()
    extension('Disable')
    renderer_ended(pid, start, 'sigterm', ctx)
    assert view() is None
    assert native_error(screenshot(f'{ctx.label}-ext-disabled'), CURRENT['layout']) < 0.5
    assert background_settings()[0] == WALLPAPER_URI
    extension('Enable')
    settled(ctx, previous_pid=pid, timeout=RESUME_BOUND + 2, label=f'{ctx.label}-ext-enabled')


# ---------------------------------------------------------------- teardowns (restoration)

def restored(ctx, label):
    """The user's own wallpaper is what is on screen and nothing of ours is left."""
    name = CURRENT['layout']
    shot = screenshot(f'{label}-restored', keep=True)
    error = native_error(shot, name)
    assert error < 0.5, (label, 'screen is not the user wallpaper', error)
    assert background_settings() == [WALLPAPER_URI, WALLPAPER_URI, 'zoom'], background_settings()
    assert not pb.play_groups(), pb.play_groups()
    stray = leaks(set())
    assert not stray, (label, 'leftover processes', stray)
    current = view()
    assert current is None or (current['children'] == 0 and current['pid'] is None and not current['layer']), current
    return error


def teardown(ctx, method):
    count('teardown-' + method)
    pid = ctx.pid
    workers = pb.worker_pids()
    start = time.monotonic()
    if method in ('stop', 'sigterm', 'sigkill', 'revoke', 'disable'):
        if method == 'stop':
            private([BIN / 'one-mk', 'stop'])
        elif method == 'sigterm':
            ctx.daemon.process.send_signal(signal.SIGTERM)
        elif method == 'sigkill':
            ctx.daemon.process.send_signal(signal.SIGKILL)
        elif method == 'revoke':
            private([BIN / 'one-mk', 'wallpaper', 'consent', 'revoke', pb.PEER])
        else:
            private([BIN / 'one-mk', 'wallpaper', 'disable'])
        if pid and renderer_alive(pid):
            how = 'revoked' if method in ('revoke', 'disable') else 'owner'
            timing(f'teardown_{method}_exit_s', renderer_ended(pid, start, how, ctx))
        disarm()
    elif method == 'renderer-kill':
        # Disarmed first: no restart follows, so this is the bare SIGKILL case.
        evaluate('globalThis.__omkLabArmed = false; true')
        evaluate(f'(() => {{ const b = {BRIDGE}; b._authorized = false; return true; }})()')
        if pid:
            os.kill(pid, signal.SIGKILL)
            wait(lambda: exited(pid), 2, 0.005, 'renderer killed')
            wait(lambda: all(state(worker) is None for worker in workers), 3, 0.01, 'workers die with the renderer')
        disarm()
        stale = pb.play_groups()
        assert all(not members for members in stale.values()), stale
        if stale:
            # Process-free directories only; the next renderer start reclaims them.
            REPORT.setdefault('stale_empty_groups_after_bare_sigkill', []).append(len(stale))
            ctx.daemon.stop()
            ctx.daemon = None
            reclaim_check()
    elif method == 'extension-disable':
        extension('Disable')
        if pid:
            timing('teardown_extension-disable_exit_s', renderer_ended(pid, start, 'sigterm', ctx))
        evaluate('globalThis.__omkLabArmed = false; true')
    else:
        raise ValueError(method)
    if ctx.daemon:
        ctx.daemon.stop()
    for renderer in pb.renderers_since(ctx.offset):
        pb.reaped(renderer, timeout=6)
    error = restored(ctx, f'{ctx.label}-{method}')
    if method == 'extension-disable':
        extension('Enable')
        wait(lambda: (lambda v: v and v['pid'] is None)(view()), 5, 0.05, 'extension re-enabled dormant')
    return error


def reclaim_check():
    """A bare renderer SIGKILL leaves its worker's (empty) group; the next
    renderer reclaims it at start, before any playback."""
    stale = list(pb.play_groups())
    write_settings_next()
    daemon = Daemon('gif')
    try:
        offset = pb.log_size()
        arm()
        pid = wait(lambda: pb.renderers_since(offset)[:1], 8, 0.05, 'renderer start')[0]
        line = pb.await_line(pid, offset, 'event=reclaimed stale_playback_groups=', 5)
        assert all(group not in pb.play_groups() for group in stale), (stale, pb.play_groups())
        REPORT.setdefault('reclaimed_at_start', []).append(line)
        disarm()
    finally:
        daemon.stop()
    for renderer in pb.renderers_since(offset):
        pb.reaped(renderer, timeout=6)
    assert not pb.play_groups()


REVISION = [5000]


class Daemon(pb.Daemon):
    """The step B laboratory daemon with this run's canvas and origin."""

    def __init__(self, item):
        pb.Daemon.count += 1
        self.log_path = LAB / 'daemon.log'
        self.offset = self.log_path.stat().st_size if self.log_path.exists() else 0
        read, write = os.pipe()
        config = {'peer': pb.PEER, 'media': pb.REPORT['media'][item]['hash'], 'canvas': CANVAS, 'origin': list(ORIGIN)}
        os.write(write, json.dumps(config).encode())
        os.close(write)
        self.log = self.log_path.open('a')
        self.log.write(f'--- daemon start {pb.Daemon.count} ({item})\n')
        self.log.flush()
        self.process = subprocess.Popen(
            [str(HARNESS), 'run', 'env', f'ONE_MK_M1_LAB_FD={read}', str(BIN / 'one-mk'), 'run', '--backend', 'null',
             '--no-discovery', '--no-notify', '--no-clipboard', '--port', str(pb.free_port())],
            env=pb.ENV, cwd=LAB, stdout=self.log, stderr=self.log, pass_fds=(read,))
        os.close(read)
        self.pid = self.process.pid
        wait(self._ready, 15, what='daemon authority record')


def write_settings_next():
    REVISION[0] += 1
    pb.write_settings(REVISION[0])


# ---------------------------------------------------------------- Shell restart

FAKES = {}


def start_fakes():
    log = (LAB / 'fake-logind.log').open('w')
    FAKES['logind'] = subprocess.Popen([str(HARNESS), 'run', 'python3', str(HERE / 'client/fake_logind.py'), str(SESSION)],
                                       env=pb.ENV, cwd=LAB, stdout=log, stderr=log)
    log.close()
    wait(lambda: 'fake_logind: ready' in (LAB / 'fake-logind.log').read_text(), 5, 0.05, 'fake logind')


def stop_fakes():
    process = FAKES.pop('logind', None)
    if process and process.poll() is None:
        process.terminate()
        process.wait(timeout=5)


def prepare_shell():
    """After a (re)start: harness extension, overview hidden, our extension
    enabled and dormant, the user's wallpaper, the laboratory hook."""
    pb.extension('Enable', 'spike-harness@one-mk.test')
    evaluate('Main.overview.hide(); true')
    extension('Enable')
    wait(lambda: evaluate(f'Boolean({BRIDGE}?._ownsName && {BRIDGE}?._ready)'), 5, 0.1, 'bridge ready')
    assert evaluate(f'{BRIDGE}._pid === null && {BRIDGE}._authorized === false')
    evaluate('Main.messageTray.getSources().forEach(source => source.destroy()); true')
    install_lab_hook()
    if WALLPAPER_URI:
        set_user_wallpaper(WALLPAPER_URI)
    start_fakes()
    time.sleep(1.5)
    assert evaluate('Main.messageTray.getSources().length') == 0


def keep_shell_log():
    index = REPORT['shell_sessions']
    shutil.copy2(SESSION / 'log/shell.log', LAB / f'shell-{index}.log')


def restart_shell(ctx, label, uninstall=False):
    """GNOME Shell dies (SIGKILL, as a crash): the renderer and its worker
    must end on their own, before anything is cleaned up for them."""
    count('shell-restart' if not uninstall else 'uninstall-restart')
    global BASELINE_SHELL
    pid = ctx.pid
    workers = pb.worker_pids()
    if uninstall:
        start = time.monotonic()
        result = private(['gdbus', 'call', '--session', '-d', 'org.gnome.Shell.Extensions', '-o',
                          '/org/gnome/Shell/Extensions', '-m', 'org.gnome.Shell.Extensions.UninstallExtension', UUID]).stdout
        assert 'true' in result, result
        if pid:
            timing('teardown_uninstall_exit_s', renderer_ended(pid, start, 'sigterm', ctx))
        assert not (SESSION / 'data/gnome-shell/extensions' / UUID).exists()
        assert extension_state() is None
        evaluate('globalThis.__omkLabArmed = false; true')
        if ctx.daemon:
            ctx.daemon.stop()
        restored(ctx, f'{label}-uninstalled')
        pid = None
        workers = []
    shell = shell_pid()
    assert shell == BASELINE_SHELL, 'the Shell restarted unexpectedly'
    start = time.monotonic()
    os.kill(shell, signal.SIGKILL)
    if pid:
        elapsed = pb.await_exit(pid, start, 5)
        timing('shell_kill_to_renderer_exit_s', elapsed)
        wait(lambda: all(state(worker) is None for worker in workers), 3, 0.01, 'workers end with the Shell')
        wait(lambda: not pb.play_groups(), 3, 0.02, 'renderer removed its group itself')
    wait(lambda: exited(shell), 5, 0.02, 'GNOME Shell killed')
    if ctx.daemon:
        ctx.daemon.stop()
    stop_fakes()
    time.sleep(2.0)
    keep_shell_log()
    if uninstall:
        install_extension(SESSION / 'data/gnome-shell/extensions')
    pb.run([HARNESS, 'restart', '3'])
    REPORT['shell_sessions'] += 1
    CURRENT['layout'] = 'three'
    prepare_shell()
    assert shell_monitors() == logical('three')
    BASELINE_SHELL = shell_pid()
    error = restored(ctx, f'{label}-new-shell')
    note(f'{label}: Shell restarted (uninstall={uninstall}); native error {error:.3f}')
    return error


# ---------------------------------------------------------------- setup

def generate_media():
    directory = LAB / 'media'
    directory.mkdir()
    paths = media.save_images(directory, 'pattern', *ITEMS['gif']['size'], ITEMS['gif']['frames'], 100)
    files = {'gif': paths['gif'], 'apng': paths['png'], 'webp': paths['webp'],
             'still': media.save_still(directory, *ITEMS['still']['size']),
             'vp9': media.encode_pattern_video(directory / 'pattern-vp9.webm', *ITEMS['vp9']['size'], 30, 30, 'vp9'),
             'av1': media.encode_pattern_video(directory / 'pattern-av1.webm', *ITEMS['av1']['size'], 30, 30, 'av1'),
             'clip1080p': media.encode_clip_1080p(directory / 'clip-1080p30.webm')}
    frames = [media.pattern(*ITEMS['longgif']['size'], index, ITEMS['longgif']['frames']) for index in range(4)]
    long_path = directory / 'long-frames.gif'
    frames[0].save(long_path, save_all=True, append_images=frames[1:], duration=ITEMS['longgif']['durations'],
                   loop=0, optimize=False, disposal=1)
    files['longgif'] = long_path
    for item in ('gif', 'apng', 'webp', 'longgif'):
        decoded = media.decoded_frames(files[item])
        assert len(decoded) == ITEMS[item]['frames']
        assert all(frame.tobytes() == pb.reference(item, index).tobytes() for index, frame in enumerate(decoded)), item
    return files


def import_media(files, runtime):
    pb.LIBRARY.mkdir(mode=0o700)
    formats = {'gif': 'gif', 'apng': 'apng', 'webp': 'webp', 'still': 'still_png', 'vp9': 'vp9_webm',
               'av1': 'av1_webm', 'clip1080p': 'vp9_webm', 'longgif': 'gif'}
    for item, path in files.items():
        result = pb.run([BIN / 'one-mk', 'wallpaper', 'import', path, '--runtime', runtime, '--output', pb.LIBRARY,
                         '--cgroup-root', CGROUP / 'import', '--json'], check=False, timeout=600)
        assert result.returncode == 0, (item, result.stdout, result.stderr)
        record = json.loads(result.stdout)
        assert record['media']['format'] == formats[item], record
        assert record['media']['frames'] == ITEMS[item]['frames'], record
        pb.REPORT['media'][item] = {'hash': record['source_blake3'], 'bytes': record['source_bytes'], **record['media']}
    check('all eight generated items imported by the real sandboxed importer (delegated cgroup v2 of this run\'s '
          'own transient systemd scope): ' + ', '.join(f'{item}={value["format"]}/{value["frames"]}f'
                                                         for item, value in pb.REPORT['media'].items()))


def user_wallpaper():
    """The user's own wallpaper for this private session: a generated picture
    nothing of ours resembles."""
    image = Image.new('RGB', (1920, 1080))
    draw = ImageDraw.Draw(image)
    for y in range(1080):
        draw.line([(0, y), (1919, y)], fill=(20 + y // 9, 60 + y // 12, 140 - y // 10))
    for index in range(9):
        draw.ellipse([120 + index * 200, 300 + (index % 3) * 150, 260 + index * 200, 440 + (index % 3) * 150],
                     fill=(230, 200 - index * 15, 60 + index * 20))
    path = LAB / 'user-wallpaper.png'
    image.save(path)
    return 'file://' + str(path)


def shell_pid():
    return next(process for process, comm in lab_processes().items() if comm == 'gnome-shell')


BASELINE_SHELL = None


def install_extension(root):
    destination = root / UUID
    shutil.copytree(pb.REPO / 'extensions/gnome' / UUID, destination)
    (destination / 'libexec').mkdir()
    wrapper = destination / 'libexec/one-mk-wall'
    wrapper.write_text(f'#!/bin/sh\nexport ONE_MK_WALL_M1_PLAYBACK="{LAB / "playback.json"}"\n'
                       f'exec >>"{pb.RENDERER_LOG}" 2>&1\nexec "{BIN / "one-mk-wall"}" --present=gnome --m1-lab\n')
    wrapper.chmod(0o700)


def capture_baselines():
    for name in LAYOUTS:
        apply_layout(name)
        previous = None
        for _ in range(20):
            time.sleep(0.6)
            shot = screenshot(f'baseline-{name}')
            if previous is not None and ImageChops.difference(body(shot, name), body(previous, name)).getbbox() is None:
                break
            previous = shot
        else:
            raise AssertionError(f'native background for {name} never settles')
        BASELINES[name] = shot
        shot.save(SHOTS / f'baseline-{name}.png')
    check('user wallpaper set in the private session and native baselines captured for all '
          f'{len(LAYOUTS)} layouts ({", ".join(f"{name} {physical_size(name)[0]}x{physical_size(name)[1]}" for name in LAYOUTS)})')


# ---------------------------------------------------------------- directed proofs

def start_presentation(label, item, budget=True):
    ctx = Context(label)
    ctx.item = item
    write_settings_next()
    ctx.daemon = Daemon(item)
    ctx.offset = pb.log_size()
    arm(reset_budget=budget)
    settled(ctx, label=label, timeout=8)
    return ctx


def proof_layouts():
    apply_layout('two')
    ctx = start_presentation('layouts-gif', 'gif')
    results = []
    for target in ['three', 'one', 'two@2', 'two@1.25', 'swapped', 'rotated', 'stacked', 'column3', 'two', 'three', 'two']:
        seconds = event_layout(ctx, target)
        results.append({'layout': target, 'what': DESCRIPTION[target], 'fits_canvas': fits(target),
                        'seconds': round(seconds, 3)})
    event_layout_burst(ctx)
    apply_layout_settled(ctx, 'two')
    pid = ctx.pid
    teardown(ctx, 'stop')
    REPORT['layout_matrix'] = results
    worst = max(entry['seconds'] for entry in results if entry['fits_canvas'])
    check(f'layout change / hot-plug with real GIF playback (same renderer PID {pid} throughout): '
          + '; '.join(f'{entry["what"]} -> {"revealed in" if entry["fits_canvas"] else "static user wallpaper in"} '
                      f'{entry["seconds"]:.2f}s' for entry in results)
          + ' (each revealed layout then verified exact on every pixel at scale 1, at block centres when scaled)'
          + f'; transition screenshots never showed a frame sliced for the old layout; worst {worst:.2f}s; '
            f'a 3-change burst (150 ms apart) converged; renderer exit and restoration after stop')
    for item in ('vp9', 'still'):
        ctx = start_presentation(f'layouts-{item}', item)
        for target in ['three', 'two@1.25', 'swapped', 'two']:
            event_layout(ctx, target)
        teardown(ctx, 'stop')
    check('VP9 (PSNR) and the still (exact, holding) re-slice through add, fractional scale, reposition and back')


def apply_layout_settled(ctx, name):
    if CURRENT['layout'] != name:
        event_layout(ctx, name)


def proof_lock():
    apply_layout('two')
    ctx = start_presentation('lock-vp9', 'vp9')
    for _ in range(6):
        event_lock(ctx)
    current = view()
    assert not current['exhausted'] and current['attempts'] <= 1, current
    teardown(ctx, 'stop')
    unlocks = REPORT['timings']['unlock_to_settled_s'][-6:]
    exits = REPORT['timings']['lock_to_renderer_exit_s'][-6:]
    check(f'lock/unlock x6 within about a minute with VP9: at lock the renderer exits via SIGTERM in '
          f'{min(exits):.0f}-{max(exits):.0f} ms (worker and cgroup removed by the renderer itself); while locked '
          f'no bridge, renderer, decoder, cgroup or timer exists and the lock screen shows no frame; each unlock '
          f'starts a fresh renderer showing verified advancing frames in {min(unlocks):.0f}-{max(unlocks):.0f} ms; '
          f'the restart cap is not consumed by lock cycles (attempts {current["attempts"]}, not exhausted)')


def proof_suspend():
    apply_layout('two')
    ctx = start_presentation('suspend-apng', 'apng')
    event_suspend(ctx, with_lock=False)
    event_suspend(ctx, with_lock=False)
    event_suspend(ctx, with_lock=True)
    teardown(ctx, 'stop')
    sleeps = REPORT['timings']['sleep_to_renderer_exit_s'][-3:]
    wakes = REPORT['timings']['wake_to_settled_s'][-3:]
    check(f'suspend/resume via PrepareForSleep on the private system bus with APNG: before sleep the presentation '
          f'is hidden and the renderer exits (SIGTERM, {min(sleeps):.0f}-{max(sleeps):.0f} ms, no worker or cgroup '
          f'left); asleep the screen is the user wallpaper with no restart/watchdog timers; after resume a fresh '
          f'renderer shows verified advancing frames ({min(wakes):.0f}-{max(wakes):.0f} ms), both with and without '
          f'the Shell\'s lock-on-suspend')


def thread_switches(pid):
    result = {}
    for task in Path(f'/proc/{pid}/task').iterdir():
        name = (task / 'comm').read_text().strip()
        values = {}
        for line in (task / 'status').read_text().splitlines():
            key, _, value = line.partition(':')
            if key in ('voluntary_ctxt_switches', 'nonvoluntary_ctxt_switches'):
                values[key] = int(value)
        result[f'{name}/{task.name}'] = values['voluntary_ctxt_switches'] + values['nonvoluntary_ctxt_switches']
    return result


def proof_holding():
    apply_layout('two')
    ctx = start_presentation('holding-still', 'still')
    current = view()
    assert current['state'] == 4 and 'watchdog' not in current['sources'], current
    pid = ctx.pid
    assert any('event=holding' in line for line in pb.lines_for(pid, ctx.offset))
    first = screenshot('holding-first', keep=True)
    before = thread_switches(pid)
    time.sleep(12)
    after = thread_switches(pid)
    later = screenshot('holding-later', keep=True)
    assert ImageChops.difference(body(first, 'two'), body(later, 'two')).getbbox() is None
    current = view()
    assert renderer_alive(pid) and current['layer'] and current['state'] == 4 and 'watchdog' not in current['sources'], current
    deltas = {name: after[name] - before[name] for name in after if name in before}
    presenter = sum(value for name, value in deltas.items() if name.startswith('presenter/'))
    assert presenter == 0, deltas
    REPORT['holding'] = {'thread_switches_12s': deltas, 'view': current}
    teardown(ctx, 'stop')
    ctx = start_presentation('holding-longgif', 'longgif')
    pid = ctx.pid
    time.sleep(12)
    lines = pb.lines_for(pid, ctx.offset)
    holds = sum('event=holding' in line for line in lines)
    moves = sum('event=moving' in line for line in lines)
    assert renderer_alive(pid) and holds >= 3 and moves >= 3, (holds, moves)
    verify(ctx, 'holding-longgif', keep=True)
    REPORT['holding']['long_frames'] = {'holding_reports': holds, 'moving_reports': moves}
    teardown(ctx, 'stop')
    check(f'stills and long frames announce Holding instead of being re-committed: the still stays revealed 12 s '
          f'(4x the Shell watchdog) unchanged in state 4 with no Shell watchdog source and zero presenter-thread '
          f'wakeups (renderer thread switches over 12 s: {deltas}); a GIF with 2.6 s frames alternates '
          f'Holding/Playing ({holds}/{moves} in 12 s) and is never stopped by the watchdog')


def proof_failure_cap():
    apply_layout('two')
    ctx = start_presentation('cap-gif', 'gif')
    for _ in range(2):
        event_renderer_kill(ctx)
    pid = ctx.pid
    os.kill(pid, signal.SIGKILL)
    wait(lambda: exited(pid), 2, 0.005, 'renderer killed')
    current = wait(lambda: (lambda v: v if v and v['exhausted'] else None)(view()), 8, 0.1, 'restart cap reached')
    assert current['pid'] is None and not current['layer'] and current['children'] == 0, current
    assert current['lastStop']['cause'] == 'renderer exited' and not current['lastStop']['planned'], current
    wait(lambda: view()['escalations'] == 0, 3, 0.05, 'no escalation timer for an exited renderer')
    assert native_error(screenshot('cap-static'), 'two') < 0.5
    # The killed renderer could not remove its worker's group: empty, and
    # reclaimed by the next renderer at start.
    stale = pb.play_groups()
    assert len(stale) == 1 and not list(stale.values())[0], stale
    disarm()
    ctx.daemon.stop()
    ctx.daemon = None
    ctx.pid = None
    reclaim_check()
    restored(ctx, 'cap')
    check('renderer crashes still count: two SIGKILLs are each followed by a fresh verified renderer (Shell restart '
          'after 2 s, then 4 s), the third within the minute latches the restart cap; the screen is the user wallpaper; '
          'the one empty group the killed renderer left is reclaimed by the next renderer at start')


def proof_restoration():
    REPORT['restoration'] = {}
    apply_layout('two')
    for method, item in (('stop', 'gif'), ('sigterm', 'vp9'), ('sigkill', 'av1'), ('revoke', 'webp'), ('disable', 'apng'),
                         ('renderer-kill', 'vp9'), ('extension-disable', 'clip1080p')):
        ctx = start_presentation(f'restore-{method}', item)
        error = teardown(ctx, method)
        REPORT['restoration'][method] = round(error, 3)
    ctx = start_presentation('restore-uninstall', 'gif')
    REPORT['restoration']['uninstall'] = round(restart_shell(ctx, 'restore-uninstall', uninstall=True), 3)
    apply_layout('two')
    ctx = start_presentation('restore-shell', 'vp9')
    REPORT['restoration']['shell-restart'] = round(restart_shell(ctx, 'restore-shell'), 3)
    apply_layout('two')
    ctx = start_presentation('after-restart', 'gif')
    teardown(ctx, 'stop')
    check('restoration: after daemon one-mk stop, SIGTERM, SIGKILL, consent revoke, wallpaper disable, bare renderer '
          'SIGKILL, extension disable, extension uninstall and GNOME Shell SIGKILL + restart, the screen equals the '
          'user\'s own wallpaper (mean error ' + ', '.join(f'{k} {v}' for k, v in REPORT['restoration'].items())
          + '), picture-uri untouched, no renderer, worker, cgroup or other process left; playback works again '
          'in the restarted Shell')


def proof_flicker():
    """Diagnostic only (M1_ONLY=flicker): screenshots every ~0.1 s around
    holds, with and without a preceding lock/unlock."""
    apply_layout('one')
    ctx = start_presentation('flicker', 'longgif')
    samples = []
    for phase in ('plain', 'after-unlock'):
        if phase == 'after-unlock':
            event_lock(ctx)
        end = time.monotonic() + 8
        while time.monotonic() < end:
            shot = screenshot('flicker')
            kind = media_on_screen('longgif', shot, 'one')
            current = view()
            samples.append((phase, round(time.monotonic(), 2), kind, current['state'], current['layer'], current['fresh'],
                            current['clones'], current['pid']))
            if kind != 'media':
                shot.save(SHOTS / f'flicker-{len(samples):04d}.png')
    REPORT['flicker'] = samples
    teardown(ctx, 'stop')
    bad = [sample for sample in samples if sample[2] != 'media']
    assert not bad, bad


# ---------------------------------------------------------------- randomized sequences

EVENT_WEIGHTS = {'layout': 5, 'layout-burst': 1, 'lock': 3, 'suspend': 2, 'suspend-lock': 1, 'renderer-kill': 1,
                 'pause': 2, 'extension-cycle': 2}
TEARDOWN_WEIGHTS = {'stop': 15, 'sigterm': 14, 'sigkill': 14, 'revoke': 11, 'disable': 9, 'renderer-kill': 11,
                    'extension-disable': 12, 'shell-restart': 7, 'uninstall': 4}


def pick(weights):
    return random.choices(list(weights), weights=list(weights.values()))[0]


def sequence(index):
    if not fits(CURRENT['layout']):
        apply_layout(random.choice([name for name in LAYOUTS if fits(name)]))
    item = random.choice(list(ITEMS))
    label = f'seq{index:03d}'
    events = []
    kills = 0
    for _ in range(random.randint(2, 4)):
        event = pick(EVENT_WEIGHTS)
        if event == 'renderer-kill':
            if kills:
                event = 'lock'
            kills += 1
        events.append(event)
    method = pick(TEARDOWN_WEIGHTS)
    record = {'sequence': index, 'item': item, 'layout_start': CURRENT['layout'], 'events': [], 'teardown': method}
    note(f'{label}: item={item} layout={CURRENT["layout"]} events={events} teardown={method}')
    ctx = start_presentation(label, item)
    for event in events:
        if event == 'layout':
            target = random.choice([name for name in LAYOUTS if name != CURRENT['layout']])
            event_layout(ctx, target)
            record['events'].append(f'layout:{target}')
            continue
        record['events'].append(event)
        if ctx.static_expected() and event in ('renderer-kill', 'pause', 'layout-burst'):
            # Nothing is presented in an unfit layout; bring a fitting one back first.
            target = random.choice([name for name in LAYOUTS if fits(name)])
            event_layout(ctx, target)
            record['events'].append(f'layout:{target}')
        {'layout-burst': event_layout_burst, 'lock': event_lock, 'suspend': lambda c: event_suspend(c, False),
         'suspend-lock': lambda c: event_suspend(c, True), 'renderer-kill': event_renderer_kill,
         'pause': event_pause, 'extension-cycle': event_extension_cycle}[event](ctx)
    if method in ('shell-restart', 'uninstall'):
        restart_shell(ctx, label, uninstall=method == 'uninstall')
    else:
        teardown(ctx, method)
    record['layout_end'] = CURRENT['layout']
    REPORT['sequences'].append(record)
    pb.evaluate('Main.extensionManager.lookup("spike-harness@one-mk.test").stateObj.collectGarbage(); true')


# ---------------------------------------------------------------- main

GATE = pb.runpy.run_path(str(HERE / 'm1-risk-probe.py'))


def identity(record):
    """The M1a-2 gate's warning identity, plus D-Bus connection numbers
    wherever they appear (the gate only normalizes word-bounded ones, so
    'The name :1.17' of a later session differed from ':1.16')."""
    return re.sub(r':1\.\d+', ':1.CONNECTION', GATE['warning_identity'](record))


def records_between(log, start, end=None):
    return GATE['warning_records'](log[start:end])


CONTROL = {}


def control_phase():
    """Harness controls with nothing of ours enabled or running: the Shell's
    own reaction to the stand-in logind's PrepareForSleep, a lock/unlock, and
    a Shell SIGKILL + restart. Warning records seen here are attributable to
    the harness, not to the wallpaper component."""
    log_path = SESSION / 'log/shell.log'
    start = len(log_path.read_text())
    set_lock_on_suspend(False)
    for _ in range(2):
        simulate_sleep(True)
        time.sleep(0.8)
        simulate_sleep(False)
        time.sleep(0.8)
    set_lock_on_suspend(True)
    simulate_sleep(True)
    wait(lambda: locked(), 5, 0.05, 'control: locked on suspend')
    simulate_sleep(False)
    time.sleep(0.5)
    unlock()
    wait(lambda: not locked(), 5, 0.05, 'control: unlocked')
    lock()
    wait(lambda: locked(), 5, 0.05, 'control: locked')
    unlock()
    wait(lambda: not locked(), 5, 0.05, 'control: unlocked')
    time.sleep(1.0)
    suspend_records = records_between(log_path.read_text(), start)
    # A Shell crash and restart with nothing of ours present.
    evaluate('Main.overview.hide(); true')
    shell = shell_pid()
    os.kill(shell, signal.SIGKILL)
    wait(lambda: exited(shell), 5, 0.02, 'control: Shell killed')
    stop_fakes()
    time.sleep(2.0)
    keep_shell_log()
    restart_records = records_between((LAB / f'shell-{REPORT["shell_sessions"]}.log').read_text(), start)
    restart_records = [record for record in restart_records if record not in suspend_records]
    pb.run([HARNESS, 'restart', '3'])
    REPORT['shell_sessions'] += 1
    CURRENT['layout'] = 'three'
    CONTROL['records'] = suspend_records + restart_records
    CONTROL['identities'] = {identity(record) for record in CONTROL['records']}
    REPORT['control_warnings'] = {'suspend_and_lock_without_component': suspend_records,
                                  'shell_kill_without_component': restart_records}
    check(f'harness control without the component (extension disabled, no daemon or renderer): the stand-in '
          f'logind\'s PrepareForSleep, lock/unlock and a Shell SIGKILL + restart produce {len(suspend_records)} + '
          f'{len(restart_records)} warning records of their own; these identities are reported separately and are not '
          f'charged to, nor hidden from, the component gate below')


def shell_record(record):
    """Whether a warning record comes from GNOME Shell/Mutter itself (the
    gnome-shell process: GNOME Shell, libmutter, Gjs, JS, Clutter...), as
    opposed to another session service writing to the same log (for example
    evolution-*-factory or xdg-desktop-portal). Records without a program
    prefix count as Shell records (strict)."""
    match = re.match(r'\(([^:()]+):\d+\)', record.splitlines()[0])
    return match is None or os.path.basename(match.group(1)) in ('gnome-shell', 'mutter')


def gate_warnings():
    """Gate: zero new Mutter/Shell warning records versus the M1a-2 baseline,
    apart from identities the control phase produced without the component.
    Other session services' new records are reported, attributed, not hidden."""
    baseline = (HERE / 'm1a2-evidence/baseline-shell.log').read_text()
    known = {identity(record) for record in GATE['warning_records'](baseline)}
    control = CONTROL.get('identities', set())
    keep_shell_log()
    results = {}
    for index in range(1, REPORT['shell_sessions'] + 1):
        records = GATE['warning_records']((LAB / f'shell-{index}.log').read_text())
        unknown = [record for record in records if identity(record) not in known]
        results[index] = {'records': len(records),
                          'harness_control': [record for record in unknown if identity(record) in control],
                          'new': [record for record in unknown if identity(record) not in control and shell_record(record)],
                          'new_other_services': [record for record in unknown
                                                 if identity(record) not in control and not shell_record(record)]}
    REPORT['warnings'] = results
    new = [record for value in results.values() for record in value['new']]
    assert not new, new
    others = [record for value in results.values() for record in value['new_other_services']]
    assert not [record for record in others if 'one-mk' in record.splitlines()[0]], others
    harness = sum(len(value['harness_control']) for value in results.values())
    check(f'zero new Mutter/Shell warning records versus the M1a-2 baseline across all {len(results)} Shell '
          f'sessions ({sum(value["records"] for value in results.values())} records: baseline-known, or {harness} '
          f'of the identities the control phase produced without the component); {len(others)} new records from '
          f'other session services (not Shell/Mutter, never ours) are listed in result.json')

def main():
    global WALLPAPER_URI, BASELINE_SHELL
    random.seed(SEED)
    LAB.mkdir(mode=0o700)
    (LAB / 'home').mkdir()
    SHOTS.mkdir()
    sources = LAB / 'extensions'
    sources.mkdir()
    started = False
    cgroups = False
    try:
        for binary in ('one-mk', 'one-mk-wall', 'one-mk-wall-import'):
            os.chmod(BIN / binary, 0o755)
        REPORT['binary_sha256'] = {binary: hashlib.sha256((BIN / binary).read_bytes()).hexdigest()
                                   for binary in ('one-mk', 'one-mk-wall', 'one-mk-wall-import')}
        REPORT['source_sha256'] = {str(path.relative_to(pb.REPO)): hashlib.sha256(path.read_bytes()).hexdigest()
                                   for path in [pb.REPO / 'extensions/gnome' / UUID / 'extension.js', Path(__file__),
                                                HERE / 'client/fake_logind.py', HERE / 'isolated-session.sh']}
        REPORT['seed'] = SEED
        pb.prepare_cgroups()
        cgroups = True
        runtime = pb.prepare_runtime()
        files = generate_media()
        import_media(files, runtime)
        pb.write_playback_config(runtime)
        shutil.copytree(HERE / 'ext/spike-harness@one-mk.test', sources / 'spike-harness@one-mk.test')
        install_extension(sources)
        pb.ENV['EXT_SRC'] = str(sources)
        REPORT['shell'] = pb.run(['gnome-shell', '--version']).stdout.strip()
        assert pb.run([BIN / 'one-mk-wall', '--present=gnome'], check=False).returncode == 3
        WALLPAPER_URI = user_wallpaper()
        pb.run([HARNESS, 'start', '3'])
        started = True
        pb.extension('Enable', 'spike-harness@one-mk.test')
        evaluate('Main.overview.hide(); true')
        start_fakes()
        control_phase()
        prepare_shell()
        BASELINE_SHELL = shell_pid()
        assert background_settings() == [WALLPAPER_URI, WALLPAPER_URI, 'zoom']
        capture_baselines()
        apply_layout('two')
        only = [name for name in os.environ.get('M1_ONLY', '').split(',') if name]
        REPORT['proofs_run'] = only or 'all'
        for name, proof in (('layouts', proof_layouts), ('lock', proof_lock), ('suspend', proof_suspend),
                            ('holding', proof_holding), ('cap', proof_failure_cap), ('restoration', proof_restoration),
                            ('flicker', proof_flicker)):
            if (not only and name != 'flicker') or name in only:
                proof()
        for index in range(SEQUENCES):
            sequence(index)
            if index % 10 == 9:
                print(f'sequence {index + 1}/{SEQUENCES}', flush=True)
        REPORT['event_counts'] = REPORT['events']
        check(f'{SEQUENCES} randomized lifecycle sequences (seed {SEED}; events {json.dumps(REPORT["events"])}): every '
              f'sequence started real playback of a random imported item, every event ended in verified frames or the '
              f'expected static user wallpaper within its bound, every teardown restored the user wallpaper with no '
              f'leftover process or cgroup')
        gate_warnings()
        REPORT['timing_summary'] = {name: {'count': len(values), 'median_ms': statistics.median(values),
                                           'max_ms': max(values)} for name, values in REPORT['timings'].items()}
        REPORT['passed'] = True
    except BaseException as error:
        REPORT['passed'] = False
        REPORT['failure'] = f'{type(error).__name__}: {error}'
        REPORT['traceback'] = traceback.format_exc()
        note('FAIL ' + REPORT['failure'])
        try:
            screenshot('failure', keep=True)
            REPORT['failure_view'] = view()
        except BaseException:
            pass
        raise
    finally:
        stop_fakes()
        if (SESSION / 'log/shell.log').exists():
            shutil.copy2(SESSION / 'log/shell.log', LAB / 'shell.log')
        if started:
            pb.run([HARNESS, 'stop'], check=False)
        REPORT['remaining_lab_processes'] = lab_processes()
        if cgroups:
            try:
                REPORT['remaining_play_groups'] = pb.play_groups()
                pb.remove_cgroups()
            except OSError as error:
                REPORT['cgroup_cleanup_error'] = str(error)
        shutil.rmtree(sources, ignore_errors=True)
        (LAB / 'result.json').write_text(json.dumps(REPORT, indent=2, default=str) + '\n')
        print(json.dumps({key: value for key, value in REPORT.items()
                          if key not in ('checks', 'sequences', 'timings', 'media', 'traceback')}, indent=2, default=str))


if __name__ == '__main__':
    sys.exit(main())
