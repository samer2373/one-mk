// one-mk live wallpaper bridge: FEASIBILITY SPIKE, not product code.
//
// Independently written for one-mk (MIT OR Apache-2.0). It only *presents*
// pixels an external renderer already drew: there is no decoder, parser or
// network code in here.
//
// Layering: Main.layoutManager._backgroundGroup is the first (lowest) child
// of global.window_group; the Shell always lowers its own BackgroundActors to
// the bottom of that group, so our layer (appended to the group) sits above
// the static wallpaper and below every window actor, DING's desktop windows
// included.
//
// Frames: the renderer is a plain Wayland toplevel spawned through
// Meta.WaylandClient, so ownership of its windows is proven by the Wayland
// connection (owns_window), not by title or app-id. Its window actor is
// hidden and a Clutter.Clone of it is painted, cropped per monitor, inside
// our background layer.
//
// Test hooks (start/stop/mode) are called through Eval from the isolated test
// session only; the D-Bus surface is the minimal one sketched in the spike
// report.

import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'org.onemk.WallpaperBridgeSpike1';
const OBJ_PATH = '/org/onemk/WallpaperBridgeSpike1';
const PROTOCOL = 1;
const STALE_US = 3 * GLib.USEC_PER_SEC;
const IFACE = `
<node>
  <interface name="${BUS_NAME}">
    <method name="Hello">
      <arg type="u" direction="in" name="version"/>
      <arg type="u" direction="out" name="version"/>
      <arg type="a(iiii)" direction="out" name="monitors"/>
    </method>
    <method name="FrameReady">
      <arg type="u" direction="in" name="seq"/>
    </method>
    <signal name="PauseHint">
      <arg type="b" name="paused"/>
      <arg type="s" name="reason"/>
    </signal>
    <signal name="Layout">
      <arg type="a(iiii)" name="monitors"/>
    </signal>
  </interface>
</node>`;

// Software fallback (spike step 2c): fixed size, fixed file, no paths from
// the caller.
const IMG_W = 1920, IMG_H = 1080, IMG_BYTES = IMG_W * IMG_H * 4;

class Bridge {
    constructor() {
        this._mode = 'clone';      // 'color' | 'clone' | 'image'
        this._hideMode = 'hide';   // 'hide' | 'opacity' | 'none'
        this._setType = true;
        this._signals = [];
        this._monitorBoxes = [];
        this._clones = [];
        this._wc = null;
        this._renderer = null;     // {win, actor, lastDamage, firstDamage}
        this._paused = false;
        this._reason = 'none';
        this._stale = false;
        this._coverageId = 0;
        this._watchdogId = 0;
        this.stats = {hints: [], damage: 0, imageFrames: 0, imageUs: 0, events: []};

        this._layer = new Clutter.Actor({name: 'one-mk-wallpaper-layer', reactive: false});
        // We may be (re-)enabled while already locked: the Shell bounces
        // later extensions when an earlier one is disabled, e.g. on lock.
        this._layer.visible = !Main.sessionMode.isLocked;
        Main.layoutManager._backgroundGroup.add_child(this._layer);

        this._connect(Main.layoutManager, 'monitors-changed', () => {
            this._event('monitors-changed');
            this._rebuild();
            this._emitLayout();
        });
        this._connect(global.display, 'window-created', (_d, win) => this._maybeAdopt(win));
        for (const s of ['restacked', 'in-fullscreen-changed'])
            this._connect(global.display, s, () => this._queueCoverage());
        this._connect(global.workspace_manager, 'active-workspace-changed', () => {
            this._event('workspace-changed');
            this._queueCoverage();
        });
        for (const s of ['showing', 'hidden'])
            this._connect(Main.overview, s, () => {
                this._event(`overview-${s}`);
                this._queueCoverage();
            });
        this._connect(Main.sessionMode, 'updated', () => {
            this._event(`session-mode ${Main.sessionMode.currentMode} locked=${Main.sessionMode.isLocked}`);
            this._layer.visible = !Main.sessionMode.isLocked;
            this._queueCoverage();
        });
        if (Main.screenShield) {
            this._connect(Main.screenShield, 'locked-changed', () => {
                this._event(`locked-changed ${Main.screenShield.locked}`);
                this._queueCoverage();
            });
        }

        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJ_PATH);
        this._nameId = Gio.bus_own_name_on_connection(Gio.DBus.session, BUS_NAME,
            Gio.BusNameOwnerFlags.DO_NOT_QUEUE, null, () => this._event('lost bus name'));

