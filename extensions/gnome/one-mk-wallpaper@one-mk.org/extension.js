// SPDX-License-Identifier: MIT OR Apache-2.0
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';
import * as LoginManager from 'resource:///org/gnome/shell/misc/loginManager.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const NAME = 'org.onemk.WallpaperBridge1';
const PATH = '/org/onemk/WallpaperBridge1';
// Renderer states: 0 starting, 1 playing, 2 paused, 3 failed, 4 holding (a
// still or a long frame is shown: no damage is expected, so no watchdog runs).
const PLAYING = 1, HOLDING = 4;
// A stopped renderer gets SIGTERM to clean up its decoder; SIGKILL after this.
const TERMINATE_GRACE_MS = 2000;
let bridgeInterface;

function showing(state) {
    return state === PLAYING || state === HOLDING;
}

/** Ends a renderer: SIGTERM, then SIGKILL if it is still there after the
 * grace period. Lives in the recovery object, so it survives disable. */
function terminate(recovery, process) {
    if (!process || recovery.escalations.has(process))
        return;
    process.send_signal(15);
    recovery.escalations.set(process, GLib.timeout_add(GLib.PRIORITY_DEFAULT, TERMINATE_GRACE_MS, () => {
        recovery.escalations.delete(process);
        process.force_exit();
        return GLib.SOURCE_REMOVE;
    }));
}

function validLayout(monitors) {
    if (monitors.length > 16)
        return false;
    if (!monitors.length)
        return true;
    let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity, scale = 0.5;
    for (const [x, y, width, height, factor] of monitors) {
        if (![x, y, width, height, x + width, y + height].every(value =>
            Number.isInteger(value) && value >= -2147483648 && value <= 2147483647) ||
            width <= 0 || height <= 0 || !Number.isFinite(factor) || factor < 0.5 || factor > 4)
            return false;
        left = Math.min(left, x); top = Math.min(top, y);
        right = Math.max(right, x + width); bottom = Math.max(bottom, y + height);
        scale = Math.max(scale, factor);
    }
    const width = Math.ceil((right - left) * scale), height = Math.ceil((bottom - top) * scale);
    return width <= 16384 && height <= 16384 && width * height <= 64000000;
}

