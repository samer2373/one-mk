// SPDX-License-Identifier: MIT OR Apache-2.0
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const NAME = 'org.onemk.WallpaperBridge1';
const PATH = '/org/onemk/WallpaperBridge1';
let bridgeInterface;

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
                this._stop();
                this._object.unexport();
                this._exported = false;
            });
        this._ownerWatch = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus',
            'org.freedesktop.DBus', 'NameOwnerChanged', '/org/freedesktop/DBus', null,
            Gio.DBusSignalFlags.NONE, (_connection, _sender, _path, _interface, _signal, value) => {
                const [name, , owner] = value.deepUnpack();
                if (name === this._sender && !owner)
                    this._stop();
            });
        this._connect(Main.layoutManager, 'monitors-changed', () => {
            this._layer.hide();
            this._later('layout', 100, () => {
                const monitors = this._layout();
                if (!validLayout(monitors) || this._serial === 0xffffffff) {
                    this.reason = 'layout out of bounds or serial exhausted; re-enable bridge';
                    this._authorized = false;
                    this._stop();
                    return;
                }
                this._monitors = monitors;
                this._serial++;
                if (this._renderer) {
                    this._renderer.fresh = false;
                    this._later('replacement', 2000, () => this._stop());
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
                this._stop();
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
            if (value > 3 || !this._admit(this._stateTimes, 10))
                return fail('InvalidArgs', 'invalid state or state rate exceeded');
            if (this._state !== value) {
                this._state = value;
                this._rebuild();
            }
            invocation.return_value(null);
            if (value === 3)
                this._stop();
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
        this._recovery.attempts = this._recovery.attempts.filter(time => now - time < 60 * GLib.USEC_PER_SEC);
        if (this._recovery.attempts.length >= 3) {
            this._recovery.exhausted = true;
            this.reason = 'restart limit reached; disable then explicitly retry in a new session';
            this._extension.report(this.reason);
            return;
        }
        const delay = this._recovery.unlock ? 1 : [1000, 2000, 4000][this._recovery.attempts.length];
        this._later('restart', delay, () => {
            if (!this._authorized || this._reasons || !this._ownsName || this._recovery.children.size)
                return;
            this._recovery.unlock = false;
            this._recovery.attempts.push(GLib.get_monotonic_time());
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
                this._recovery.children.add(process);
                process.wait_async(null, (source, result) => {
                    source.wait_finish(result);
                    this._recovery.children.delete(process);
                    if (this._client?.get_subprocess() === process)
                        this._stop();
                    this._recovery.wake?.();
                });
                this._later('hello', 2000, () => this._stop());
                this._later('first-frame', 5000, () => {
                    if (!this._renderer?.fresh || this._state !== 1)
                        this._stop();
                });
            } catch (error) {
                this.reason = `renderer unavailable: ${error.message}`;
                this._stop();
            } finally {
                launcher.close();
            }
        });
    }

    _stop() {
        this._layer.hide();
        for (const name of ['restart', 'hello', 'first-frame', 'watchdog', 'replacement'])
            this._cancel(name);
        const process = this._client?.get_subprocess();
        this._client = null;
        this._pid = null;
        this._sender = null;
        this._state = 0;
        this._drop();
        process?.force_exit();
        this._restart();
    }

    _drop() {
        const renderer = this._renderer;
        this._renderer = null;
        if (renderer) {
            renderer.actor.disconnectObject(renderer);
            renderer.window.disconnectObject(renderer);
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
            this._stop();
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
            this._later('retired', 2000, () => this._stop());
        }
        const renderer = {window, actor, fresh: false, lastDamage: GLib.get_monotonic_time()};
        this._renderer = renderer;
        const left = Math.min(...this._monitors.map(monitor => monitor[0]));
        const top = Math.min(...this._monitors.map(monitor => monitor[1]));
        const right = Math.max(...this._monitors.map(monitor => monitor[0] + monitor[2]));
        const bottom = Math.max(...this._monitors.map(monitor => monitor[1] + monitor[3]));
        window.move_resize_frame(false, left, top, right - left, bottom - top);
        actor.connectObject('notify::visible', () => this._hide(renderer),
            'destroy', () => this._drop(), 'damaged', () => {
                const rect = window.get_buffer_rect();
                if (rect.width !== right - left || rect.height !== bottom - top) {
                    this._stop();
                    return;
                }
                renderer.lastDamage = GLib.get_monotonic_time();
                if (!renderer.fresh) {
                    this._cancel('replacement');
                    renderer.fresh = true;
                    this._rebuild();
                }
                this._watchdog();
            }, renderer);
        window.connectObject('unmanaged', () => this._drop(), renderer);
        this._hide(renderer);
        this._rebuild();
    }

    _watchdog() {
        if (!this._renderer || this._reasons || this._state !== 1)
            return;
        this._later('watchdog', 1000, () => {
            if (this._renderer && GLib.get_monotonic_time() - this._renderer.lastDamage > 3 * GLib.USEC_PER_SEC) {
                this._renderer.window.check_alive(global.get_current_time());
                this._stop();
            } else
                this._watchdog();
        });
    }

    _rebuild() {
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
            (Main.overview.visible ? 4 : 0) | (!this._monitors.length ? 8 : 0);
        const previous = this._reasons;
        this._reasons = reasons;
        this._clones.forEach((clone, index) => { clone.visible = !covered[index]; });
        this._layer.visible = this._ownsName && reasons === 0 && this._state === 1 && this._renderer?.fresh === true;
        if (reasons !== previous)
            this._emit('PauseHint', new GLib.Variant('(u)', [reasons]));
        if (reasons) {
            this._cancel('restart');
            this._cancel('watchdog');
            if (this._client)
                this._stop();
        } else {
            if (previous & 2)
                this._recovery.unlock = true;
            this._restart();
            this._watchdog();
        }
    }

    destroy() {
        this._destroyed = true;
        this._cancellable.cancel();
        this._stop();
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
        this._recovery ??= {attempts: [], children: new Set(), exhausted: false, unlock: false};
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
