import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const NAME = 'org.onemk.WallpaperVideo1';
const PATH = '/org/onemk/WallpaperVideo1';

export default class WallpaperVideo extends Extension {
    enable() {
        if (Config.PACKAGE_VERSION !== '50.1')
            throw new Error('oneMK video is qualified only on GNOME Shell 50.1');
        this._generation = 0;
        this._client = null;
        this._windows = new Map();
        this._candidates = new Map();
        this._layer = new Clutter.Actor({reactive: false});
        Main.layoutManager._backgroundGroup.add_child(this._layer);
        const xml = new TextDecoder().decode(this.dir.get_child('bridge.xml').load_contents(null)[1]);
        this._object = new Gio.DBusExportedObject({g_interface_info: Gio.DBusInterfaceInfo.new_for_xml(xml)});
        this._object.connect('handle-method-call', (_object, method, invocation, parameters) => {
            this._method(method, invocation, parameters).catch(error => {
                invocation.return_dbus_error(`${NAME}.Refused`, error.message);
            });
        });
        this._object.export(Gio.DBus.session, PATH);
        this._owner = Gio.bus_own_name_on_connection(Gio.DBus.session, NAME, Gio.BusNameOwnerFlags.NONE, null, () => this._stop());
        this._created = global.display.connect('window-created', (_display, window) => this._candidate(window));
        this._layout = Main.layoutManager.connect('monitors-changed', () => this._rebuild());
        this._watch = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus', 'org.freedesktop.DBus', 'NameOwnerChanged',
            '/org/freedesktop/DBus', null, Gio.DBusSignalFlags.NONE, (_bus, _sender, _path, _interface, _signal, parameters) => {
                const [name, , owner] = parameters.deep_unpack();
                if (name === this._client?.sender && !owner)
                    this._stop();
            });
    }

    _credential(sender, method) {
        return new Promise((resolve, reject) => {
            Gio.DBus.session.call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', method,
                new GLib.Variant('(s)', [sender]), new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 1000, null,
                (connection, result) => {
                    try { resolve(connection.call_finish(result).deep_unpack()[0]); }
                    catch (error) { reject(error); }
                });
        });
    }

    async _method(method, invocation, parameters) {
        if (method === 'Inspect') {
            invocation.return_value(new GLib.Variant('(s)', [Config.PACKAGE_VERSION]));
            return;
        }
        const sender = invocation.get_sender();
        if (method === 'Register') {
            const [secret] = parameters.deep_unpack();
            if (!/^[a-f0-9]{32}$/.test(secret) || this._client)
                throw new Error('oneMK wallpaper is already running or the launch token is invalid');
            const generation = this._generation;
            const [uid, pid] = await Promise.all([
                this._credential(sender, 'GetConnectionUnixUser'), this._credential(sender, 'GetConnectionUnixProcessID'),
            ]);
            if (uid !== new Gio.Credentials().get_unix_user() || pid <= 1 || !this._object ||
                generation !== this._generation || this._client)
                throw new Error('The renderer must belong to this desktop user');
            this._client = {sender, pid, secret};
        } else {
            if (sender !== this._client?.sender)
                throw new Error('Only the registered renderer can control this wallpaper');
            if (method === 'Presented') {
                const titles = [...this._windows.values()].filter(entry => entry.fresh && entry.box?.visible).map(entry => entry.window.get_title());
                invocation.return_value(new GLib.Variant('(as)', [titles]));
                return;
            }
            if (method === 'Stop')
                this._stop();
            else
                throw new Error('Unknown wallpaper method');
        }
        invocation.return_value(null);
    }

    _candidate(window) {
        const tryAdopt = () => this._adopt(window);
        window.connectObject('notify::title', tryAdopt, 'unmanaged', () => {
            this._candidates.delete(window);
            window.disconnectObject(this);
            this._drop(window);
        }, this);
        this._candidates.set(window, true);
        tryAdopt();
    }

    _adopt(window) {
        const client = this._client;
        const title = window.get_title() ?? '';
        if (!client || window.get_pid() !== client.pid || !title.startsWith(`oneMK:${client.secret}:`) || this._windows.has(window))
            return;
        const geometry = title.slice(39).split(':').map(Number);
        if (geometry.length !== 5 || geometry.some(value => !Number.isSafeInteger(value)) || this._windows.size >= 16)
            return;
        const [, x, y, width, height] = geometry;
        if (!Main.layoutManager.monitors.some(monitor => monitor.x === x && monitor.y === y && monitor.width === width && monitor.height === height))
            return;
        const actor = window.get_compositor_private();
        if (!actor)
            return;
        const entry = {window, actor, x, y, width, height, fresh: false, box: null};
        this._windows.set(window, entry);
        window.hide_from_window_list();
        window.set_type(Meta.WindowType.DESKTOP);
        window.stick();
        window.lower();
        window.move_resize_frame(false, x, y, width, height);
        const hide = () => {
            if (actor.visible)
                actor.hide();
        };
        actor.connectObject('notify::visible', hide, 'damaged', () => {
            const rect = window.get_buffer_rect();
            entry.fresh = rect.width === width && rect.height === height;
            if (entry.box)
                entry.box.visible = entry.fresh;
        }, 'destroy', () => actor.disconnectObject(entry), entry);
        hide();
        this._rebuild();
    }

    _rebuild() {
        this._layer.destroy_all_children();
        for (const entry of this._windows.values()) {
            entry.box = null;
            if (!Main.layoutManager.monitors.some(monitor => monitor.x === entry.x && monitor.y === entry.y &&
                monitor.width === entry.width && monitor.height === entry.height))
                continue;
            const box = new Clutter.Actor({x: entry.x, y: entry.y, width: entry.width, height: entry.height,
                clip_to_allocation: true, reactive: false, visible: entry.fresh});
            box.add_child(new Clutter.Clone({source: entry.actor, reactive: false}));
            this._layer.add_child(box);
            entry.box = box;
        }
    }

    _drop(window) {
        const entry = this._windows.get(window);
        if (!entry)
            return;
        this._windows.delete(window);
        entry.actor.disconnectObject(entry);
        entry.box?.destroy();
    }

    _stop() {
        this._generation++;
        this._client = null;
        for (const entry of this._windows.values()) {
            entry.box?.destroy();
            entry.box = null;
        }
        this._windows.clear();
    }

    disable() {
        this._stop();
        for (const window of this._candidates.keys())
            window.disconnectObject(this);
        this._candidates.clear();
        global.display.disconnect(this._created);
        Main.layoutManager.disconnect(this._layout);
        Gio.DBus.session.signal_unsubscribe(this._watch);
        Gio.bus_unown_name(this._owner);
        this._object.unexport();
        this._object = null;
        this._layer.destroy();
    }
}