        this._rebuild();
    }

    destroy() {
        this.stopRenderer();
        if (this._coverageId)
            GLib.source_remove(this._coverageId);
        this._coverageId = 0;
        this._stopWatchdog();
        for (const [obj, id] of this._signals)
            obj.disconnect(id);
        this._signals = [];
        Gio.bus_unown_name(this._nameId);
        this._dbus.unexport();
        this._dbus = null;
        this._layer.destroy();
        this._layer = null;
        this._monitorBoxes = [];
        this._clones = [];
    }

    _connect(obj, sig, fn) {
        this._signals.push([obj, obj.connect(sig, fn)]);
    }

    _event(s) {
        this.stats.events.push(`${GLib.get_monotonic_time() / 1000 | 0} ${s}`);
        if (this.stats.events.length > 200)
            this.stats.events.shift();
    }

    // ---------- test hooks (Eval only) ----------
    setMode(mode, hideMode = 'hide') {
        this._mode = mode;
        this._stale = false;
        if (this._renderer)
            this._renderer.lastDamage = GLib.get_monotonic_time();
        this._hideMode = hideMode;
        if (this._renderer)
            this._applyHide();
        this._rebuild();
        return `${this._mode}/${this._hideMode}`;
    }

    startRenderer(argv) {
        this.stopRenderer();
        const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.NONE});
        this._wc = Meta.WaylandClient.new_subprocess(global.context, launcher, argv);
        const proc = this._wc.get_subprocess();
        const pid = Number(proc.get_identifier());
        this._pid = pid;
        this._event(`spawned renderer pid=${pid}`);
        proc.wait_async(null, () => {
            this._event(`renderer exited pid=${pid}`);
            if (this._wc && this._wc.get_subprocess() === proc)
                this._dropRenderer('exited');
        });
        return this._pid;
    }

    stopRenderer() {
        const proc = this._wc?.get_subprocess();
        this._dropRenderer('stopped');
        this._wc = null;
        proc?.send_signal(15);
    }

    // ---------- renderer window adoption ----------
    _maybeAdopt(win) {
        // Only windows on the connection we created ourselves.
        if (!this._wc || !this._wc.owns_window(win))
            return;
        // A DESKTOP-type window cannot be resized once mapped, so on a layout
        // change the renderer opens a new toplevel at the new canvas size;
        // the newest owned window replaces the previous one.
        if (this._renderer)
            this._dropRenderer('replaced by new renderer window');
        const actor = win.get_compositor_private();
        if (!actor) {
            this._event('adopt: no actor yet');
            return;
        }
        win.hide_from_window_list();
        // DESKTOP type: exempt from the work-area constraint (else Mutter
        // keeps it under the top bar) and stacked in the desktop layer.
        // Must happen before the first configure: the size is fixed then.
        if (this._setType)
            win.set_type(Meta.WindowType.DESKTOP);
        const r = {win, actor, lastDamage: GLib.get_monotonic_time()};
        this._renderer = r;
        // connectObject/disconnectObject (Shell signal tracker) also drops
        // the handlers when Mutter destroys the actor first (client killed).
        actor.connectObject(
            'damaged', () => {
                this.stats.damage++;
                if (!r.firstDamage) {
                    r.firstDamage = true;
                    this._event('first frame from renderer');
                }
                r.lastDamage = GLib.get_monotonic_time();
                if (this._stale) {
                    this._stale = false;
                    this._event('renderer fresh again');
                    this._rebuild();
                }
                this._probeBackoff = 5;
            },
            // Re-assert after Shell's map animation or any later show().
            'notify::visible', () => this._applyHide(),
            'notify::opacity', () => this._applyHide(),
            'destroy', () => this._dropRenderer('actor destroyed'),
            r);
        win.connectObject(
            'unmanaged', () => this._dropRenderer('unmanaged'),
            'notify::is-alive', () => this._event(`is-alive ${win.is_alive}`),
            'raised', () => win.lower(),
            // Geometry changes (our own resize, hot-plug): re-crop the clones.
            'size-changed', () => this._rebuild(),
            'position-changed', () => this._rebuild(),
            r);
        // Stack operations only once Mutter has put the window in the stack.
        global.compositor.get_laters().add(Meta.LaterType.BEFORE_REDRAW, () => {
            if (this._renderer === r) {
                win.stick();
                win.lower();
            }
            return GLib.SOURCE_REMOVE;
        });
        this._placeRenderer();
        this._applyHide();
        this._event(`adopted window "${win.get_title()}" pid=${win.get_pid()}`);
        this._rebuild();
        this._startWatchdog();
        this._queueCoverage();
    }

    _canvas() {
        const ms = Main.layoutManager.monitors;
        const x = Math.min(...ms.map(m => m.x)), y = Math.min(...ms.map(m => m.y));
        const x2 = Math.max(...ms.map(m => m.x + m.width)), y2 = Math.max(...ms.map(m => m.y + m.height));
        return {x, y, width: x2 - x, height: y2 - y};
    }

    _placeRenderer() {
        const c = this._canvas();
        this._renderer?.win.move_resize_frame(false, c.x, c.y, c.width, c.height);
    }

    _applyHide() {
        const a = this._renderer?.actor;
        if (!a || this._inHide)
            return;
        this._inHide = true;
        if (this._hideMode !== 'opacity' && a.opacity !== 255)
            a.opacity = 255;
        if (this._hideMode === 'hide' && a.visible)
            a.hide();
        else if (this._hideMode === 'opacity' && a.opacity !== 0)
            a.opacity = 0;
        else if (this._hideMode === 'none' && !a.visible)
            a.show();
        this._inHide = false;
    }

    _dropRenderer(why) {
        const r = this._renderer;
        this._renderer = null;
        this._stopWatchdog();
        if (r) {
            r.actor.disconnectObject(r);
            r.win.disconnectObject(r);
            this._event(`renderer dropped: ${why}`);
        }
        this._rebuild();
    }

    // Damage watchdog: runs only while a renderer is adopted. A clone that
    // is not painted gets no frame callbacks, so a stale renderer can never
    // prove it recovered unless we show it again: re-probe with back-off
    // (5 s, 10 s, ... 60 s), and ask Mutter's xdg_wm_base ping (check_alive)
    // whether the client is hung.
    _startWatchdog() {
        if (this._watchdogId)
            return;
        this._probeBackoff = 5;
        this._watchdogId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
            const r = this._renderer;
            if (!r)
                return GLib.SOURCE_CONTINUE;
            const now = GLib.get_monotonic_time();
            if (this._stale) {
                if (now >= this._nextProbe) {
                    this._event(`probe renderer (backoff ${this._probeBackoff} s)`);
                    this._probeBackoff = Math.min(this._probeBackoff * 2, 60);
                    this._stale = false;
                    r.lastDamage = now;
                    this._rebuild();
                }
            } else if (!this._paused && now - r.lastDamage > STALE_US) {
                this._stale = true;
                this._nextProbe = now + this._probeBackoff * GLib.USEC_PER_SEC;
                r.win.check_alive(global.display.get_current_time_roundtrip());
                this._event(`renderer stale for ${(now - r.lastDamage) / 1000 | 0} ms: static fallback`);
                this._rebuild();
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    _stopWatchdog() {
        if (this._watchdogId)
            GLib.source_remove(this._watchdogId);
        this._watchdogId = 0;
    }

    // ---------- presentation ----------
    _rebuild() {
        this._layer.destroy_all_children();
        this._monitorBoxes = [];
        this._clones = [];
        for (const m of Main.layoutManager.monitors) {
            const box = new Clutter.Actor({
                x: m.x, y: m.y, width: m.width, height: m.height,
                clip_to_allocation: true, reactive: false,
            });
            this._layer.add_child(box);
            this._monitorBoxes.push(box);
            if (this._mode === 'color') {
                box.add_child(new St.Widget({
                    width: m.width, height: m.height,
                    style: `background-color: ${m.index % 2 ? '#2a9d5c' : '#c0306a'};`,
                }));
            } else if (this._mode === 'clone' && this._renderer && !this._stale) {
                // One source, N cropped views: offset the clone by where the
                // source buffer sits relative to this monitor.
                const b = this._renderer.win.get_buffer_rect();
                const clone = new Clutter.Clone({
                    source: this._renderer.actor,
                    x: b.x - m.x, y: b.y - m.y,
                });
                box.add_child(clone);
                this._clones.push(clone);
                this._queueCoverage();
            } else if (this._mode === 'image') {
                if (!this._image) {
                    this._image = St.ImageContent.new_with_preferred_size(IMG_W, IMG_H);
                }
                box.add_child(new Clutter.Actor({
                    width: m.width, height: m.height, content: this._image,
                }));
            }
            // Anything else: empty box, so the Shell's static wallpaper shows.
        }
    }

    // ---------- coverage / pause hints ----------
    _queueCoverage() {
        if (this._coverageId)
            return;
        this._coverageId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
            this._coverageId = 0;
            this._updateCoverage();
            return GLib.SOURCE_REMOVE;
        });
    }

    _monitorCovered(m) {
        const ws = global.workspace_manager.get_active_workspace();
        for (const actor of global.get_window_actors()) {
            const w = actor.get_meta_window();
            if (!w || w === this._renderer?.win || w.minimized || !w.showing_on_its_workspace())
                continue;
            if (!w.located_on_workspace(ws) || w.get_window_type() !== Meta.WindowType.NORMAL)
                continue;
            if (w.is_fullscreen() && w.get_monitor() === m.index)
                return true;
            const f = w.get_frame_rect();
            if (f.x <= m.x && f.y <= m.y + 64 && f.x + f.width >= m.x + m.width &&
                f.y + f.height >= m.y + m.height)
                return true;   // maximized or larger (top bar allowance)
        }
        return false;
    }

    _updateCoverage() {
        let reason = 'none';
        const covered = Main.layoutManager.monitors.map(m => this._monitorCovered(m));
        // Enforce, do not just advise: an unpainted clone gets its source no
        // frame callbacks, so even a renderer that ignores PauseHint stalls.
        this._clones.forEach((c, i) => (c.visible = !covered[i]));
        if (Main.sessionMode.isLocked || Main.screenShield?.locked)
            reason = 'locked';
        else if (covered.every(Boolean))
            reason = 'covered';
        const paused = reason !== 'none';
        if (paused === this._paused && reason === this._reason)
            return;
        this._paused = paused;
        this._reason = reason;
        if (this._renderer)
            this._renderer.lastDamage = GLib.get_monotonic_time();
        this.stats.hints.push(`${GLib.get_monotonic_time() / 1000 | 0} ${paused} ${reason}`);
        this._dbus?.emit_signal('PauseHint', new GLib.Variant('(bs)', [paused, reason]));
    }

    _monitorsVariant() {
        return Main.layoutManager.monitors.map(m => [m.x, m.y, m.width, m.height]);
    }

    _emitLayout() {
        this._dbus?.emit_signal('Layout', new GLib.Variant('(a(iiii))', [this._monitorsVariant()]));
    }

    // ---------- D-Bus methods (caller must be our own renderer) ----------
    async _callerIsRenderer(invocation) {
        if (!this._pid)
            return false;
        const reply = await Gio.DBus.session.call('org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', 'GetConnectionUnixProcessID',
            new GLib.Variant('(s)', [invocation.get_sender()]), new GLib.VariantType('(u)'),
            Gio.DBusCallFlags.NONE, 1000, null);
        const [pid] = reply.deepUnpack();
        return pid === this._pid;
    }

    async HelloAsync([version], invocation) {
        try {
            if (!await this._callerIsRenderer(invocation)) {
                invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied', 'not our renderer');
                return;
            }
            if (version !== PROTOCOL) {
                invocation.return_dbus_error('org.freedesktop.DBus.Error.NotSupported', 'protocol');
                return;
            }
            invocation.return_value(new GLib.Variant('(ua(iiii))', [PROTOCOL, this._monitorsVariant()]));
        } catch (e) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', 'internal');
        }
    }

    // Step 2c measurement only: copy a fixed-size RGBA frame from a fixed
    // file in our runtime dir into an St.ImageContent (CPU path).
    async FrameReadyAsync([_seq], invocation) {
        try {
            if (!await this._callerIsRenderer(invocation)) {
                invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied', 'not our renderer');
                return;
            }
            invocation.return_value(null);
            if (this._mode !== 'image' || !this._image)
                return;
            const t0 = GLib.get_monotonic_time();
            const path = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'one-mk-spike-frame.rgba']);
            const [ok, bytes] = GLib.file_get_contents(path);
            if (!ok || bytes.length !== IMG_BYTES)
                return;
            const ctx = global.stage.context.get_backend().get_cogl_context();
            this._image.set_bytes(ctx, new GLib.Bytes(bytes), Cogl.PixelFormat.RGBA_8888, IMG_W, IMG_H, IMG_W * 4);
            this.stats.imageFrames++;
            this.stats.imageUs += GLib.get_monotonic_time() - t0;
        } catch (e) {
            this._event(`FrameReady error ${e.message}`);
        }
    }
}

export default class WallpaperSpike extends Extension {
    enable() {
        this.bridge = new Bridge();
    }

    disable() {
        this.bridge?.destroy();
        this.bridge = null;
    }
}
