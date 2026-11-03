// SPDX-License-Identifier: MIT OR Apache-2.0
//
// While one-mk has this computer's mouse on another computer, the desktop
// (GNOME Wayland) keeps drawing the pointer where it left. This hides it
// then. one-mk asks over D-Bus: Hide while the mouse is away, asked again
// every few seconds, and Show when it is back. After another computer has
// moved this one's pointer and let go, HideUntilMoved hides it where it
// stands until this computer's own mouse or touchpad is used. The pointer
// also comes back if one-mk goes away, if asking stops for HOLD_SECONDS
// (Hide only), and when this extension is disabled, so it can never stay
// hidden.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';

const NAME = 'org.onemk.Pointer1';
const PATH = '/org/onemk/Pointer1';
const HOLD_SECONDS = 15;
// Motion this soon after HideUntilMoved is the last of what moved it there.
const GRACE_MS = 400;
// The pointer: shown, hidden while one-mk holds the mouse away (Hide), or
// hidden until the user moves it (HideUntilMoved).
const SHOWN = 0, HELD = 1, UNTIL_MOVED = 2;

export default class OneMkPointer extends Extension {
    enable() {
        this._tracker = global.backend.get_cursor_tracker();
        this._mode = SHOWN;
        this._timer = 0;
        this._holder = null;
        this._motion = 0;
        const xml = new TextDecoder().decode(this.dir.get_child('bridge.xml').load_contents(null)[1]);
        this._object = new Gio.DBusExportedObject({g_interface_info: Gio.DBusInterfaceInfo.new_for_xml(xml)});
        this._object.connect('handle-method-call', (_object, method, invocation) => {
            try {
                this._method(method, invocation);
            } catch (error) {
                invocation.return_dbus_error(`${NAME}.Refused`, String(error.message ?? error));
            }
        });
        this._object.export(Gio.DBus.session, PATH);
        this._owner = Gio.bus_own_name_on_connection(Gio.DBus.session, NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        // The one that asked to hide it went away: show it again.
        this._watch = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus', 'org.freedesktop.DBus', 'NameOwnerChanged',
            '/org/freedesktop/DBus', null, Gio.DBusSignalFlags.NONE, (_bus, _sender, _path, _interface, _signal, parameters) => {
                const [name, , owner] = parameters.deep_unpack();
                if (this._mode !== SHOWN && name === this._holder && !owner)
                    this._show();
            });
    }

    disable() {
        this._show();
        if (this._watch) {
            Gio.DBus.session.signal_unsubscribe(this._watch);
            this._watch = 0;
        }
        if (this._owner) {
            Gio.bus_unown_name(this._owner);
            this._owner = 0;
        }
        if (this._object) {
            this._object.unexport();
            this._object = null;
        }
        this._tracker = null;
    }

    _method(method, invocation) {
        switch (method) {
        case 'Inspect':
            invocation.return_value(new GLib.Variant('(s)', [Config.PACKAGE_VERSION]));
            break;
        case 'Hide':
            this._hide(invocation.get_sender());
            invocation.return_value(null);
            break;
        case 'HideUntilMoved':
            this._hideUntilMoved(invocation.get_sender());
            invocation.return_value(null);
            break;
        case 'Show':
            this._show();
            invocation.return_value(null);
            break;
        case 'State':
            invocation.return_value(new GLib.Variant('(bb)', [this._mode !== SHOWN, this._tracker.get_pointer_visible()]));
            break;
        default:
            throw new Error(`unknown method ${method}`);
        }
    }

    _inhibit() {
        if (this._mode === SHOWN)
            this._tracker.inhibit_cursor_visibility();
    }

    _hide(sender) {
        this._holder = sender;
        this._stopWatching();
        this._inhibit();
        this._mode = HELD;
        // Not asked again in time: it comes back.
        if (this._timer)
            GLib.source_remove(this._timer);
        this._timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, HOLD_SECONDS, () => {
            this._timer = 0;
            this._show();
            return GLib.SOURCE_REMOVE;
        });
    }

    _hideUntilMoved(sender) {
        // While one-mk holds the mouse away that already hides it.
        if (this._mode === HELD)
            return;
        this._holder = sender;
        this._inhibit();
        this._mode = UNTIL_MOVED;
        if (this._motion)
            return;
        // The pointer moved: the tracker says so for any device, injected or not.
        const since = GLib.get_monotonic_time();
        this._motion = this._tracker.connect('position-invalidated', () => {
            if (GLib.get_monotonic_time() - since > GRACE_MS * 1000)
                this._show();
        });
    }

    _stopWatching() {
        if (this._motion) {
            this._tracker.disconnect(this._motion);
            this._motion = 0;
        }
    }

    _show() {
        if (this._timer) {
            GLib.source_remove(this._timer);
            this._timer = 0;
        }
        this._stopWatching();
        this._holder = null;
        if (this._mode !== SHOWN && this._tracker)
            this._tracker.uninhibit_cursor_visibility();
        this._mode = SHOWN;
    }
}
