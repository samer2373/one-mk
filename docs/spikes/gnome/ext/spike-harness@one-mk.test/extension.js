// Test-only: equivalent of `--unsafe-mode` for the isolated headless Shell,
// which has no such command-line flag in GNOME 50.
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

export default class SpikeHarness extends Extension {
    enable() {
        global.context.unsafe_mode = true;
    }

    disable() {
        global.context.unsafe_mode = false;
    }
}