class Bridge {
    constructor(extension, recovery) {
        this._extension = extension;
        this._recovery = recovery;
        this._authorized = false;
        this._destroyed = false;
        this._ownsName = false;
        this._ready = false;
        this._sender = null;
        this._pid = null;
        this._client = null;
        this._renderer = null;
        this._signals = [];
        this._sources = new Map();
        this._clones = [];
        this._retired = new Set();
        this._serial = 1;
        this._state = 0;
        this._attempt = null;
        this._reasons = 2;
        this._credentialPending = false;
        this._helloTimes = [];
        this._stateTimes = [];
        this._replacementTimes = [];
        this._cancellable = new Gio.Cancellable();
        this.reason = 'owner-authenticated daemon presentation plan unavailable; static wallpaper';
        this._layer = new Clutter.Actor({reactive: false, visible: false});
        Main.layoutManager._backgroundGroup.add_child(this._layer);
        this._monitors = this._layout();
        const xml = new TextDecoder().decode(extension.dir.get_child('bridge.xml').load_contents(null)[1]);
        bridgeInterface ??= Gio.DBusInterfaceInfo.new_for_xml(xml);
        this._interface = bridgeInterface;
        this._object = new Gio.DBusExportedObject({g_interface_info: this._interface});
        this._object.connect('handle-method-call', (_object, method, invocation, parameters) => {
            this._method(method, invocation, parameters).catch(() => {
                invocation.return_dbus_error(`${NAME}.Failed`, 'bridge request failed');
            });
        });
        this._object.export(Gio.DBus.session, PATH);
        this._exported = true;
        this._name = Gio.bus_own_name_on_connection(Gio.DBus.session, NAME,
            Gio.BusNameOwnerFlags.DO_NOT_QUEUE, () => {
                if (this._destroyed)
                    return;
                this._ownsName = true;
                this._verifyExport();
            }, () => {
                if (this._destroyed)
                    return;
                this._ownsName = false;
                this._ready = false;
                this.reason = 'bridge name unavailable; static wallpaper';
                this._extension.report(this.reason);
                this._stop(false, 'bridge name lost');
                this._object.unexport();
                this._exported = false;
            });
        this._ownerWatch = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus',
            'org.freedesktop.DBus', 'NameOwnerChanged', '/org/freedesktop/DBus', null,
            Gio.DBusSignalFlags.NONE, (_connection, _sender, _path, _interface, _signal, value) => {
                const [name, , owner] = value.deepUnpack();
                if (name === this._sender && !owner)
                    this._stop(false, 'renderer left the bus');
            });
        this._connect(Main.layoutManager, 'monitors-changed', () => {
            this._layer.hide();
            this._later('layout', 100, () => {
                const monitors = this._layout();
                if (!validLayout(monitors) || this._serial === 0xffffffff) {
                    this.reason = 'layout out of bounds or serial exhausted; re-enable bridge';
                    this._authorized = false;
                    this._stop(true, 'layout out of bounds');
                    return;
                }
                if (JSON.stringify(monitors) === JSON.stringify(this._monitors)) {
                    // Same logical geometry (for example a mode or output
                    // change Mutter reports without moving anything): the
                    // current window and its slices stay valid.
                    this._coverage();
                    return;
                }
                this._monitors = monitors;
                this._serial++;
                if (this._renderer) {
                    // Sliced for the old layout: never revealed again; only a
                    // window adopted under the new serial can be.
                    this._renderer.fresh = false;
                    this._later('replacement', 2000, () => this._stop(false, 'no window for the new layout in 2 s'));
                }
                this._emit('Layout', new GLib.Variant('(ua(iiiid))', [this._serial, monitors]));
                this._rebuild();
                this._coverage();
            });
        });
        this._connect(global.display, 'window-created', (_display, window) => this._adopt(window));
        for (const signal of ['restacked', 'in-fullscreen-changed'])
            this._connect(global.display, signal, () => this._queueCoverage());
        this._connect(global.workspace_manager, 'active-workspace-changed', () => this._queueCoverage());
        for (const signal of ['showing', 'hidden'])
            this._connect(Main.overview, signal, () => this._coverage());
        this._connect(Main.sessionMode, 'updated', () => this._coverage());
        this._connect(Main.screenShield, 'locked-changed', () => this._coverage());
        // Suspend: logind's PrepareForSleep. Displays are off while asleep, so
        // the presentation stops before sleeping (reason 8) and a fresh
        // renderer starts after resume: no frame from before sleep is reused.
        this._login = LoginManager.getLoginManager();
        this._connect(this._login, 'prepare-for-sleep', () => this._coverage());
    }

    _connect(object, signal, callback) {
        this._signals.push([object, object.connect(signal, callback)]);
    }

    async _verifyExport() {
        try {
            const reply = await Gio.DBus.session.call(Gio.DBus.session.get_unique_name(), PATH,
                'org.freedesktop.DBus.Introspectable', 'Introspect', null,
                new GLib.VariantType('(s)'), Gio.DBusCallFlags.NONE, 500, this._cancellable);
            if (this._destroyed || !this._ownsName)
                return;
            const [xml] = reply.deepUnpack();
            if (!xml.includes('name="Hello"') || !xml.includes('name="SetState"'))
                throw new Error('exported protocol is incomplete');
            this._ready = true;
            this._coverage();
        } catch (error) {
            if (!this._destroyed) {
                this.reason = `bridge export unavailable: ${error.message}`;
                this._extension.report(this.reason);
                this._stop(false, 'bridge export unavailable');
            }
        }
    }

    _layout() {
        return Main.layoutManager.monitors.map(monitor => [monitor.x, monitor.y,
            monitor.width, monitor.height, global.display.get_monitor_scale(monitor.index)]);
    }

    _cancel(name) {
        const source = this._sources.get(name);
        if (source)
            GLib.source_remove(source);
        this._sources.delete(name);
    }

    _later(name, milliseconds, callback) {
        if (this._destroyed || this._sources.has(name))
            return;
        this._sources.set(name, GLib.timeout_add(GLib.PRIORITY_DEFAULT, milliseconds, () => {
            this._sources.delete(name);
            if (!this._destroyed)
                callback();
            return GLib.SOURCE_REMOVE;
        }));
    }

    _admit(times, limit) {
        const now = GLib.get_monotonic_time();
        while (times.length && now - times[0] >= GLib.USEC_PER_SEC)
            times.shift();
        if (times.length >= limit)
            return false;
        times.push(now);
        return true;
    }

    async _credential(method, sender) {
        const reply = await Gio.DBus.session.call('org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', method, new GLib.Variant('(s)', [sender]),
            new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 500, this._cancellable);
        return reply.deepUnpack()[0];
    }

    async _method(method, invocation, parameters) {
        const fail = (name, reason) => invocation.return_dbus_error(
            name === 'AccessDenied' ? 'org.freedesktop.DBus.Error.AccessDenied' : `${NAME}.${name}`, reason);
        const sender = invocation.get_sender();
        if (!this._ownsName || this._destroyed || !this._pid || !this._client)
            return fail('AccessDenied', 'not the spawned renderer');
        if (this._sender !== sender) {
            if (method !== 'Hello' || this._sender || this._credentialPending || !this._admit(this._helloTimes, 4))
                return fail('AccessDenied', 'unbound renderer');
            this._credentialPending = true;
            const client = this._client, pid = this._pid;
            try {
                const callerPid = await this._credential('GetConnectionUnixProcessID', sender);
                const uid = await this._credential('GetConnectionUnixUser', sender);
                if (uid !== this._uid() || callerPid !== pid || this._client !== client ||
                    this._destroyed || !this._ownsName || this._pid !== pid)
                    return fail('AccessDenied', 'renderer credentials changed');
                this._sender = sender;
            } finally {
                this._credentialPending = false;
            }
        }
        if (parameters.get_size() > 4096 || parameters.get_type_string() !== '(u)')
            return fail('InvalidArgs', 'expected one bounded uint32');
        const [value] = parameters.deepUnpack();
        if (method === 'Hello') {
            if (value !== 1)
                return fail('NotSupported', 'protocol must be 1');
            if (!validLayout(this._monitors))
                return fail('InvalidArgs', 'layout exceeds physical canvas budget');
            this._cancel('hello');
            invocation.return_value(new GLib.Variant('(uua(iiiid))', [1, this._serial, this._monitors]));
            this._emit('PauseHint', new GLib.Variant('(u)', [this._reasons]));
        } else if (method === 'SetState') {
            if (value > HOLDING || !this._admit(this._stateTimes, 10))
                return fail('InvalidArgs', 'invalid state or state rate exceeded');
            if (this._state !== value) {
                this._state = value;
                if (value === PLAYING && this._renderer) {
                    // Progress is expected again: watch for it from now on.
                    this._renderer.lastDamage = GLib.get_monotonic_time();
                    this._watchdog();
                } else {
                    this._cancel('watchdog');
                }
                // A paused renderer has no surface by design (for example a
                // layout outside the committed canvas): the layer stays
                // hidden, so no replacement window is awaited.
                if (value === 2)
                    this._cancel('replacement');
                // The clones do not change with the state (Playing and
                // Holding alternate on every long frame): only visibility.
                if (!showing(value))
                    this._layer.hide();
                this._queueCoverage();
            }
            invocation.return_value(null);
            if (value === 3)
                this._stop(false, 'renderer reported failure');
        } else {
            fail('InvalidArgs', 'unknown method');
        }
    }

    _uid() {
        return new Gio.Credentials().get_unix_user();
    }

    _emit(signal, value) {
        if (this._ownsName && this._sender && value.get_size() <= 4096)
            Gio.DBus.session.emit_signal(this._sender, PATH, NAME, signal, value);
    }

    _restart() {
        if (!this._authorized || !this._ownsName || !this._ready || this._destroyed || this._reasons ||
            this._client || this._recovery.children.size || this._recovery.exhausted)
            return;
        const now = GLib.get_monotonic_time();
        const recent = time => now - time < 60 * GLib.USEC_PER_SEC;
        // `attempts` holds starts that may still count as failures; a run the
        // Shell itself ended (pause, lock, sleep, disable) is credited back
        // in `_stop`, so ordinary lock/unlock or cover/uncover never exhausts
        // the cap. `starts` holds every start and only lengthens the delay.
        this._recovery.attempts = this._recovery.attempts.filter(recent);
        this._recovery.starts = this._recovery.starts.filter(recent);
        if (this._recovery.attempts.length >= 3) {
            this._recovery.exhausted = true;
            this.reason = 'restart limit reached; disable then explicitly retry in a new session';
            this._extension.report(this.reason);
            return;
        }
        const backoff = Math.min(Math.max(this._recovery.attempts.length, this._recovery.starts.length - 2, 0), 2);
        const delay = this._recovery.unlock ? 1 : [1000, 2000, 4000][backoff];
        this._later('restart', delay, () => {
            if (!this._authorized || this._reasons || !this._ownsName || this._recovery.children.size)
                return;
            this._recovery.unlock = false;
            this._attempt = GLib.get_monotonic_time();
            this._recovery.attempts.push(this._attempt);
            this._recovery.starts.push(this._attempt);
            const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.NONE});
            try {
                this._client = Meta.WaylandClient.new_subprocess(global.context, launcher,
                    [GLib.build_filenamev([this._extension.path, 'libexec', 'one-mk-wall']), '--present=gnome']);
                const process = this._client.get_subprocess();
                this._pid = Number(process.get_identifier());
                this._state = 0;
                this._sender = null;
                this._stateTimes = [];
                this._replacementTimes = [];
                const recovery = this._recovery;
                recovery.children.add(process);
                process.wait_async(null, (source, result) => {
                    source.wait_finish(result);
                    recovery.children.delete(process);
                    if (this._client?.get_subprocess() === process)
                        this._stop(false, 'renderer exited');
                    // After _stop: an exited process needs no escalation (and
                    // Gio no longer signals a reaped child).
                    const escalation = recovery.escalations.get(process);
                    if (escalation) {
                        GLib.source_remove(escalation);
                        recovery.escalations.delete(process);
                    }
                    recovery.wake?.();
                });
                this._later('hello', 2000, () => this._stop(false, 'no Hello in 2 s'));
                this._later('first-frame', 5000, () => {
                    // A renderer paused by its plan shows nothing on purpose.
                    if (this._state !== 2 && (!this._renderer?.fresh || !showing(this._state)))
                        this._stop(false, 'no first frame in 5 s');
                });
            } catch (error) {
                this.reason = `renderer unavailable: ${error.message}`;
                this._stop(false, 'renderer could not start');
            } finally {
                launcher.close();
            }
        });
    }

    /** Hides the presentation and ends the renderer. `planned` marks a stop
     * the Shell chose (pause reason, lock, sleep, disable), not a failure. */
    _stop(planned = false, cause = 'stopped') {
        if (this._client)
            this.lastStop = {cause, planned, pid: this._pid};
        this._layer.hide();
        for (const name of ['restart', 'hello', 'first-frame', 'watchdog', 'replacement', 'geometry'])
            this._cancel(name);
        const process = this._client?.get_subprocess();
        if (planned && process && this._attempt !== null) {
            const index = this._recovery.attempts.indexOf(this._attempt);
            if (index >= 0)
                this._recovery.attempts.splice(index, 1);
        }
        this._attempt = null;
        this._client = null;
        this._pid = null;
        this._sender = null;
        this._state = 0;
        this._drop(true);
        terminate(this._recovery, process);
        this._restart();
    }

    /** Forgets the current window. `ending` keeps it hidden until the
     * compositor destroys it, so a renderer that is still exiting can never
     * show a stale frame on its own. */
    _drop(ending = false) {
        const renderer = this._renderer;
        this._renderer = null;
        if (renderer) {
            renderer.actor.disconnectObject(renderer);
            renderer.window.disconnectObject(renderer);
            if (ending) {
                this._hide(renderer);
                renderer.actor.connectObject('notify::visible', () => this._hide(renderer), renderer);
            }
        }
        this._rebuild();
    }

    _hide(renderer) {
        if (!renderer.hiding) {
            renderer.hiding = true;
            renderer.actor.hide();
            renderer.hiding = false;
        }
    }

    _adopt(window) {
        if (!this._client?.owns_window(window))
            return;
        window.hide_from_window_list();
        window.set_type(Meta.WindowType.DESKTOP);
        const actor = window.get_compositor_private();
        if (!actor || this._retired.size || !this._admit(this._replacementTimes, 2)) {
            this._stop(false, `window refused (actor ${Boolean(actor)}, retired ${this._retired.size})`);
            return;
        }
        if (this._renderer) {
            const previous = this._renderer;
            this._drop();
            this._retired.add(previous);
            const cleanup = () => {
                this._retired.delete(previous);
                previous.actor.disconnectObject(previous);
                previous.window.disconnectObject(previous);
                this._cancel('retired');
            };
            previous.actor.connectObject('notify::visible', () => this._hide(previous), 'destroy', cleanup, previous);
            previous.window.connectObject('unmanaged', cleanup, previous);
            this._hide(previous);
            this._later('retired', 2000, () => this._stop(false, 'retired window outlived 2 s'));
        }
        const renderer = {window, actor, fresh: false, lastDamage: GLib.get_monotonic_time(), serial: this._serial};
        this._renderer = renderer;
        const left = Math.min(...this._monitors.map(monitor => monitor[0]));
        const top = Math.min(...this._monitors.map(monitor => monitor[1]));
        const right = Math.max(...this._monitors.map(monitor => monitor[0] + monitor[2]));
        const bottom = Math.max(...this._monitors.map(monitor => monitor[1] + monitor[3]));
        window.move_resize_frame(false, left, top, right - left, bottom - top);
        renderer.width = right - left;
        renderer.height = bottom - top;
        renderer.geometryChecks = 0;
        actor.connectObject('notify::visible', () => this._hide(renderer),
            'destroy', () => this._drop(), 'damaged', () => this._damaged(renderer), renderer);
        window.connectObject('unmanaged', () => this._drop(), renderer);
        this._hide(renderer);
        this._rebuild();
    }

    _damaged(renderer) {
        if (renderer !== this._renderer)
            return;
        const rect = renderer.window.get_buffer_rect();
        // Mutter 50 reports the first damage before the window has buffer
        // geometry. An empty rectangle is not content yet, but a still sends
        // nothing more, so look again shortly (bounded: 40 x 25 ms).
        if (!rect.width || !rect.height) {
            if (renderer.geometryChecks++ < 40)
                this._later('geometry', 25, () => this._damaged(renderer));
            return;
        }
        // A window adopted under an older layout is sliced for it: its frames
        // are never content again, whatever their size (the replacement
        // deadline still applies).
        if (renderer.serial !== this._serial)
            return;
        if (rect.width !== renderer.width || rect.height !== renderer.height) {
            this._stop(false, 'window of the wrong size');
            return;
        }
        renderer.lastDamage = GLib.get_monotonic_time();
        if (!renderer.fresh) {
            // The first healthy frame satisfies the start deadline for good;
            // later replacements have their own deadline.
            this._cancel('first-frame');
            this._cancel('replacement');
            renderer.fresh = true;
            this._rebuild();
        }
        this._watchdog();
    }

    _watchdog() {
        if (!this._renderer || this._reasons || this._state !== PLAYING)
            return;
        this._later('watchdog', 1000, () => {
            if (this._state !== PLAYING || this._reasons)
                return;
            if (this._renderer && GLib.get_monotonic_time() - this._renderer.lastDamage > 3 * GLib.USEC_PER_SEC) {
                this._renderer.window.check_alive(global.get_current_time());
                this._stop(false, 'no damage for 3 s');
            } else
                this._watchdog();
        });
    }

    _rebuild() {
        // The layer is never visible without a fresh window behind it; it is
        // revealed again only by the coverage check that follows.
        if (!this._renderer?.fresh || !showing(this._state) || this._sources.has('layout'))
            this._layer.hide();
        this._layer.destroy_all_children();
        this._clones = [];
        if (!this._renderer)
            return;
        const rect = this._renderer.window.get_buffer_rect();
        for (const [x, y, width, height] of this._monitors) {
            const box = new Clutter.Actor({x, y, width, height, clip_to_allocation: true, reactive: false});
            const clone = new Clutter.Clone({source: this._renderer.actor, x: rect.x - x, y: rect.y - y});
            box.add_child(clone);
            this._layer.add_child(box);
            this._clones.push(clone);
        }
        this._queueCoverage();
    }

    _queueCoverage() {
        this._later('coverage', 100, () => this._coverage());
    }

    _coverage() {
        const workspace = global.workspace_manager.get_active_workspace();
        const covered = Main.layoutManager.monitors.map(monitor => global.get_window_actors().some(actor => {
            const window = actor.get_meta_window();
            if (!window || window === this._renderer?.window || window.minimized ||
                !window.showing_on_its_workspace() || !window.located_on_workspace(workspace) ||
                window.get_window_type() !== Meta.WindowType.NORMAL)
                return false;
            const rect = window.get_frame_rect();
            return (window.is_fullscreen() && window.get_monitor() === monitor.index) ||
                (rect.x <= monitor.x && rect.y <= monitor.y + 64 &&
                 rect.x + rect.width >= monitor.x + monitor.width && rect.y + rect.height >= monitor.y + monitor.height);
        }));
        const reasons = (covered.length && covered.every(Boolean) ? 1 : 0) |
            (Main.sessionMode.isLocked || Main.screenShield.locked ? 2 : 0) |
            (Main.overview.visible ? 4 : 0) |
            (!this._monitors.length || this._login.preparingForSleep ? 8 : 0);
        const previous = this._reasons;
        this._reasons = reasons;
        this._clones.forEach((clone, index) => { clone.visible = !covered[index]; });
        this._layer.visible = this._ownsName && reasons === 0 && !this._sources.has('layout') &&
            showing(this._state) && this._renderer?.fresh === true;
        if (reasons !== previous)
            this._emit('PauseHint', new GLib.Variant('(u)', [reasons]));
        if (reasons) {
            this._cancel('restart');
            this._cancel('watchdog');
            if (this._client)
                this._stop(true, `pause reasons ${reasons}`);
        } else {
            // Unlock and wake from sleep start a fresh renderer at once.
            if (previous & 10)
                this._recovery.unlock = true;
            this._restart();
            this._watchdog();
        }
    }

    destroy() {
        this._destroyed = true;
        this._cancellable.cancel();
        this._stop(true, 'bridge disabled');
        for (const name of [...this._sources.keys()])
            this._cancel(name);
        for (const [object, signal] of this._signals)
            object.disconnect(signal);
        for (const renderer of this._retired) {
            renderer.actor.disconnectObject(renderer);
            renderer.window.disconnectObject(renderer);
        }
        Gio.DBus.session.signal_unsubscribe(this._ownerWatch);
        if (this._exported) {
            this._object.unexport();
            this._exported = false;
        }
        Gio.bus_unown_name(this._name);
        this._layer.destroy();
    }
}

