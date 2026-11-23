// The wallpaper extension's enable(), run against stand-ins for GNOME Shell (no desktop is touched).
// The promise: an enable that fails leaves nothing attached, and disable() is safe at any stage.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../../extensions/gnome/one-mk-wallpaper@one-mk.org/extension.js', import.meta.url), 'utf8');

function fixture(failure) {
    const added = [];
    const seen = {destroyed: 0};
    const context = {
        Extension: class {}, Config: {PACKAGE_VERSION: '50.1'}, TextDecoder,
        GLib: {}, Meta: {},
        Clutter: {Actor: class { destroy() { seen.destroyed++; } }},
        Main: {layoutManager: {_backgroundGroup: {add_child: actor => added.push(actor)}}},
        global: {display: {disconnect() { throw new Error('nothing was connected'); }}},
        Gio: {
            DBus: {session: {signal_unsubscribe() { throw new Error('nothing was subscribed'); }}},
            bus_unown_name() { throw new Error('no name was owned'); },
            DBusInterfaceInfo: {new_for_xml: () => { if (failure === 'xml') throw new Error('bad bridge.xml'); return {}; }},
            DBusExportedObject: class {
                connect() {}
                export() { throw new Error('no bus'); }
                unexport() { throw new Error('it was never exported'); }
            },
        },
    };
    vm.createContext(context);
    vm.runInContext(source.replace(/^import .*;$/gm, '').replace('export default class ', 'class ') + '\nglobalThis.Video = WallpaperVideo;', context);
    const bridge = new context.Video();
    bridge.dir = {get_child: () => ({load_contents: () => [true, new TextEncoder().encode('<node/>')]})};
    return {bridge, added, seen};
}

test('a bridge description that cannot be read adds nothing to the background', () => {
    const {bridge, added} = fixture('xml');
    assert.throws(() => bridge.enable(), /bad bridge.xml/);
    assert.equal(added.length, 0);
    bridge.disable();
});

test('an export that fails takes the background layer out again, once, however often it is enabled', () => {
    const {bridge, added, seen} = fixture('export');
    for (let attempt = 1; attempt <= 3; attempt++) {
        assert.throws(() => bridge.enable(), /no bus/);
        assert.deepEqual([added.length, seen.destroyed], [attempt, attempt]);
    }
    bridge.disable();
    assert.equal(seen.destroyed, 3);
});
