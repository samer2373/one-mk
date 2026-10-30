// Test-only: equivalent of `--unsafe-mode` for the isolated headless Shell,
// which has no such command-line flag in GNOME 50.
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import GLib from 'gi://GLib';
import System from 'system';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

export default class SpikeHarness extends Extension {
    collectGarbage() {
        System.gc();
        return true;
    }

    timeUs() {
        return GLib.get_monotonic_time();
    }

    enable() {
        global.context.unsafe_mode = true;
        this.events = [];
        this._signals = [];
        const record = kind => {
            this.events.push({kind, us: GLib.get_monotonic_time(), locked: Main.screenShield.locked});
            if (this.events.length > 100)
                this.events.shift();
        };
        this._signals.push([Main.layoutManager, Main.layoutManager.connect('monitors-changed', () => record('layout'))]);
        this._signals.push([Main.screenShield, Main.screenShield.connect('locked-changed', () => record('lock'))]);
    }

    disable() {
        global.context.unsafe_mode = false;
        for (const [object, signal] of this._signals)
            object.disconnect(signal);
        this._signals = [];
    }
}