export default class WallpaperExtension extends Extension {
    report(reason) {
        this.reason = reason;
        console.log(`one-mk wallpaper: ${reason}`);
        const directory = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'one-mk-wallpaper']);
        GLib.mkdir_with_parents(directory, 0o700);
        Gio.File.new_for_path(GLib.build_filenamev([directory, 'status.json'])).replace_contents(
            JSON.stringify({shell_owner: Gio.DBus.session.get_unique_name(), shell_version: Config.PACKAGE_VERSION,
                reason, restart_limit_reached: this._recovery?.exhausted === true}), null, false,
            Gio.FileCreateFlags.PRIVATE | Gio.FileCreateFlags.REPLACE_DESTINATION, null);
    }

    enable() {
        if (!this.metadata['one-mk-qualified-shell-versions'].includes(Config.PACKAGE_VERSION)) {
            this.report(`unqualified GNOME Shell ${Config.PACKAGE_VERSION}; static wallpaper`);
            throw new Error(this.reason);
        }
        this._recovery ??= {attempts: [], starts: [], children: new Set(), escalations: new Map(),
            exhausted: false, unlock: false};
        this.bridge = new Bridge(this, this._recovery);
        this._recovery.wake = () => this.bridge?._restart();
        this.report(this.bridge.reason);
    }

    disable() {
        this._recovery && (this._recovery.wake = null);
        this.bridge?.destroy();
        this.bridge = null;
        this.report(this._recovery?.exhausted ? 'restart limit reached; static wallpaper' : 'extension disabled; static wallpaper');
    }
}
